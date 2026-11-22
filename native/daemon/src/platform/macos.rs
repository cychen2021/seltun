//! macOS code. libproc gives the owners of sockets. The system configuration gives the primary interface. `route` sets the routes.
//!
//! The `unsafe` blocks read the socket union of libproc and call `setsockopt`, `geteuid`, `if_nametoindex` and `if_indextoname` with buffers that this module owns.

#![allow(unsafe_code)]

use std::ffi::{CStr, CString};
use std::io::{Error, Result, Write};
use std::mem::size_of;
use std::net::{Ipv4Addr, SocketAddr};
use std::os::fd::AsRawFd;
use std::process::{Command, Stdio};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use libproc::file_info::{ListFDs, ProcFDType, pidfdinfo};
use libproc::net_info::{SocketFDInfo, SocketInfoKind};
use libproc::proc_pid::{listpidinfo, pidpath};
use libproc::processes::{ProcFilter, pids_by_type};

use super::Proto;
use crate::matcher::Matcher;

/// The instruction that the daemon shows when it does not run as root.
pub const ELEVATION_HINT: &str = "To create the TUN adapter, the daemon must run as root. Run `seltun autostart enable` one time. Then run `seltun daemon start`.";

/// True if this process runs as root. The daemon needs root to create the utun interface and its routes.
pub fn is_elevated() -> bool {
    // SAFETY: this call has no arguments and no state.
    unsafe { libc::geteuid() == 0 }
}

/// Do nothing. Only Windows has a console that the daemon must attach to.
pub fn attach_parent_console() {}

/// A socket: `(protocol, local address, local port, remote address, remote port)`.
type Row = (Proto, Ipv4Addr, u16, Ipv4Addr, u16);

fn port(raw: i32) -> u16 {
    u16::from_be(raw as u16)
}

fn ip(raw: u32) -> Ipv4Addr {
    Ipv4Addr::from(u32::from_be(raw))
}

/// The IPv4 TCP and UDP sockets of `pid`. If the process cannot be read, the list is empty.
fn sockets(pid: u32) -> Vec<Row> {
    let pid = pid as i32;
    let Ok(fds) = listpidinfo::<ListFDs>(pid, 4096) else {
        return Vec::new();
    };
    let mut rows = Vec::new();
    for fd in fds {
        if !matches!(ProcFDType::from(fd.proc_fdtype), ProcFDType::Socket) {
            continue;
        }
        let Ok(info) = pidfdinfo::<SocketFDInfo>(pid, fd.proc_fd) else {
            continue;
        };
        let proto = match (info.psi.soi_family, info.psi.soi_protocol) {
            (libc::AF_INET, libc::IPPROTO_TCP) => Proto::Tcp,
            (libc::AF_INET, libc::IPPROTO_UDP) => Proto::Udp,
            _ => continue,
        };
        // SAFETY: `soi_kind` tells which member of the union the kernel filled. A TCP socket has `pri_tcp`, and its first member is the same `InSockInfo` as `pri_in`.
        let inet = unsafe {
            match SocketInfoKind::from(info.psi.soi_kind) {
                SocketInfoKind::Tcp => info.psi.soi_proto.pri_tcp.tcpsi_ini,
                SocketInfoKind::In => info.psi.soi_proto.pri_in,
                _ => continue,
            }
        };
        // SAFETY: the socket is in the family `AF_INET`. Thus the addresses are the IPv4 members of the unions.
        let (local, remote) = unsafe {
            (
                inet.insi_laddr.ina_46.i46a_addr4.s_addr,
                inet.insi_faddr.ina_46.i46a_addr4.s_addr,
            )
        };
        rows.push((
            proto,
            ip(local),
            port(inet.insi_lport),
            ip(remote),
            port(inet.insi_fport),
        ));
    }
    rows
}

struct Selected {
    at: Instant,
    /// The selected processes that run: `(pid, path of the executable)`.
    pids: Vec<(u32, String)>,
}

/// The owner of a socket. macOS has no table of all sockets with their owners. Thus the lookup reads the sockets of the selected processes only. It keeps the list of the selected processes and finds them again if a lookup finds nothing or if the list is old, because a selected process can start at any time. The list is for one matcher. Thus use one `Owners` for each matcher.
#[derive(Default)]
pub struct Owners {
    selected: Mutex<Option<Selected>>,
}

const FRESH: Duration = Duration::from_millis(1000);
const MISS_REFRESH: Duration = Duration::from_millis(20);

fn running_selected(matcher: &Matcher) -> Vec<(u32, String)> {
    let Ok(pids) = pids_by_type(ProcFilter::All) else {
        return Vec::new();
    };
    pids.into_iter()
        .filter(|&pid| pid != 0)
        .filter_map(|pid| pidpath(pid as i32).ok().map(|path| (pid, path)))
        .filter(|(_, path)| matcher.matches(path))
        .collect()
}

fn find(rows: &[Row], proto: Proto, local: SocketAddr, remote: Option<SocketAddr>) -> bool {
    let SocketAddr::V4(local) = local else {
        return false;
    };
    rows.iter().any(|(p, addr, lport, raddr, rport)| {
        *p == proto
            && *lport == local.port()
            && (*addr == *local.ip() || addr.is_unspecified())
            && match (proto, remote) {
                (Proto::Tcp, Some(SocketAddr::V4(r))) => *raddr == *r.ip() && *rport == r.port(),
                _ => true,
            }
    })
}

impl Owners {
    /// The executable of the process that owns the socket `local`, if `matcher` selects that process. For TCP, the lookup also uses `remote`.
    pub fn selected(
        &self,
        proto: Proto,
        local: SocketAddr,
        remote: Option<SocketAddr>,
        matcher: &Matcher,
    ) -> Option<String> {
        if matcher.is_empty() {
            return None;
        }
        let mut guard = self.selected.lock().unwrap_or_else(|e| e.into_inner());
        for attempt in 0..2 {
            if let Some(list) = guard.as_ref() {
                let age = list.at.elapsed();
                if age <= FRESH || attempt == 1 {
                    for (pid, path) in &list.pids {
                        if find(&sockets(*pid), proto, local, remote) {
                            return Some(path.clone());
                        }
                    }
                    if age <= MISS_REFRESH || attempt == 1 {
                        return None;
                    }
                }
            }
            *guard = Some(Selected {
                at: Instant::now(),
                pids: running_selected(matcher),
            });
        }
        None
    }
}

fn if_index(name: &str) -> Option<u32> {
    let name = CString::new(name).ok()?;
    // SAFETY: `name` is a valid C string during the call.
    let index = unsafe { libc::if_nametoindex(name.as_ptr()) };
    (index != 0).then_some(index)
}

fn if_name(index: u32) -> Result<String> {
    let mut buf = [0 as libc::c_char; libc::IF_NAMESIZE];
    // SAFETY: `buf` has `IF_NAMESIZE` bytes, the size that the call needs.
    let name = unsafe { libc::if_indextoname(index, buf.as_mut_ptr()) };
    if name.is_null() {
        return Err(Error::last_os_error());
    }
    // SAFETY: the call succeeded. Thus `buf` has a C string.
    Ok(unsafe { CStr::from_ptr(buf.as_ptr()) }
        .to_string_lossy()
        .into_owned())
}

/// The name of the primary interface in the text that `scutil` shows for `State:/Network/Global/IPv4`.
fn primary_from_scutil(text: &str) -> Option<&str> {
    text.lines().find_map(|line| {
        let (key, value) = line.split_once(':')?;
        (key.trim() == "PrimaryInterface").then(|| value.trim())
    })
}

/// The index of the physical interface: the primary IPv4 interface of the system configuration, if it is not `exclude` (the TUN adapter). Traffic that must not enter the TUN leaves through this interface. The TUN routes do not change the primary interface, because the utun interface is not a network service. Each call finds the interface again, because Wi-Fi and Ethernet connect and disconnect.
pub fn physical_interface(exclude: u32) -> Option<u32> {
    let mut child = Command::new("/usr/sbin/scutil")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    child
        .stdin
        .take()?
        .write_all(b"show State:/Network/Global/IPv4\n")
        .ok()?;
    let output = child.wait_with_output().ok()?;
    let text = String::from_utf8_lossy(&output.stdout);
    let index = if_index(primary_from_scutil(&text)?)?;
    (index != exclude).then_some(index)
}

/// Pin `socket` to the interface with this index (`IP_BOUND_IF`). The traffic of the socket leaves through this interface, although the TUN routes cover all destinations.
pub fn bind_to_interface<S: AsRawFd>(socket: &S, index: u32) -> Result<()> {
    // SAFETY: `index` is valid during the call, and the call receives its size.
    let code = unsafe {
        libc::setsockopt(
            socket.as_raw_fd(),
            libc::IPPROTO_IP,
            libc::IP_BOUND_IF,
            (&raw const index).cast(),
            size_of::<u32>() as libc::socklen_t,
        )
    };
    if code == 0 {
        Ok(())
    } else {
        Err(Error::last_os_error())
    }
}

fn route(args: &[&str]) -> Result<()> {
    let output = Command::new("/sbin/route").arg("-n").args(args).output()?;
    if output.status.success() {
        Ok(())
    } else {
        let text = String::from_utf8_lossy(&output.stderr);
        Err(Error::other(format!(
            "route {}: {}",
            args.join(" "),
            text.trim()
        )))
    }
}

/// The two half-default routes (`0.0.0.0/1` and `128.0.0.0/1`). They send each destination that has no more specific route into the TUN adapter. LAN, loopback, and the subnet of the TUN have more specific routes. The default route stays, and the scoped default route of the physical interface carries the sockets that the daemon pins to it. The routes go away with the interface. The code also removes them when the daemon stops.
pub struct Routes {
    interface: String,
}

const HALVES: [&str; 2] = ["0.0.0.0/1", "128.0.0.0/1"];

impl Routes {
    pub fn install(tun_index: u32) -> Result<Self> {
        let routes = Self {
            interface: if_name(tun_index)?,
        };
        for half in HALVES {
            // An old route from a daemon that stopped without cleanup blocks the new one.
            let _ = route(&["delete", "-net", half]);
            route(&["add", "-net", half, "-interface", &routes.interface])?;
        }
        Ok(routes)
    }
}

impl Drop for Routes {
    fn drop(&mut self) {
        for half in HALVES {
            let _ = route(&["delete", "-net", half, "-interface", &self.interface]);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::TcpListener;

    #[test]
    fn finds_this_process_as_the_owner_of_its_connection() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let local = listener.local_addr().unwrap();
        let client = std::net::TcpStream::connect(local).unwrap();
        let (_server, _) = listener.accept().unwrap();
        let me = std::env::current_exe().unwrap();
        let name = me.file_name().unwrap().to_string_lossy().into_owned();
        let owners = Owners::default();
        let image = owners
            .selected(
                Proto::Tcp,
                client.local_addr().unwrap(),
                Some(local),
                &Matcher::new(&[name]),
            )
            .unwrap();
        assert_eq!(
            std::fs::canonicalize(image).unwrap(),
            std::fs::canonicalize(me).unwrap()
        );
        // A process that the matcher does not select is not the owner.
        assert!(
            Owners::default()
                .selected(
                    Proto::Tcp,
                    client.local_addr().unwrap(),
                    Some(local),
                    &Matcher::new(&["not-this-process"]),
                )
                .is_none()
        );
    }

    #[test]
    fn reads_the_primary_interface() {
        let text = "<dictionary> {\n  PrimaryInterface : en0\n  PrimaryService : 1234\n  Router : 192.168.1.1\n}\n";
        assert_eq!(primary_from_scutil(text), Some("en0"));
        assert_eq!(primary_from_scutil("  No such key\n"), None);
    }

    #[test]
    fn finds_a_primary_interface() {
        // A computer that is offline has no primary interface.
        if let Some(index) = physical_interface(u32::MAX) {
            assert!(if_name(index).is_ok());
        }
    }

    #[test]
    fn port_and_ip_decode_network_order() {
        assert_eq!(port(0x5000), 80);
        assert_eq!(ip(u32::from_ne_bytes([127, 0, 0, 1])), Ipv4Addr::LOCALHOST);
    }
}
