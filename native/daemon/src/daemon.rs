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

#[cfg(windows)]
async fn start(settings: Settings) -> Result<Active> {
    use std::net::Ipv4Addr;

    use anyhow::Context as _;
    use tun::AbstractDevice;

    use crate::forward::{self, Context};
    use crate::matcher::Matcher;
    use crate::platform::{Routes, is_elevated};

    /// A fixed adapter GUID. Windows then keeps one network profile for the adapter. Without it, Windows makes a new profile at each start.
    const ADAPTER_GUID: u128 = 0x5e17_0a11_c0de_4f1d_9a3b_2c64_7e01_5e17;

    if !is_elevated() {
        bail!(
            "To create the TUN adapter, the daemon must run with administrator rights. Run `seltun autostart enable` one time in an elevated terminal. Then run `seltun daemon start`."
        );
    }
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
    let device = tun::create_as_async(&config).context("Cannot create the Wintun adapter")?;
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
        if let Err(error) = forward::run(device, ctx, mtu, stopped).await {
            tracing::error!("The data path failed: {error:#}");
            *report.lock().unwrap_or_else(|e| e.into_inner()) = Some(format!("{error:#}"));
        }
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

#[cfg(not(windows))]
async fn start(_settings: Settings) -> Result<Active> {
    bail!("seltund works on Windows only.")
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
