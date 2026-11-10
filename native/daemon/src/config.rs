//! Command line and the tables of `config.toml` the daemon reads.
//!
//! The Python CLI reads the same file, so the control address the CLI dials and the one the daemon binds cannot drift apart. The daemon reads `[upstream]`, `[tun]`, `[processes]` and `[daemon]`; an unknown key inside one of them is an error, so a typo cannot silently fall back to a default.

use std::net::{Ipv4Addr, SocketAddr};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use clap::Parser;
use serde::Deserialize;

pub const DEFAULT_UPSTREAM: &str = "127.0.0.1:7897";
pub const DEFAULT_CONTROL: &str = "127.0.0.1:7898";
pub const DEFAULT_TUN_NAME: &str = "seltun";
pub const DEFAULT_TUN_ADDRESS: &str = "198.19.0.1";

#[derive(Parser, Debug, Clone)]
#[command(
    name = "seltund",
    version,
    about = "Steers selected processes into a TUN adapter and forwards them to Clash."
)]
pub struct Cli {
    /// The config shared with the `seltun` CLI.
    #[arg(long, default_value = "config.toml")]
    pub config: PathBuf,

    /// The state file (`{"enabled": bool}`) the CLI writes.
    #[arg(long, default_value = "state.json")]
    pub state: PathBuf,

    /// Append the log here instead of writing it to stderr (what autostart uses).
    #[arg(long)]
    pub log_file: Option<PathBuf>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum Protocol {
    #[default]
    Socks5,
    Http,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Upstream {
    /// Clash's mixed (or SOCKS5 / HTTP) port.
    pub address: SocketAddr,
    pub protocol: Protocol,
    /// Whether UDP of selected processes goes through Clash (SOCKS5 only); otherwise only DNS goes direct and the rest is dropped.
    pub udp: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Tun {
    pub name: String,
    pub address: Ipv4Addr,
    pub mtu: u16,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Settings {
    pub upstream: Upstream,
    pub tun: Tun,
    /// Entries of `[processes].names`, as written.
    pub processes: Vec<String>,
    pub control: SocketAddr,
}

#[derive(Deserialize, Default, Debug)]
#[serde(rename_all = "kebab-case", deny_unknown_fields)]
struct UpstreamTable {
    address: Option<String>,
    protocol: Option<Protocol>,
    udp: Option<bool>,
}

#[derive(Deserialize, Default, Debug)]
#[serde(rename_all = "kebab-case", deny_unknown_fields)]
struct TunTable {
    name: Option<String>,
    address: Option<String>,
    mtu: Option<u16>,
}

#[derive(Deserialize, Default, Debug)]
#[serde(rename_all = "kebab-case", deny_unknown_fields)]
struct ProcessesTable {
    names: Option<Vec<String>>,
}

#[derive(Deserialize, Default, Debug)]
#[serde(rename_all = "kebab-case", deny_unknown_fields)]
struct DaemonTable {
    control: Option<String>,
}

/// The tables the daemon reads; the other tables belong to the Python side and are ignored here.
#[derive(Deserialize, Default, Debug)]
struct File {
    upstream: Option<UpstreamTable>,
    tun: Option<TunTable>,
    processes: Option<ProcessesTable>,
    daemon: Option<DaemonTable>,
}

fn parse_addr(what: &str, text: &str) -> Result<SocketAddr> {
    text.parse()
        .with_context(|| format!("{what} must be host:port with a numeric IP, got {text:?}"))
}

impl Settings {
    /// Read `path`; a missing file means every default.
    pub fn load(path: &Path) -> Result<Self> {
        let text = match std::fs::read_to_string(path) {
            Ok(text) => text,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => String::new(),
            Err(error) => return Err(error).with_context(|| format!("reading {}", path.display())),
        };
        Self::parse(&text).with_context(|| format!("in {}", path.display()))
    }

    pub fn parse(text: &str) -> Result<Self> {
        let file: File = toml::from_str(text)?;
        let upstream = file.upstream.unwrap_or_default();
        let tun = file.tun.unwrap_or_default();
        let address: Ipv4Addr = {
            let text = tun.address.as_deref().unwrap_or(DEFAULT_TUN_ADDRESS);
            text.parse()
                .with_context(|| format!("[tun].address must be an IPv4 address, got {text:?}"))?
        };
        if address.is_loopback() || address.is_unspecified() {
            bail!("[tun].address must not be loopback or unspecified");
        }
        let protocol = upstream.protocol.unwrap_or_default();
        let udp = upstream.udp.unwrap_or(false);
        if udp && protocol != Protocol::Socks5 {
            bail!("[upstream].udp needs protocol = \"socks5\"");
        }
        Ok(Self {
            upstream: Upstream {
                address: parse_addr(
                    "[upstream].address",
                    upstream.address.as_deref().unwrap_or(DEFAULT_UPSTREAM),
                )?,
                protocol,
                udp,
            },
            tun: Tun {
                name: tun.name.unwrap_or_else(|| DEFAULT_TUN_NAME.into()),
                address,
                mtu: tun.mtu.unwrap_or(1500),
            },
            processes: file.processes.unwrap_or_default().names.unwrap_or_default(),
            control: parse_addr(
                "[daemon].control",
                file.daemon
                    .unwrap_or_default()
                    .control
                    .as_deref()
                    .unwrap_or(DEFAULT_CONTROL),
            )?,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_file_is_all_defaults() {
        let s = Settings::parse("").unwrap();
        assert_eq!(s.upstream.address.to_string(), DEFAULT_UPSTREAM);
        assert_eq!(s.control.to_string(), DEFAULT_CONTROL);
        assert_eq!(s.tun.name, "seltun");
        assert!(s.processes.is_empty() && !s.upstream.udp);
    }

    #[test]
    fn reads_tables_and_ignores_foreign_ones() {
        let s = Settings::parse(
            "[upstream]\naddress = \"127.0.0.1:1080\"\nprotocol = \"http\"\n[processes]\nnames = [\"a.exe\"]\n[other]\nx = 1\n",
        )
        .unwrap();
        assert_eq!(s.upstream.address.port(), 1080);
        assert_eq!(s.upstream.protocol, Protocol::Http);
        assert_eq!(s.processes, ["a.exe"]);
    }

    #[test]
    fn unknown_key_is_rejected() {
        assert!(Settings::parse("[upstream]\nadress = \"x\"\n").is_err());
    }

    #[test]
    fn udp_needs_socks5() {
        assert!(Settings::parse("[upstream]\nprotocol = \"http\"\nudp = true\n").is_err());
    }
}
