//! The services that the daemon needs from the operating system: the owner of a socket, the physical interface, and the routes that send traffic into the TUN adapter.
//!
//! The code works on Windows only. On other systems, each function reports that it found nothing. Thus the pure logic builds and tests on each system.

#[cfg(windows)]
mod windows;
#[cfg(windows)]
pub use windows::*;

#[cfg(not(windows))]
mod other;
#[cfg(not(windows))]
pub use other::*;

use std::net::IpAddr;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Proto {
    Tcp,
    Udp,
}

/// True if the TUN adapter can use the DNS server `ip` of the physical interface. These servers are not usable:
///
/// - The placeholders `fec0:0:0:ffff::1` to `::3`. Windows shows them on an IPv6 interface without servers. They do not answer.
/// - Link-local IPv6 servers. They need the scope of the physical interface, and the TUN adapter has a different scope.
/// - The unspecified address.
pub fn is_usable_dns(ip: &IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => !v4.is_unspecified(),
        IpAddr::V6(v6) => {
            let s = v6.segments();
            let placeholder = s[..7] == [0xfec0, 0, 0, 0xffff, 0, 0, 0] && (1..=3).contains(&s[7]);
            let link_local = s[0] & 0xffc0 == 0xfe80;
            !(placeholder || link_local || v6.is_unspecified())
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn usable(ip: &str) -> bool {
        is_usable_dns(&ip.parse().unwrap())
    }

    #[test]
    fn real_servers_are_usable() {
        assert!(usable("192.168.1.1"));
        assert!(usable("127.0.0.1"));
        assert!(usable("2001:4860:4860::8888"));
        assert!(usable("fd7a:115c:a1e0::53"));
    }

    #[test]
    fn placeholders_and_link_local_servers_are_not_usable() {
        assert!(!usable("fec0:0:0:ffff::1"));
        assert!(!usable("fec0:0:0:ffff::3"));
        assert!(!usable("fe80::1"));
        assert!(!usable("0.0.0.0"));
        assert!(!usable("::"));
        // Only the three placeholders. Other site-local addresses are real servers.
        assert!(usable("fec0:0:0:ffff::4"));
    }
}
