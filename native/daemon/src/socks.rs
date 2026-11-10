//! The client of the upstream proxy: SOCKS5 (`CONNECT` and `UDP ASSOCIATE`) and HTTP `CONNECT`.
//!
//! The daemon makes each connection to the upstream from its own process. Thus the proxy (if it matches the process of the connection) sees all forwarded traffic as traffic of `seltund.exe`. A target is always an IP address, because the TUN sees only what the process already resolved.

use std::io::{Error, ErrorKind, Result};
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpStream, UdpSocket};

use crate::config::{Protocol, Upstream};

fn invalid(message: impl Into<String>) -> Error {
    Error::new(ErrorKind::InvalidData, message.into())
}

/// `ATYP | addr | port`. This is the address form that SOCKS5 uses in requests, replies, and UDP headers.
fn encode_addr(out: &mut Vec<u8>, addr: SocketAddr) {
    match addr.ip() {
        IpAddr::V4(ip) => {
            out.push(1);
            out.extend_from_slice(&ip.octets());
        },
        IpAddr::V6(ip) => {
            out.push(4);
            out.extend_from_slice(&ip.octets());
        },
    }
    out.extend_from_slice(&addr.port().to_be_bytes());
}

/// Parse `ATYP | addr | port` from the start of `data`. Return the address and the number of bytes that the function used. The function rejects a domain. The daemon asks only for IP addresses, and a relay answers with an IP address.
fn decode_addr(data: &[u8]) -> Result<(SocketAddr, usize)> {
    let short = || invalid("truncated SOCKS5 address");
    let (ip, used): (IpAddr, usize) = match *data.first().ok_or_else(short)? {
        1 => {
            let octets: [u8; 4] = data.get(1..5).ok_or_else(short)?.try_into().unwrap();
            (Ipv4Addr::from(octets).into(), 5)
        },
        4 => {
            let octets: [u8; 16] = data.get(1..17).ok_or_else(short)?.try_into().unwrap();
            (Ipv6Addr::from(octets).into(), 17)
        },
        other => {
            return Err(invalid(format!(
                "The SOCKS5 address type {other} is not supported."
            )));
        },
    };
    let port = data.get(used..used + 2).ok_or_else(short)?;
    Ok((
        SocketAddr::new(ip, u16::from_be_bytes([port[0], port[1]])),
        used + 2,
    ))
}

/// Read a SOCKS5 reply (`VER | REP | RSV | ATYP | addr | port`) and return the bound address.
async fn read_reply(stream: &mut TcpStream) -> Result<SocketAddr> {
    let mut head = [0u8; 4];
    stream.read_exact(&mut head).await?;
    if head[0] != 5 {
        return Err(invalid("The reply is not a SOCKS5 reply."));
    }
    if head[1] != 0 {
        return Err(Error::new(
            ErrorKind::ConnectionRefused,
            format!(
                "The upstream refused the request (SOCKS5 reply {}).",
                head[1]
            ),
        ));
    }
    let rest = match head[3] {
        1 => 4 + 2,
        4 => 16 + 2,
        other => return Err(invalid(format!("unsupported SOCKS5 address type {other}"))),
    };
    let mut buf = vec![head[3]];
    buf.resize(1 + rest, 0);
    stream.read_exact(&mut buf[1..]).await?;
    Ok(decode_addr(&buf)?.0)
}

async fn socks5_handshake(stream: &mut TcpStream) -> Result<()> {
    stream.write_all(&[5, 1, 0]).await?;
    let mut answer = [0u8; 2];
    stream.read_exact(&mut answer).await?;
    if answer != [5, 0] {
        return Err(invalid(
            "The upstream does not accept SOCKS5 without authentication.",
        ));
    }
    Ok(())
}

async fn socks5_command(
    stream: &mut TcpStream,
    command: u8,
    target: SocketAddr,
) -> Result<SocketAddr> {
    socks5_handshake(stream).await?;
    let mut request = vec![5, command, 0];
    encode_addr(&mut request, target);
    stream.write_all(&request).await?;
    read_reply(stream).await
}

async fn http_connect(stream: &mut TcpStream, target: SocketAddr) -> Result<()> {
    let request = format!("CONNECT {target} HTTP/1.1\r\nHost: {target}\r\n\r\n");
    stream.write_all(request.as_bytes()).await?;
    // Read one byte at a time up to the blank line. Thus the code does not read the tunneled stream.
    let mut head = Vec::new();
    while !head.ends_with(b"\r\n\r\n") {
        if head.len() > 8192 {
            return Err(invalid("The CONNECT answer of the upstream is too long."));
        }
        head.push(stream.read_u8().await?);
    }
    let line = head.split(|&b| b == b'\r').next().unwrap_or_default();
    let line = String::from_utf8_lossy(line);
    if line.split_whitespace().nth(1) != Some("200") {
        return Err(Error::new(
            ErrorKind::ConnectionRefused,
            format!("The upstream refused the request: {line}"),
        ));
    }
    Ok(())
}

/// A TCP stream to `target` through the upstream.
pub async fn connect(upstream: &Upstream, target: SocketAddr) -> Result<TcpStream> {
    let mut stream = TcpStream::connect(upstream.address).await?;
    stream.set_nodelay(true)?;
    match upstream.protocol {
        Protocol::Socks5 => {
            socks5_command(&mut stream, 1, target).await?;
        },
        Protocol::Http => http_connect(&mut stream, target).await?,
    }
    Ok(stream)
}

/// A SOCKS5 UDP association. Datagrams to each target go through the UDP relay of the upstream until the code drops this value. The drop closes the control connection that keeps the association open.
pub struct UdpAssociation {
    _control: TcpStream,
    socket: UdpSocket,
}

impl UdpAssociation {
    pub async fn open(upstream: &Upstream) -> Result<Self> {
        let mut control = TcpStream::connect(upstream.address).await?;
        let unspecified = SocketAddr::new(Ipv4Addr::UNSPECIFIED.into(), 0);
        let mut relay = socks5_command(&mut control, 3, unspecified).await?;
        if relay.ip().is_unspecified() {
            relay.set_ip(upstream.address.ip());
        }
        let socket = UdpSocket::bind((Ipv4Addr::LOCALHOST, 0)).await?;
        socket.connect(relay).await?;
        Ok(Self {
            _control: control,
            socket,
        })
    }

    pub async fn send_to(&self, target: SocketAddr, payload: &[u8]) -> Result<()> {
        let mut datagram = vec![0, 0, 0];
        encode_addr(&mut datagram, target);
        datagram.extend_from_slice(payload);
        self.socket.send(&datagram).await?;
        Ok(())
    }

    /// The next datagram, with its source and its payload. The function copies the payload to the start of `buf`.
    pub async fn recv(&self, buf: &mut [u8]) -> Result<(SocketAddr, usize)> {
        let mut raw = vec![0u8; buf.len() + 22];
        let n = self.socket.recv(&mut raw).await?;
        let raw = &raw[..n];
        if raw.len() < 3 || raw[2] != 0 {
            return Err(invalid(
                "The UDP datagram from the upstream is fragmented or too short.",
            ));
        }
        let (source, used) = decode_addr(&raw[3..])?;
        let payload = &raw[3 + used..];
        let len = payload.len().min(buf.len());
        buf[..len].copy_from_slice(&payload[..len]);
        Ok((source, len))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn address_round_trip() {
        for text in ["1.2.3.4:80", "[2001:db8::1]:443"] {
            let addr: SocketAddr = text.parse().unwrap();
            let mut buf = Vec::new();
            encode_addr(&mut buf, addr);
            assert_eq!(decode_addr(&buf).unwrap(), (addr, buf.len()));
        }
    }

    #[test]
    fn truncated_address_is_an_error() {
        assert!(decode_addr(&[1, 1, 2]).is_err());
        assert!(decode_addr(&[3, 1, 2, 3]).is_err());
    }
}
