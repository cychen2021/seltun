//! What the daemon needs from the operating system: who owns a socket, which interface is the real one, and the routes that send traffic into the TUN adapter.
//!
//! Windows only; elsewhere every function reports "nothing found", so the pure logic builds and tests anywhere.

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
