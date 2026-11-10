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
        // Colors only on a terminal; otherwise stderr is a pipe or file.
        None => builder
            .with_writer(std::io::stderr)
            .with_ansi(std::io::stderr().is_terminal())
            .init(),
    }
    Ok(())
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    init_logging(&cli)?;

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
    // Come back in the state the user left: the persisted on/off switch.
    daemon.reload().await;

    tokio::select! {
        _ = control::serve(listener, daemon.clone(), quit.clone()) => {},
        _ = quit.notified() => tracing::info!("The CLI requested a shutdown."),
        _ = tokio::signal::ctrl_c() => tracing::info!("interrupted"),
    }
    daemon.stop().await;
    Ok(())
}
