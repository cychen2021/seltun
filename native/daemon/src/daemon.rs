//! The control logic of the daemon. It makes what is running the same as what the user selected (`state.json` and `config.toml`).
//!
//! "On" means that a TUN adapter, its routes, and the task that forwards the flows are present. "Off" means that they are not present, and the daemon only answers on the control channel. [`Daemon::reload`] is the only entry point. The CLI writes the state file and then calls it. The startup also calls it one time, to get the saved state.

use std::sync::{Arc, Mutex};

use anyhow::{Result, bail};
use serde::Serialize;
use tokio::sync::{Mutex as AsyncMutex, watch};
use tokio::task::JoinHandle;

use crate::config::{Cli, Settings};
use crate::state::State;

/// The information that `seltun status` shows.
#[derive(Serialize, Debug, Clone)]
pub struct Status {
    pub pid: u32,
    /// The state that the user selected (the saved switch).
    pub enabled: bool,
    /// True if the TUN adapter is up and the daemon forwards the flows.
    pub active: bool,
    /// The reason why the daemon is not in the selected state, if it is not.
    pub error: Option<String>,
    pub processes: Vec<String>,
    pub upstream: String,
    pub elevated: bool,
}

struct Active {
    settings: Settings,
    stop: watch::Sender<bool>,
    task: JoinHandle<()>,
    /// The task sets this value if the data path stops with an error.
    failure: Arc<Mutex<Option<String>>>,
}

impl Active {
    fn running(&self) -> bool {
        !self.task.is_finished()
    }

    async fn stop(self) {
        let _ = self.stop.send(true);
        let _ = self.task.await;
    }
}

#[derive(Default)]
struct Inner {
    active: Option<Active>,
    error: Option<String>,
    enabled: bool,
    settings: Option<Settings>,
}

pub struct Daemon {
    cli: Cli,
    inner: AsyncMutex<Inner>,
}

impl Daemon {
    pub fn new(cli: Cli) -> Self {
        Self {
            cli,
            inner: AsyncMutex::new(Inner::default()),
        }
    }

    /// Read the state and the config again, and make the daemon match them. The returned status shows the result.
    pub async fn reload(&self) -> Status {
        let mut inner = self.inner.lock().await;
        inner.error = None;
        let state = State::load(&self.cli.state);
        let settings = Settings::load(&self.cli.config);
        match (state, settings) {
            (Ok(state), Ok(settings)) => {
                inner.enabled = state.enabled;
                inner.settings = Some(settings.clone());
                if let Err(error) = Self::reconcile(&mut inner, state.enabled, settings).await {
                    tracing::error!("{error:#}");
                    inner.error = Some(format!("{error:#}"));
                }
            },
            (state, settings) => {
                // Do not change what runs. A typo in the config must not stop the traffic of the user.
                let error = state.err().or(settings.err()).map(|e| format!("{e:#}"));
                tracing::error!("{}", error.as_deref().unwrap_or_default());
                inner.error = error;
            },
        }
        Self::snapshot(&mut inner)
    }

    pub async fn status(&self) -> Status {
        Self::snapshot(&mut *self.inner.lock().await)
    }

    /// Remove the data path. The daemon stops.
    pub async fn stop(&self) {
        if let Some(active) = self.inner.lock().await.active.take() {
            active.stop().await;
        }
    }

    async fn reconcile(inner: &mut Inner, enabled: bool, settings: Settings) -> Result<()> {
        if let Some(active) = &inner.active
            && (!active.running() || !enabled || active.settings != settings)
        {
            let active = inner.active.take().expect("checked above");
            if let Some(error) = active.failure.lock().ok().and_then(|mut f| f.take()) {
                tracing::warn!("The previous data path failed: {error}");
            }
            active.stop().await;
            tracing::info!("Steering stopped.");
        }
        if enabled && inner.active.is_none() {
            if settings.processes.is_empty() {
                bail!(
                    "The switch is on, but [processes].names is empty. There is no process to steer."
                );
            }
            inner.active = Some(start(settings).await?);
            tracing::info!("Steering started.");
        }
        Ok(())
    }

    fn snapshot(inner: &mut Inner) -> Status {
        let active = inner.active.as_ref().is_some_and(Active::running);
        if let Some(a) = &inner.active
            && !a.running()
            && let Some(failure) = a.failure.lock().ok().and_then(|f| f.clone())
        {
            inner.error.get_or_insert(failure);
        }
        let settings = inner.settings.as_ref();
        Status {
            pid: std::process::id(),
            enabled: inner.enabled,
            active,
            error: inner.error.clone(),
            processes: settings.map(|s| s.processes.clone()).unwrap_or_default(),
            upstream: settings
                .map(|s| s.upstream.address.to_string())
                .unwrap_or_default(),
            elevated: crate::platform::is_elevated(),
        }
    }
}

#[cfg(any(windows, target_os = "macos"))]
async fn start(settings: Settings) -> Result<Active> {
    use anyhow::Context as _;
    use tun::AbstractDevice;

    use crate::forward::{self, Context};
    use crate::matcher::Matcher;
    use crate::platform::{ELEVATION_HINT, Routes, is_elevated};

    if !is_elevated() {
        bail!(ELEVATION_HINT);
    }
    let device = create_device(&settings)?;
    let index = device
        .tun_index()
        .context("Cannot read the adapter index")? as u32;
    let routes = Routes::install(index).context("Cannot add the TUN routes")?;

    let ctx = Arc::new(Context::new(
        settings.upstream.clone(),
        Matcher::new(&settings.processes),
        index,
    ));
    let (stop, stopped) = watch::channel(false);
    let failure = Arc::new(Mutex::new(None));
    let report = failure.clone();
    let mtu = settings.tun.mtu;
    let task = tokio::spawn(async move {
        #[cfg(windows)]
        let dns = tokio::spawn(mirror_dns(index));
        #[cfg(target_os = "macos")]
        let scoped = tokio::spawn(keep_scoped_default(index));
        if let Err(error) = forward::run(device, ctx, mtu, stopped).await {
            tracing::error!("The data path failed: {error:#}");
            *report.lock().unwrap_or_else(|e| e.into_inner()) = Some(format!("{error:#}"));
        }
        #[cfg(windows)]
        dns.abort();
        #[cfg(target_os = "macos")]
        scoped.abort();
        // The adapter goes away with the stack, and the routes go with it. The code also removes the routes, to be sure.
        drop(routes);
    });
    Ok(Active {
        settings,
        stop,
        task,
        failure,
    })
}

#[cfg(windows)]
fn create_device(settings: &Settings) -> Result<tun::AsyncDevice> {
    use std::net::Ipv4Addr;

    use anyhow::Context as _;

    /// A fixed adapter GUID. Windows then keeps one network profile for the adapter. Without it, Windows makes a new profile at each start.
    const ADAPTER_GUID: u128 = 0x5e17_0a11_c0de_4f1d_9a3b_2c64_7e01_5e17;

    let wintun = std::env::current_exe()?.with_file_name("wintun.dll");
    if !wintun.is_file() {
        bail!(
            "{} is missing. Run `seltun init` to download it.",
            wintun.display()
        );
    }
    let mut config = tun::Configuration::default();
    config
        .tun_name(&settings.tun.name)
        .address(settings.tun.address)
        // A /30 network: the address of the adapter and a nominal peer. The half-default routes send the traffic into the adapter.
        .netmask(Ipv4Addr::new(255, 255, 255, 252))
        .mtu(settings.tun.mtu)
        // The highest interface metric. Windows then prefers the physical connection as the primary network. Without this, the adapter has a low automatic metric, and Windows shows the adapter (a wired network) instead of Wi-Fi. The metric does not change the forwarding, because the routes of the adapter have longer prefixes than the default route.
        .metric(9999)
        .up();
    config.platform_config(|platform| {
        platform.device_guid(ADAPTER_GUID);
        platform.wintun_file(wintun.as_os_str());
    });
    tun::create_as_async(&config).context("Cannot create the Wintun adapter")
}

/// Create a utun interface. macOS gives it the next free name (`utunN`), so the daemon does not use `[tun].name`. A utun interface is point to point. Its peer is the next address in the /30 network.
#[cfg(target_os = "macos")]
fn create_device(settings: &Settings) -> Result<tun::AsyncDevice> {
    use std::net::Ipv4Addr;

    use anyhow::Context as _;

    let peer = Ipv4Addr::from(u32::from(settings.tun.address).wrapping_add(1));
    let mut config = tun::Configuration::default();
    config
        .address(settings.tun.address)
        .destination(peer)
        .netmask(Ipv4Addr::new(255, 255, 255, 252))
        .mtu(settings.tun.mtu)
        .up();
    tun::create_as_async(&config).context("Cannot create the utun interface")
}

/// The time between two checks of the DNS servers of the physical interface.
#[cfg(windows)]
const DNS_CHECK: std::time::Duration = std::time::Duration::from_secs(5);

/// Give the TUN adapter the DNS servers of the physical interface, and keep them the same when the physical interface changes. The task runs until the data path stops it.
///
/// Some programs use only the DNS servers of the interface with the best route to the Internet. Because of the half-default routes, that interface is the TUN adapter. Without servers of its own, the adapter shows only the IPv6 placeholders `fec0:0:0:ffff::1` to `::3`, which do not answer. The programs then cannot resolve names. A query to a copied server takes the same path as before: a server in the local network has a more specific route, and the daemon sends the other queries of a process that is not selected direct.
#[cfg(windows)]
async fn mirror_dns(tun_index: u32) {
    use crate::platform;

    let mut applied: Option<Vec<std::net::IpAddr>> = None;
    loop {
        let servers = tokio::task::spawn_blocking(move || {
            platform::physical_interface(tun_index).map(|index| {
                platform::dns_servers(index)
                    .into_iter()
                    .filter(platform::is_usable_dns)
                    .collect::<Vec<_>>()
            })
        })
        .await
        .ok()
        .flatten();
        if let Some(servers) = servers
            && applied.as_ref() != Some(&servers)
        {
            let list = servers.clone();
            match tokio::task::spawn_blocking(move || platform::set_dns_servers(tun_index, &list))
                .await
            {
                Ok(Ok(())) => tracing::info!(
                    ?servers,
                    "The TUN adapter uses the DNS servers of the physical interface."
                ),
                Ok(Err(error)) => {
                    tracing::warn!("Cannot set the DNS servers of the TUN adapter: {error}")
                },
                Err(_) => {},
            }
            // Also after an error, do not try the same list again. A new list starts a new attempt.
            applied = Some(servers);
        }
        tokio::time::sleep(DNS_CHECK).await;
    }
}

/// The time between two checks of the scoped default route of the primary interface.
#[cfg(target_os = "macos")]
const SCOPED_ROUTE_CHECK: std::time::Duration = std::time::Duration::from_secs(2);

/// Keep a working scoped default route on the primary interface while the data path runs (see `platform::keep_scoped_default`). The task follows a change of the network, for example from home to an office, and it adds the route again if macOS removed it.
#[cfg(target_os = "macos")]
async fn keep_scoped_default(tun_index: u32) {
    loop {
        let _ =
            tokio::task::spawn_blocking(move || crate::platform::keep_scoped_default(tun_index))
                .await;
        tokio::time::sleep(SCOPED_ROUTE_CHECK).await;
    }
}

#[cfg(not(any(windows, target_os = "macos")))]
async fn start(_settings: Settings) -> Result<Active> {
    bail!(crate::platform::ELEVATION_HINT)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn daemon(dir: &std::path::Path, config: &str, state: &str) -> Daemon {
        std::fs::write(dir.join("config.toml"), config).unwrap();
        std::fs::write(dir.join("state.json"), state).unwrap();
        Daemon::new(Cli {
            config: dir.join("config.toml"),
            state: dir.join("state.json"),
            log_file: None,
        })
    }

    fn scratch(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("seltun-daemon-{name}"));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[tokio::test]
    async fn off_means_nothing_runs() {
        let d = daemon(
            &scratch("off"),
            "[processes]\nnames = [\"a.exe\"]\n",
            r#"{"enabled": false}"#,
        );
        let status = d.reload().await;
        assert!(!status.enabled && !status.active && status.error.is_none());
        assert_eq!(status.processes, ["a.exe"]);
    }

    #[tokio::test]
    async fn on_without_processes_is_reported() {
        let d = daemon(&scratch("empty"), "", r#"{"enabled": true}"#);
        let status = d.reload().await;
        assert!(status.enabled && !status.active);
        assert!(status.error.unwrap().contains("names is empty"));
    }

    #[tokio::test]
    async fn broken_config_is_reported_and_keeps_state() {
        let d = daemon(
            &scratch("broken"),
            "[upstream]\nbogus = 1\n",
            r#"{"enabled": false}"#,
        );
        assert!(d.reload().await.error.is_some());
    }
}
