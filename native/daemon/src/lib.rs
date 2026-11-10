//! `seltund`, the seltun daemon: steers selected processes into a TUN adapter and forwards them to Clash.
//!
//! See `docs/daemon.md` for the design.

pub mod config;
pub mod control;
pub mod daemon;
pub mod forward;
pub mod matcher;
pub mod platform;
pub mod socks;
pub mod state;
