//! The data path. A userspace TCP/IP stack (`ipstack`) ends the flows that arrive in the TUN adapter. Then the daemon forwards the flows.
//!
//! The daemon finds the process of each flow in the socket tables. If the process is selected, the daemon sends the flow to the upstream proxy. If not, the flow leaves through the physical interface without a change. A socket that the daemon pins to that interface carries it. Thus the daemon proxies only the selected processes.

use std::io::{Error, ErrorKind, Result as IoResult};
use std::net::{Ipv4Addr, SocketAddr};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use anyhow::Result;
use ipstack::{IpStack, IpStackConfig, IpStackStream};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::net::{TcpSocket, TcpStream, UdpSocket};
use tokio::sync::watch;

use crate::config::Upstream;
use crate::matcher::Matcher;
use crate::platform::{self, Owners, Proto};
use crate::socks::{self, UdpAssociation};

/// The time that a flow can be idle before the daemon closes it.
const UDP_IDLE: Duration = Duration::from_secs(60);
/// The idle time of a DNS flow (UDP port 53). A DNS query gets its answer in a few seconds, and each flow holds a socket. A long idle time with many queries uses many file descriptors.
const DNS_IDLE: Duration = Duration::from_secs(10);

/// The time that a UDP flow to `dst` can be idle before the daemon closes it.
fn udp_idle(dst: SocketAddr) -> Duration {
    if dst.port() == 53 { DNS_IDLE } else { UDP_IDLE }
}
/// The time that the daemon keeps the physical interface before it looks for it again.
const IFACE_TTL: Duration = Duration::from_secs(2);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verdict {
    /// Through the upstream proxy.
    Proxy,
    /// Out through the physical interface, without a change.
    Direct,
}

/// The data that the daemon uses to decide on a flow and to forward it.
pub struct Context {
    pub upstream: Upstream,
    pub matcher: Matcher,
    /// The interface index of the TUN adapter. The search for the physical interface excludes it.
    pub tun_index: u32,
    owners: Owners,
    interface: Mutex<Option<(Instant, Option<u32>)>>,
}

impl Context {
    pub fn new(upstream: Upstream, matcher: Matcher, tun_index: u32) -> Self {
        Self {
            upstream,
            matcher,
            tun_index,
            owners: Owners::default(),
            interface: Mutex::new(None),
        }
    }

    /// Proxy a flow only if the process that owns its source socket is selected. If the daemon does not find the owner, the flow goes direct.
    pub fn verdict(&self, proto: Proto, src: SocketAddr, dst: SocketAddr) -> Verdict {
        let remote = (proto == Proto::Tcp).then_some(dst);
        match self.owners.selected(proto, src, remote, &self.matcher) {
            Some(image) => {
                tracing::debug!(%image, %src, %dst, ?proto, "proxy");
                Verdict::Proxy
            },
            None => Verdict::Direct,
        }
    }

    fn real_interface(&self) -> Option<u32> {
        let mut cached = self.interface.lock().unwrap_or_else(|e| e.into_inner());
        if let Some((at, index)) = *cached
            && at.elapsed() < IFACE_TTL
        {
            return index;
        }
        let index = platform::physical_interface(self.tun_index);
        *cached = Some((Instant::now(), index));
        index
    }

    fn pin<S: SocketPin>(&self, socket: &S) -> IoResult<()> {
        let index = self.real_interface().ok_or_else(|| {
            Error::new(
                ErrorKind::NotConnected,
                "There is no network interface other than the TUN.",
            )
        })?;
        socket.pin(index)
    }
}

/// A socket that the daemon can pin to an interface.
trait SocketPin {
    fn pin(&self, index: u32) -> IoResult<()>;
}

#[cfg(windows)]
impl<T: std::os::windows::io::AsRawSocket> SocketPin for T {
    fn pin(&self, index: u32) -> IoResult<()> {
        platform::bind_to_interface(self, index)
    }
}

#[cfg(not(windows))]
impl SocketPin for TcpSocket {
    fn pin(&self, index: u32) -> IoResult<()> {
        platform::bind_to_interface(self, index)
    }
}

#[cfg(not(windows))]
impl SocketPin for UdpSocket {
    fn pin(&self, index: u32) -> IoResult<()> {
        platform::bind_to_interface(self, index)
    }
}

async fn connect_direct(ctx: &Context, dst: SocketAddr) -> IoResult<TcpStream> {
    let socket = match dst {
        SocketAddr::V4(_) => TcpSocket::new_v4()?,
        SocketAddr::V6(_) => {
            return Err(Error::new(
                ErrorKind::Unsupported,
                "IPv6 does not go into the TUN.",
            ));
        },
    };
    ctx.pin(&socket)?;
    socket.connect(dst).await
}

async fn handle_tcp<S>(mut flow: S, src: SocketAddr, dst: SocketAddr, ctx: Arc<Context>)
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    let probe = ctx.clone();
    let verdict = tokio::task::spawn_blocking(move || probe.verdict(Proto::Tcp, src, dst))
        .await
        .unwrap_or(Verdict::Direct);
    // Windows tests each adapter for Internet access. If the test passes on the TUN adapter, Windows can make that adapter the main connection and show "LAN" instead of Wi-Fi. The daemon ends the test flow, so the test fails on this adapter only. The same test on the physical interface does not pass through the TUN adapter.
    let mut first = Vec::new();
    if verdict == Verdict::Direct && dst.port() == 80 {
        first = read_first_bytes(&mut flow).await;
        if is_connectivity_probe(&first) {
            tracing::debug!(%dst, "connectivity probe ended");
            let _ = flow.shutdown().await;
            return;
        }
    }
    let upstream = match verdict {
        Verdict::Proxy => socks::connect(&ctx.upstream, dst).await,
        Verdict::Direct => connect_direct(&ctx, dst).await,
    };
    let mut upstream = match upstream {
        Ok(stream) => stream,
        Err(error) => {
            tracing::debug!(%dst, ?verdict, "Connection failed: {error}");
            let _ = flow.shutdown().await;
            return;
        },
    };
    if !first.is_empty() && upstream.write_all(&first).await.is_err() {
        let _ = flow.shutdown().await;
        return;
    }
    let _ = tokio::io::copy_bidirectional(&mut flow, &mut upstream).await;
    let _ = upstream.shutdown().await;
    let _ = flow.shutdown().await;
}

/// The time that the daemon waits for the first bytes of a plain HTTP flow.
const FIRST_BYTES_WAIT: Duration = Duration::from_millis(1500);

/// Read the first bytes that the client sends. If the client sends nothing in time, return no bytes.
async fn read_first_bytes<S: AsyncRead + Unpin>(flow: &mut S) -> Vec<u8> {
    let mut buffer = vec![0u8; 1024];
    match tokio::time::timeout(FIRST_BYTES_WAIT, flow.read(&mut buffer)).await {
        Ok(Ok(count)) => {
            buffer.truncate(count);
            buffer
        },
        _ => Vec::new(),
    }
}

/// True if the bytes are the HTTP request of the Windows test for Internet access.
fn is_connectivity_probe(request: &[u8]) -> bool {
    let text = String::from_utf8_lossy(request).to_ascii_lowercase();
    text.starts_with("get ")
        && (text.contains("host: www.msftconnecttest.com") || text.contains("microsoft ncsi"))
}

async fn handle_udp<S>(mut flow: S, src: SocketAddr, dst: SocketAddr, ctx: Arc<Context>)
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    // Only unicast IPv4 goes into the TUN. The IPv6 messages of the adapter itself are not forwarded.
    // Multicast and broadcast (SSDP, LLMNR, mDNS) have no destination for a relay.
    let local_only = match dst.ip() {
        std::net::IpAddr::V4(ip) => ip.is_multicast() || ip.is_broadcast(),
        std::net::IpAddr::V6(_) => true,
    };
    if local_only {
        return;
    }
    let probe = ctx.clone();
    let verdict = tokio::task::spawn_blocking(move || probe.verdict(Proto::Udp, src, dst))
        .await
        .unwrap_or(Verdict::Direct);
    // If UDP does not go through the proxy, the DNS of a selected process goes direct, and the daemon drops its other UDP (QUIC, for example). The application then changes to TCP.
    let through_proxy = verdict == Verdict::Proxy && ctx.upstream.udp;
    if verdict == Verdict::Proxy && !through_proxy && dst.port() != 53 {
        return;
    }
    let result = if through_proxy {
        relay_udp_via_proxy(&mut flow, dst, &ctx, udp_idle(dst)).await
    } else {
        relay_udp_direct(&mut flow, dst, &ctx, udp_idle(dst)).await
    };
    if let Err(error) = result {
        tracing::debug!(%dst, "UDP flow ended: {error}");
    }
    let _ = flow.shutdown().await;
}

async fn relay_udp_direct<S>(
    flow: &mut S,
    dst: SocketAddr,
    ctx: &Context,
    idle: Duration,
) -> IoResult<()>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    let socket = UdpSocket::bind((Ipv4Addr::UNSPECIFIED, 0)).await?;
    ctx.pin(&socket)?;
    socket.connect(dst).await?;
    let (mut up, mut down) = (vec![0u8; 65535], vec![0u8; 65535]);
    loop {
        tokio::select! {
            n = flow.read(&mut up) => {
                let n = n?;
                if n == 0 { return Ok(()); }
                socket.send(&up[..n]).await?;
            }
            n = socket.recv(&mut down) => {
                let n = n?;
                flow.write_all(&down[..n]).await?;
            }
            _ = tokio::time::sleep(idle) => return Ok(()),
        }
    }
}

async fn relay_udp_via_proxy<S>(
    flow: &mut S,
    dst: SocketAddr,
    ctx: &Context,
    idle: Duration,
) -> IoResult<()>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    let association = UdpAssociation::open(&ctx.upstream).await?;
    let (mut up, mut down) = (vec![0u8; 65535], vec![0u8; 65535]);
    loop {
        tokio::select! {
            n = flow.read(&mut up) => {
                let n = n?;
                if n == 0 { return Ok(()); }
                association.send_to(dst, &up[..n]).await?;
            }
            got = association.recv(&mut down) => {
                let (_source, n) = got?;
                flow.write_all(&down[..n]).await?;
            }
            _ = tokio::time::sleep(idle) => return Ok(()),
        }
    }
}

/// Answer a ping in the daemon. A userspace program cannot forward an ICMP echo.
fn echo_reply(payload: &[u8]) -> Option<Vec<u8>> {
    use etherparse::{Icmpv4Header, Icmpv4Type};
    let (header, body) = Icmpv4Header::from_slice(payload).ok()?;
    let Icmpv4Type::EchoRequest(echo) = header.icmp_type else {
        return None;
    };
    let mut reply = Icmpv4Header::new(Icmpv4Type::EchoReply(echo));
    reply.update_checksum(body);
    let mut out = reply.to_bytes().to_vec();
    out.extend_from_slice(body);
    Some(out)
}

/// Serve the flows that arrive on `device`. The function stops when `stop` is set or when the device fails.
pub async fn run<D>(
    device: D,
    ctx: Arc<Context>,
    mtu: u16,
    mut stop: watch::Receiver<bool>,
) -> Result<()>
where
    D: AsyncRead + AsyncWrite + Unpin + Send + 'static,
{
    let mut config = IpStackConfig::default();
    config.mtu(mtu)?;
    config.udp_timeout(UDP_IDLE);
    let mut stack = IpStack::new(config, device);
    loop {
        let stream = tokio::select! {
            _ = stop.changed() => return Ok(()),
            accepted = stack.accept() => accepted?,
        };
        match stream {
            IpStackStream::Tcp(tcp) => {
                let (src, dst) = (tcp.local_addr(), tcp.peer_addr());
                tokio::spawn(handle_tcp(tcp, src, dst, ctx.clone()));
            },
            IpStackStream::Udp(udp) => {
                let (src, dst) = (udp.local_addr(), udp.peer_addr());
                tokio::spawn(handle_udp(udp, src, dst, ctx.clone()));
            },
            IpStackStream::UnknownTransport(packet) => {
                if let Some(reply) = echo_reply(packet.payload()) {
                    let _ = packet.send(reply);
                }
            },
            IpStackStream::UnknownNetwork(_) => {},
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Protocol;

    fn ctx(names: &[&str]) -> Context {
        let upstream = Upstream {
            address: "127.0.0.1:1".parse().unwrap(),
            protocol: Protocol::Socks5,
            udp: false,
        };
        Context::new(upstream, Matcher::new(names).unwrap(), 0)
    }

    #[test]
    fn unknown_owner_goes_direct() {
        let c = ctx(&["anything.exe"]);
        let v = c.verdict(
            Proto::Tcp,
            "198.19.0.1:1".parse().unwrap(),
            "1.1.1.1:80".parse().unwrap(),
        );
        assert_eq!(v, Verdict::Direct);
    }

    #[test]
    fn dns_flows_have_a_short_idle_time() {
        assert_eq!(udp_idle("8.8.8.8:53".parse().unwrap()), DNS_IDLE);
        assert_eq!(udp_idle("8.8.8.8:443".parse().unwrap()), UDP_IDLE);
    }

    #[test]
    fn windows_connectivity_test_is_found() {
        let probe = b"GET /connecttest.txt HTTP/1.1\r\nConnection: Close\r\nHost: www.msftconnecttest.com\r\nUser-Agent: Microsoft NCSI\r\n\r\n";
        assert!(is_connectivity_probe(probe));
        assert!(!is_connectivity_probe(
            b"GET / HTTP/1.1\r\nHost: example.com\r\n\r\n"
        ));
        assert!(!is_connectivity_probe(b""));
    }

    #[test]
    fn echo_request_is_answered() {
        use etherparse::{IcmpEchoHeader, Icmpv4Header, Icmpv4Type};
        let mut request =
            Icmpv4Header::new(Icmpv4Type::EchoRequest(IcmpEchoHeader { id: 7, seq: 1 }));
        request.update_checksum(b"hi");
        let mut packet = request.to_bytes().to_vec();
        packet.extend_from_slice(b"hi");
        let reply = echo_reply(&packet).unwrap();
        let (header, body) = Icmpv4Header::from_slice(&reply).unwrap();
        assert_eq!(
            header.icmp_type,
            Icmpv4Type::EchoReply(IcmpEchoHeader { id: 7, seq: 1 })
        );
        assert_eq!(body, b"hi");
    }
}
