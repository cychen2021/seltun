//! The command line and the tables of `config.toml` that the daemon reads.
//!
//! The Python CLI reads the same file. Thus the control address that the CLI uses and the address that the daemon binds are always the same. The daemon reads `[upstream]`, `[tun]`, `[processes]` and `[daemon]`. An unknown key in one of these tables is an error. Thus a typo cannot select a default without a warning.

use std::net::{Ipv4Addr, SocketAddr};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use clap::Parser;
use serde::Deserialize;

pub const DEFAULT_UPSTREAM: &str = "127.0.0.1:1080";
pub const DEFAULT_CONTROL: &str = "127.0.0.1:7898";
pub const DEFAULT_TUN_NAME: &str = "seltun";
pub const DEFAULT_TUN_ADDRESS: &str = "198.19.0.1";

#[derive(Parser, Debug, Clone)]
#[command(
    name = "seltund",
    version,
    about = "Steer selected processes into a TUN adapter and forward them to a local proxy."
)]
pub struct Cli {
    /// The config file. The `seltun` CLI uses the same file.
    #[arg(long, default_value = "config.toml")]
    pub config: PathBuf,

    /// The state file (`{"enabled": bool}`). The CLI writes this file.
    #[arg(long, default_value = "state.json")]
    pub state: PathBuf,

    /// Add the log to this file. Without this option, the daemon writes the log to stderr. Autostart uses this option.
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
    /// The address of the local proxy (SOCKS5 or HTTP).
    pub address: SocketAddr,
    pub protocol: Protocol,
    /// If true, the UDP of the selected processes goes through the proxy (SOCKS5 only). If false, DNS goes direct and the daemon drops the other UDP.
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
    /// The entries of `[processes].names`, as they are in the file.
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

/// The tables that the daemon reads. The daemon ignores the other tables. They are for the Python side.
#[derive(Deserialize, Default, Debug)]
struct File {
    upstream: Option<UpstreamTable>,
    tun: Option<TunTable>,
    processes: Option<ProcessesTable>,
    daemon: Option<DaemonTable>,
}

fn parse_addr(what: &str, text: &str) -> Result<SocketAddr> {
    text.parse().with_context(|| {
        format!("{what} must have the form host:port with a numeric IP. The value is {text:?}.")
    })
}

impl Settings {
    /// Read `path`. If the file is missing, the daemon uses all defaults.
    pub fn load(path: &Path) -> Result<Self> {
        let text = match std::fs::read_to_string(path) {
            Ok(text) => text,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => String::new(),
            Err(error) => {
                return Err(error).with_context(|| format!("Cannot read {}", path.display()));
            },
        };
        Self::parse(&text).with_context(|| format!("Error in {}", path.display()))
    }

    pub fn parse(text: &str) -> Result<Self> {
        let file: File = toml::from_str(text)?;
        let upstream = file.upstream.unwrap_or_default();
        let tun = file.tun.unwrap_or_default();
        let address: Ipv4Addr = {
            let text = tun.address.as_deref().unwrap_or(DEFAULT_TUN_ADDRESS);
            text.parse().with_context(|| {
                format!("[tun].address must be an IPv4 address. The value is {text:?}.")
            })?
        };
        if address.is_loopback() || address.is_unspecified() {
            bail!("[tun].address must not be a loopback address or an unspecified address.");
        }
        let protocol = upstream.protocol.unwrap_or_default();
        let udp = upstream.udp.unwrap_or(false);
        if udp && protocol != Protocol::Socks5 {
            bail!("[upstream].udp = true needs protocol = \"socks5\".");
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
