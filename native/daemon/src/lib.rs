//! `seltund`, the seltun daemon. It steers selected processes into a TUN adapter and forwards them to a local proxy.
//!
//! See `docs/daemon.md` for more information.

pub mod config;
pub mod control;
pub mod daemon;
pub mod forward;
pub mod matcher;
pub mod platform;
pub mod socks;
pub mod state;
