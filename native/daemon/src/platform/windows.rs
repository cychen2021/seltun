//! Windows code. IP Helper gives the owners of sockets and the physical default interface. `netsh` sets the routes.
//!
//! This is the only module with `unsafe`. Each block is a plain call into IP Helper or Winsock with a buffer that this module owns.

#![allow(unsafe_code)]

use std::io::{Error, Result};
use std::mem::size_of;
use std::net::{Ipv4Addr, SocketAddr};
use std::os::windows::io::AsRawSocket;
use std::os::windows::process::CommandExt;
use std::process::Command;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use windows_sys::Win32::Foundation::CloseHandle;
use windows_sys::Win32::NetworkManagement::IpHelper::{
    FreeMibTable, GetExtendedTcpTable, GetExtendedUdpTable, GetIfEntry2, GetIpForwardTable2,
    GetIpInterfaceEntry, MIB_IF_ROW2, MIB_IPFORWARD_TABLE2, MIB_IPINTERFACE_ROW,
    MIB_TCPTABLE_OWNER_PID, MIB_UDPTABLE_OWNER_PID, TCP_TABLE_OWNER_PID_ALL, UDP_TABLE_OWNER_PID,
};
use windows_sys::Win32::Networking::WinSock::{AF_INET, IPPROTO_IP, setsockopt};
use windows_sys::Win32::System::Threading::{
    OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION, QueryFullProcessImageNameW,
};
use windows_sys::Win32::UI::Shell::IsUserAnAdmin;

use super::Proto;

const CREATE_NO_WINDOW: u32 = 0x0800_0000;
const IP_UNICAST_IF: i32 = 31;

/// True if this process runs with administrator rights. The daemon needs them to create the Wintun adapter and its routes.
pub fn is_elevated() -> bool {
    // SAFETY: this call has no arguments and no state.
    unsafe { IsUserAnAdmin() != 0 }
}

/// Connect the standard output and the standard error of this process to the console of the parent process.
///
/// `seltund.exe` is a program of the Windows subsystem. Thus it has no console window of its own, and Task Scheduler does not open a window for it. For a foreground run in a terminal, this function gives the daemon the output to that terminal. Ctrl+C then also reaches the daemon. If the parent has no console, or the output is redirected, the call does nothing.
pub fn attach_parent_console() {
    use std::os::windows::io::IntoRawHandle;

    use windows_sys::Win32::Foundation::INVALID_HANDLE_VALUE;
    use windows_sys::Win32::System::Console::{
        ATTACH_PARENT_PROCESS, AttachConsole, GetStdHandle, STD_ERROR_HANDLE, STD_OUTPUT_HANDLE,
        SetStdHandle,
    };

    // SAFETY: `AttachConsole` has no pointer arguments. `SetStdHandle` receives a valid handle. The code leaks the handle on purpose, because the process uses it until it stops.
    unsafe {
        // The process already has an output if the parent redirected it to a file or a pipe. Then keep it.
        let current = GetStdHandle(STD_ERROR_HANDLE);
        if !current.is_null() && current != INVALID_HANDLE_VALUE {
            return;
        }
        if AttachConsole(ATTACH_PARENT_PROCESS) == 0 {
            return;
        }
        if let Ok(out) = std::fs::OpenOptions::new().write(true).open("CONOUT$") {
            let handle = out.into_raw_handle();
            SetStdHandle(STD_OUTPUT_HANDLE, handle);
            SetStdHandle(STD_ERROR_HANDLE, handle);
        }
    }
}

/// A row with `(local address, local port, remote address, remote port, pid)` for a socket. UDP has no remote address.
type Row = (Ipv4Addr, u16, Ipv4Addr, u16, u32);

fn port(raw: u32) -> u16 {
    u16::from_be((raw & 0xffff) as u16)
}

fn ip(raw: u32) -> Ipv4Addr {
    Ipv4Addr::from(u32::from_be(raw))
}

/// Call an IP Helper table function two times. The first call gives the size.
fn fetch_table(mut call: impl FnMut(*mut core::ffi::c_void, &mut u32) -> u32) -> Option<Vec<u8>> {
    let mut size = 0u32;
    let mut buf: Vec<u8> = Vec::new();
    for _ in 0..4 {
        buf.resize(size as usize + 8, 0);
        // The structures are `repr(C)` and have only `u32` fields. In practice, the allocator gives a buffer with 8-byte alignment. The code also reads the rows unaligned, to be safe.
        let code = call(buf.as_mut_ptr().cast(), &mut size);
        match code {
            0 => return Some(buf),
            122 => continue, // ERROR_INSUFFICIENT_BUFFER: `size` now has the necessary size.
            _ => return None,
        }
    }
    None
}

fn tcp_rows() -> Vec<Row> {
    let Some(buf) = fetch_table(|ptr, size| {
        // SAFETY: `ptr` points to a buffer of `*size` bytes. `fetch_table` owns the buffer.
        unsafe { GetExtendedTcpTable(ptr, size, 0, AF_INET as u32, TCP_TABLE_OWNER_PID_ALL, 0) }
    }) else {
        return Vec::new();
    };
    // SAFETY: the call succeeded. Thus the buffer has `dwNumEntries` rows after the count.
    unsafe {
        let table = buf.as_ptr().cast::<MIB_TCPTABLE_OWNER_PID>();
        let n = (*table).dwNumEntries as usize;
        let rows = std::ptr::addr_of!((*table).table)
            .cast::<windows_sys::Win32::NetworkManagement::IpHelper::MIB_TCPROW_OWNER_PID>();
        (0..n)
            .map(|i| {
                let r = rows.add(i).read_unaligned();
                (
                    ip(r.dwLocalAddr),
                    port(r.dwLocalPort),
                    ip(r.dwRemoteAddr),
                    port(r.dwRemotePort),
                    r.dwOwningPid,
                )
            })
            .collect()
    }
}

fn udp_rows() -> Vec<Row> {
    let Some(buf) = fetch_table(|ptr, size| {
        // SAFETY: same as in `tcp_rows`.
        unsafe { GetExtendedUdpTable(ptr, size, 0, AF_INET as u32, UDP_TABLE_OWNER_PID, 0) }
    }) else {
        return Vec::new();
    };
    // SAFETY: same as in `tcp_rows`.
    unsafe {
        let table = buf.as_ptr().cast::<MIB_UDPTABLE_OWNER_PID>();
        let n = (*table).dwNumEntries as usize;
        let rows = std::ptr::addr_of!((*table).table)
            .cast::<windows_sys::Win32::NetworkManagement::IpHelper::MIB_UDPROW_OWNER_PID>();
        (0..n)
            .map(|i| {
                let r = rows.add(i).read_unaligned();
                (
                    ip(r.dwLocalAddr),
                    port(r.dwLocalPort),
                    Ipv4Addr::UNSPECIFIED,
                    0,
                    r.dwOwningPid,
                )
            })
            .collect()
    }
}

/// The full path of the executable of `pid`. If the process cannot be opened, the result is none. System processes and protected processes cannot be opened.
fn image_of(pid: u32) -> Option<String> {
    if pid == 0 || pid == 4 {
        return None;
    }
    // SAFETY: the code closes the handle before it returns. The call receives the length of the buffer.
    unsafe {
        let handle = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
        if handle.is_null() {
            return None;
        }
        let mut buf = vec![0u16; 32768];
        let mut len = buf.len() as u32;
        let ok = QueryFullProcessImageNameW(handle, 0, buf.as_mut_ptr(), &mut len);
        CloseHandle(handle);
        (ok != 0).then(|| String::from_utf16_lossy(&buf[..len as usize]))
    }
}

struct Snapshot {
    at: Instant,
    tcp: Vec<Row>,
    udp: Vec<Row>,
}

/// The owner of a socket. This type keeps a snapshot of the TCP and UDP tables. It makes a new snapshot if a lookup finds nothing, because a new connection is only a few milliseconds old when its first packet arrives. It also makes a new snapshot if the hit is old, because a port can have a new owner.
pub struct Owners {
    snapshot: Mutex<Option<Snapshot>>,
}

impl Default for Owners {
    fn default() -> Self {
        Self {
            snapshot: Mutex::new(None),
        }
    }
}

const FRESH: Duration = Duration::from_millis(1000);
const MISS_REFRESH: Duration = Duration::from_millis(20);

impl Owners {
    fn find(
        rows: &[Row],
        proto: Proto,
        local: SocketAddr,
        remote: Option<SocketAddr>,
    ) -> Option<u32> {
        let SocketAddr::V4(local) = local else {
            return None;
        };
        rows.iter()
            .find(|(addr, p, raddr, rport, _)| {
                *p == local.port()
                    && (*addr == *local.ip() || addr.is_unspecified())
                    && match (proto, remote) {
                        (Proto::Tcp, Some(SocketAddr::V4(r))) => {
                            *raddr == *r.ip() && *rport == r.port()
                        },
                        _ => true,
                    }
            })
            .map(|row| row.4)
    }

    /// The executable of the process that owns the socket `local`. For TCP, the lookup also uses `remote`.
    pub fn image(
        &self,
        proto: Proto,
        local: SocketAddr,
        remote: Option<SocketAddr>,
    ) -> Option<String> {
        let mut guard = self.snapshot.lock().unwrap_or_else(|e| e.into_inner());
        for attempt in 0..2 {
            let snap = guard.as_ref();
            let rows = |s: &Snapshot| {
                if proto == Proto::Tcp {
                    s.tcp.clone()
                } else {
                    s.udp.clone()
                }
            };
            if let Some(snap) = snap {
                let age = snap.at.elapsed();
                if let Some(pid) = Self::find(&rows(snap), proto, local, remote) {
                    if age <= FRESH || attempt == 1 {
                        return image_of(pid);
                    }
                } else if age <= MISS_REFRESH || attempt == 1 {
                    return None;
                }
            }
            *guard = Some(Snapshot {
                at: Instant::now(),
                tcp: tcp_rows(),
                udp: udp_rows(),
            });
        }
        None
    }
}

/// The index of the physical interface. It has the default route with the lowest metric and is not `exclude` (the TUN adapter). Traffic that must not enter the TUN leaves through this interface. Each call finds the interface again, because Wi-Fi and Ethernet connect and disconnect.
pub fn physical_interface(exclude: u32) -> Option<u32> {
    let mut table: *mut MIB_IPFORWARD_TABLE2 = std::ptr::null_mut();
    // SAFETY: if the call succeeds, `table` is a block that IP Helper allocated. The code frees it below. The code reads rows only inside `NumEntries`.
    unsafe {
        if GetIpForwardTable2(AF_INET, &mut table) != 0 || table.is_null() {
            return None;
        }
        let n = (*table).NumEntries as usize;
        let rows = std::ptr::addr_of!((*table).Table)
            .cast::<windows_sys::Win32::NetworkManagement::IpHelper::MIB_IPFORWARD_ROW2>();
        let mut best: Option<(u32, u32)> = None;
        for i in 0..n {
            let row = rows.add(i).read_unaligned();
            if row.DestinationPrefix.PrefixLength != 0
                || row.InterfaceIndex == exclude
                || row.DestinationPrefix.Prefix.Ipv4.sin_addr.S_un.S_addr != 0
            {
                continue;
            }
            let mut iface: MIB_IPINTERFACE_ROW = std::mem::zeroed();
            iface.Family = AF_INET;
            iface.InterfaceIndex = row.InterfaceIndex;
            if GetIpInterfaceEntry(&mut iface) != 0 {
                continue;
            }
            // A disconnected adapter keeps its route and often has a better metric. An example is a TAP adapter with an old default route. Only an adapter that is up and connected counts.
            let mut link: MIB_IF_ROW2 = std::mem::zeroed();
            link.InterfaceIndex = row.InterfaceIndex;
            if GetIfEntry2(&mut link) != 0 || link.OperStatus != 1 || link.MediaConnectState == 2 {
                continue;
            }
            let metric = row.Metric.saturating_add(iface.Metric);
            if best.is_none_or(|(m, _)| metric < m) {
                best = Some((metric, row.InterfaceIndex));
            }
        }
        FreeMibTable(table.cast());
        best.map(|(_, index)| index)
    }
}

/// Pin `socket` to the interface `index` (`IP_UNICAST_IF`). The traffic of the socket leaves through this interface, although the TUN routes cover all destinations.
pub fn bind_to_interface<S: AsRawSocket>(socket: &S, index: u32) -> Result<()> {
    // For IPv4, the index is in network byte order.
    let value = index.to_be();
    // SAFETY: `value` is valid during the call, and the call receives its size.
    let code = unsafe {
        setsockopt(
            socket.as_raw_socket() as usize,
            IPPROTO_IP,
            IP_UNICAST_IF,
            (&raw const value).cast(),
            size_of::<u32>() as i32,
        )
    };
    if code == 0 {
        Ok(())
    } else {
        Err(Error::last_os_error())
    }
}

fn netsh(args: &[&str]) -> Result<()> {
    let output = Command::new("netsh")
        .args(args)
        .creation_flags(CREATE_NO_WINDOW)
        .output()?;
    if output.status.success() {
        Ok(())
    } else {
        let text = String::from_utf8_lossy(&output.stdout);
        Err(Error::other(format!(
            "netsh {}: {}",
            args.join(" "),
            text.trim()
        )))
    }
}

/// The two half-default routes (`0.0.0.0/1` and `128.0.0.0/1`). They send each destination that has no more specific route into the TUN adapter. LAN, loopback, and the subnet of the TUN have more specific routes. With these routes, the packets of a process leave from the address of the TUN. The routes go away with the adapter. The code also removes them when the daemon stops.
pub struct Routes {
    interface: String,
}

const HALVES: [&str; 2] = ["0.0.0.0/1", "128.0.0.0/1"];

impl Routes {
    pub fn install(tun_index: u32) -> Result<Self> {
        let interface = tun_index.to_string();
        let routes = Self { interface };
        for half in HALVES {
            let arg = format!("interface={}", routes.interface);
            netsh(&[
                "interface",
                "ipv4",
                "add",
                "route",
                half,
                &arg,
                "nexthop=0.0.0.0",
                "metric=1",
                "store=active",
            ])?;
        }
        Ok(routes)
    }
}

impl Drop for Routes {
    fn drop(&mut self) {
        for half in HALVES {
            let arg = format!("interface={}", self.interface);
            let _ = netsh(&[
                "interface",
                "ipv4",
                "delete",
                "route",
                half,
                &arg,
                "nexthop=0.0.0.0",
                "store=active",
            ]);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::TcpListener;

    #[test]
    fn finds_this_process_as_the_owner_of_its_listener() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let local = listener.local_addr().unwrap();
        // A listening socket has the remote address 0.0.0.0:0. Thus the test uses a connected pair.
        let client = std::net::TcpStream::connect(local).unwrap();
        let (_server, _) = listener.accept().unwrap();
        let owners = Owners::default();
        let image = owners
            .image(Proto::Tcp, client.local_addr().unwrap(), Some(local))
            .unwrap();
        let me = std::env::current_exe().unwrap();
        assert_eq!(image.to_lowercase(), me.to_string_lossy().to_lowercase());
    }

    #[test]
    fn finds_a_default_interface() {
        // A computer that is offline has no default route. A computer that is online must give a real index.
        if let Some(index) = physical_interface(u32::MAX) {
            assert!(index > 0);
            // Regression test: a disconnected TAP adapter with an old default route once had the lowest metric.
            let mut link: MIB_IF_ROW2 = unsafe { std::mem::zeroed() };
            link.InterfaceIndex = index;
            assert_eq!(unsafe { GetIfEntry2(&mut link) }, 0);
            assert_eq!(link.OperStatus, 1, "The interface {index} is not up.");
        }
    }

    #[test]
    fn port_and_ip_decode_network_order() {
        assert_eq!(port(0x5000), 80);
        assert_eq!(ip(u32::from_ne_bytes([127, 0, 0, 1])), Ipv4Addr::LOCALHOST);
    }

    #[test]
    fn owners_default_is_empty() {
        assert!(
            Owners::default()
                .image(Proto::Udp, "127.0.0.1:1".parse().unwrap(), None)
                .is_none()
        );
    }
}
