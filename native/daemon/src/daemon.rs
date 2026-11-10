//! The daemon's brain: reconcile what the user asked for (`state.json`, `config.toml`) with what is running.
//!
//! "On" means a TUN adapter, its routes and the forwarding task exist; "off" means none of that does, and the daemon only answers the control channel. [`Daemon::reload`] is the one entry point: the CLI writes the state file and calls it, and startup calls it once to come back in the persisted state.

use std::sync::{Arc, Mutex};

use anyhow::{Result, bail};
use serde::Serialize;
use tokio::sync::{Mutex as AsyncMutex, watch};
use tokio::task::JoinHandle;

use crate::config::{Cli, Settings};
use crate::state::State;

/// What `seltun status` shows.
#[derive(Serialize, Debug, Clone)]
pub struct Status {
    pub pid: u32,
    /// What the user asked for (the persisted switch).
    pub enabled: bool,
    /// Whether the TUN adapter and forwarding are up right now.
    pub active: bool,
    /// Why the daemon is not in the state asked for, if it is not.
    pub error: Option<String>,
    pub processes: Vec<String>,
    pub upstream: String,
    pub elevated: bool,
}

struct Active {
    settings: Settings,
    stop: watch::Sender<bool>,
    task: JoinHandle<()>,
    /// Set by the task if the data path dies on its own.
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

    /// Re-read the state and the config and make reality match; the outcome is in the returned status.
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
                // Keep what is running; a typo in the config must not drop the user's traffic.
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

    /// Tear the data path down (the daemon is exiting).
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
                tracing::warn!("the previous data path had failed: {error}");
            }
            active.stop().await;
            tracing::info!("steering stopped");
        }
        if enabled && inner.active.is_none() {
            if settings.processes.is_empty() {
                bail!("on, but [processes].names is empty: nothing to steer");
            }
            inner.active = Some(start(settings).await?);
            tracing::info!("steering started");
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

    /// A fixed adapter GUID, so Windows keeps one network profile for it instead of a new one per start.
    const ADAPTER_GUID: u128 = 0x5e17_0a11_c0de_4f1d_9a3b_2c64_7e01_5e17;

    if !is_elevated() {
        bail!(
            "creating the TUN adapter needs an elevated process; run `seltun autostart enable` from an elevated terminal once, then `seltun start`"
        );
    }
    let wintun = std::env::current_exe()?.with_file_name("wintun.dll");
    if !wintun.is_file() {
        bail!(
            "{} is missing; run `seltun setup` to download it",
            wintun.display()
        );
    }
    let mut config = tun::Configuration::default();
    config
        .tun_name(&settings.tun.name)
        .address(settings.tun.address)
        // A /30: the adapter's own address and a nominal peer; the half-default routes are what send traffic in.
        .netmask(Ipv4Addr::new(255, 255, 255, 252))
        .mtu(settings.tun.mtu)
        .up();
    config.platform_config(|platform| {
        platform.device_guid(ADAPTER_GUID);
        platform.wintun_file(wintun.as_os_str());
    });
    let device = tun::create_as_async(&config).context("creating the Wintun adapter")?;
    let index = device.tun_index().context("reading the adapter index")? as u32;
    let routes = Routes::install(index).context("adding the TUN routes")?;

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
            tracing::error!("data path failed: {error:#}");
            *report.lock().unwrap_or_else(|e| e.into_inner()) = Some(format!("{error:#}"));
        }
        // The adapter is gone with the stack; the routes go with it, explicitly too.
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
    bail!("seltund steers traffic on Windows only")
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
