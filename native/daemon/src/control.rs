//! The control channel. It uses a loopback TCP port. The client sends one JSON request in one line, and the daemon sends one JSON answer in one line.
//!
//! A request has no data, only a verb: `ping`, `status`, `reload` or `shutdown`. The verb `reload` makes the daemon read the files that the CLI wrote again. Thus the channel gives the elevated daemon only one instruction: read the files of the user again.

use std::sync::Arc;

use anyhow::Result;
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::Notify;

use crate::daemon::{Daemon, Status};

#[derive(Deserialize)]
struct Request {
    cmd: String,
}

#[derive(Serialize)]
struct Response {
    ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    status: Option<Status>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
}

async fn answer(line: &str, daemon: &Daemon, quit: &Notify) -> Response {
    let status = match serde_json::from_str::<Request>(line).map(|r| r.cmd) {
        Ok(cmd) => match cmd.as_str() {
            "ping" | "status" => daemon.status().await,
            "reload" => daemon.reload().await,
            "shutdown" => {
                quit.notify_one();
                daemon.status().await
            },
            other => {
                return Response {
                    ok: false,
                    status: None,
                    error: Some(format!("The command {other:?} is not known.")),
                };
            },
        },
        Err(error) => {
            return Response {
                ok: false,
                status: None,
                error: Some(format!("The request is not correct: {error}")),
            };
        },
    };
    Response {
        ok: true,
        status: Some(status),
        error: None,
    }
}

async fn serve_connection(stream: TcpStream, daemon: Arc<Daemon>, quit: Arc<Notify>) -> Result<()> {
    let (read, mut write) = stream.into_split();
    let mut lines = BufReader::new(read).lines();
    while let Some(line) = lines.next_line().await? {
        let mut out = serde_json::to_vec(&answer(&line, &daemon, &quit).await)?;
        out.push(b'\n');
        write.write_all(&out).await?;
    }
    Ok(())
}

/// Serve the control channel on `listener`. The service stops when the task stops.
pub async fn serve(listener: TcpListener, daemon: Arc<Daemon>, quit: Arc<Notify>) {
    loop {
        match listener.accept().await {
            Ok((stream, _)) => {
                let (daemon, quit) = (daemon.clone(), quit.clone());
                tokio::spawn(async move {
                    if let Err(error) = serve_connection(stream, daemon, quit).await {
                        tracing::debug!("Control connection failed: {error}");
                    }
                });
            },
            Err(error) => tracing::warn!("Control accept failed: {error}"),
        }
    }
}
