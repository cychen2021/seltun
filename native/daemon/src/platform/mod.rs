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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Proto {
    Tcp,
    Udp,
}
