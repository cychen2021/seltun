// The daemon has no console window of its own. Task Scheduler then does not open a window, and no window can stop the daemon when a user closes it.
#![cfg_attr(windows, windows_subsystem = "windows")]

use std::io::IsTerminal;
use std::sync::{Arc, Mutex};

use anyhow::{Context, Result};
use clap::Parser;
use tokio::net::TcpListener;
use tokio::sync::Notify;

use seltund::config::{Cli, Settings};
use seltund::control;
use seltund::daemon::Daemon;

fn init_logging(cli: &Cli) -> Result<()> {
    let filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| "seltund=info,warn".into());
    let builder = tracing_subscriber::fmt().with_env_filter(filter);
    match &cli.log_file {
        Some(path) => {
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent)?;
            }
            let file = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(path)
                .with_context(|| format!("Cannot open {}", path.display()))?;
            builder
                .with_ansi(false)
                .with_writer(Mutex::new(file))
                .init();
        },
        // Use colors only in a terminal. Otherwise stderr is a pipe or a file.
        None => builder
            .with_writer(std::io::stderr)
            .with_ansi(std::io::stderr().is_terminal())
            .init(),
    }
    Ok(())
}

#[tokio::main]
async fn main() -> Result<()> {
    // A foreground run writes to the terminal of the parent. A run with a log file does not use a terminal.
    if !std::env::args().any(|a| a == "--log-file" || a.starts_with("--log-file=")) {
        seltund::platform::attach_parent_console();
    }
    let cli = Cli::parse();
    init_logging(&cli)?;
    if let Some(limit) = seltund::platform::raise_open_file_limit() {
        tracing::info!(limit, "The limit of open files.");
    }

    let settings = Settings::load(&cli.config)?;
    let listener = TcpListener::bind(settings.control).await.with_context(|| {
        format!(
            "Cannot bind the control port {}. Is seltund already running?",
            settings.control
        )
    })?;
    tracing::info!(control = %settings.control, "seltund started.");

    let daemon = Arc::new(Daemon::new(cli));
    let quit = Arc::new(Notify::new());
    // Start in the state that the user selected: the saved on/off switch.
    daemon.reload().await;

    tokio::select! {
        _ = control::serve(listener, daemon.clone(), quit.clone()) => {},
        _ = quit.notified() => tracing::info!("The CLI requested a shutdown."),
        _ = tokio::signal::ctrl_c() => tracing::info!("interrupted"),
    }
    daemon.stop().await;
    Ok(())
}
