//! Stand-ins for systems other than Windows. The daemon works on Windows only. The stand-ins let the rest of the code compile and test.

use std::io::{Error, ErrorKind, Result};
use std::net::SocketAddr;

use super::Proto;

/// Do nothing. Only Windows has a console that the daemon must attach to.
pub fn attach_parent_console() {}

pub fn is_elevated() -> bool {
    false
}

#[derive(Default)]
pub struct Owners;

impl Owners {
    pub fn image(
        &self,
        _proto: Proto,
        _local: SocketAddr,
        _remote: Option<SocketAddr>,
    ) -> Option<String> {
        None
    }
}

pub fn physical_interface(_exclude: u32) -> Option<u32> {
    None
}

/// Pin `socket` to the interface with this index. The traffic of the socket then does not use the TUN routes.
pub fn bind_to_interface<S>(_socket: &S, _index: u32) -> Result<()> {
    Ok(())
}

pub struct Routes;

impl Routes {
    pub fn install(_tun_index: u32) -> Result<Self> {
        Err(Error::new(
            ErrorKind::Unsupported,
            "seltund works on Windows only.",
        ))
    }
}
