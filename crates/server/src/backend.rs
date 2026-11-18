// SPDX-License-Identifier: AGPL-3.0-only

//! Starting and stopping the Python backend as a child process.
//!
//! The gateway binds the backend's loopback socket itself and hands it to the
//! child, so no other process can take that port. The backend counts as ready once
//! it has answered an HTTP request on that socket; only then does the gateway open
//! its public port, so clients see the server come up exactly when it can answer.

use std::ffi::OsString;
use std::os::fd::AsRawFd;
use std::process::ExitStatus;
use std::time::Duration;

use nix::fcntl::{FcntlArg, FdFlag, fcntl};
use nix::sys::signal::{Signal, kill};
use nix::unistd::Pid;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::process::{Child, Command};

use crate::config::Cli;

const LOOPBACK: &str = "127.0.0.1";
const PROGRESS_INTERVAL: Duration = Duration::from_secs(30);
const PROBE_ATTEMPT_TIMEOUT: Duration = Duration::from_secs(30);
const MAX_RESPONSE_HEAD: usize = 64 * 1024;
const PROBE: &[u8] = b"GET /api/v1/status HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n";

/// The running backend process.
#[derive(Debug)]
pub struct BackendProcess {
    child: Child,
    port: u16,
}

/// Starting or waiting for the backend failed.
#[derive(Debug, thiserror::Error)]
pub enum BackendStartError {
    #[error("could not open the backend port {port}: {source}")]
    Bind { port: u16, source: std::io::Error },
    #[error("could not hand the backend socket to the backend: {0}")]
    Inherit(nix::Error),
    #[error("could not start the backend with {program:?}: {source}")]
    Spawn {
        program: String,
        source: std::io::Error,
    },
    #[error("the backend exited during startup with {0}")]
    Exited(ExitStatus),
    #[error("could not watch the backend process: {0}")]
    Wait(std::io::Error),
}

impl BackendProcess {
    /// Starts the backend with the options of the gateway.
    pub fn spawn(cli: &Cli) -> Result<Self, BackendStartError> {
        let bind_error = |source| BackendStartError::Bind {
            port: cli.backend_port,
            source,
        };
        let listener =
            std::net::TcpListener::bind((LOOPBACK, cli.backend_port)).map_err(bind_error)?;
        let port = listener.local_addr().map_err(bind_error)?.port();
        // The child inherits the socket; Rust opens it with close-on-exec.
        fcntl(&listener, FcntlArg::F_SETFD(FdFlag::empty())).map_err(BackendStartError::Inherit)?;

        let mut command = Command::new(&cli.python);
        command
            .arg(&cli.backend_app)
            .args(backend_args(cli, port, listener.as_raw_fd()))
            // Its own process group keeps a Ctrl+C in a terminal from reaching it
            // directly: the server drains open requests first and then stops it.
            .process_group(0)
            .kill_on_drop(true);
        let child = command.spawn().map_err(|source| BackendStartError::Spawn {
            program: cli.python.clone(),
            source,
        })?;
        // Only the child serves the socket from now on.
        drop(listener);
        tracing::info!(port, "started the Python backend");
        Ok(BackendProcess { child, port })
    }

    /// The base URL of the backend.
    pub fn url(&self) -> String {
        format!("http://{LOOPBACK}:{}", self.port)
    }

    /// Waits until the backend answers HTTP on its socket.
    ///
    /// The probe connection waits in the socket's backlog until the backend starts
    /// accepting, and only the child holds that socket, so an answer proves that the
    /// started backend is serving. There is no deadline: a first start can run long
    /// database migrations. While it waits, it reports progress every 30 seconds.
    pub async fn wait_until_ready(&mut self) -> Result<(), BackendStartError> {
        let started = tokio::time::Instant::now();
        let mut progress = tokio::time::interval_at(started + PROGRESS_INTERVAL, PROGRESS_INTERVAL);
        let probe = probe_until_answered(self.port);
        tokio::pin!(probe);
        loop {
            tokio::select! {
                status = self.child.wait() => {
                    return Err(match status {
                        Ok(status) => BackendStartError::Exited(status),
                        Err(error) => BackendStartError::Wait(error),
                    });
                }
                () = &mut probe => return Ok(()),
                _ = progress.tick() => {
                    tracing::info!(
                        seconds = started.elapsed().as_secs(),
                        "still waiting for the Python backend to start"
                    );
                }
            }
        }
    }

    /// Waits until the backend exits on its own.
    pub async fn wait(&mut self) -> std::io::Result<ExitStatus> {
        self.child.wait().await
    }

    /// Asks the backend to shut down and kills it after `grace`.
    pub async fn terminate(mut self, grace: Duration) {
        if let Ok(Some(_)) = self.child.try_wait() {
            return;
        }
        let pid = self
            .child
            .id()
            .and_then(|pid| i32::try_from(pid).ok())
            .map(Pid::from_raw);
        if let Some(pid) = pid
            && kill(pid, Signal::SIGTERM).is_ok()
            && let Ok(Ok(status)) = tokio::time::timeout(grace, self.child.wait()).await
        {
            tracing::info!(%status, "the Python backend stopped");
            return;
        }
        tracing::warn!("the Python backend did not stop in time, killing it");
        if let Err(error) = self.child.kill().await {
            tracing::error!(%error, "could not kill the Python backend");
        }
    }
}

/// Sends HTTP requests until one gets a complete response head. Each attempt has a
/// deadline, so a connection the backend never answers does not stall startup.
async fn probe_until_answered(port: u16) {
    loop {
        if let Ok(Ok(true)) = tokio::time::timeout(PROBE_ATTEMPT_TIMEOUT, probe_once(port)).await {
            return;
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
}

/// Whether the backend answered with a well-formed HTTP response head.
async fn probe_once(port: u16) -> std::io::Result<bool> {
    let mut stream = TcpStream::connect((LOOPBACK, port)).await?;
    stream.write_all(PROBE).await?;
    let mut head = Vec::with_capacity(512);
    let mut chunk = [0u8; 512];
    while !head.windows(4).any(|window| window == b"\r\n\r\n") {
        let read = stream.read(&mut chunk).await?;
        if read == 0 || head.len() > MAX_RESPONSE_HEAD {
            return Ok(false);
        }
        head.extend_from_slice(chunk.get(..read).unwrap_or_default());
    }
    Ok(is_status_line(&head))
}

/// `HTTP/1.x NNN` at the start of a response.
fn is_status_line(head: &[u8]) -> bool {
    let Some(line) = head.split(|byte| *byte == b'\r').next() else {
        return false;
    };
    let mut parts = line.splitn(3, |byte| *byte == b' ');
    let version = parts.next().unwrap_or_default();
    let status = parts.next().unwrap_or_default();
    (version == b"HTTP/1.1" || version == b"HTTP/1.0")
        && status.len() == 3
        && status.iter().all(u8::is_ascii_digit)
}

/// Arguments for `finanze/args.py`. The backend serves the inherited socket, keeps
/// `--host` and `--port` for its messages, and learns the public port for the
/// redirect URLs it builds.
fn backend_args(cli: &Cli, port: u16, socket_fd: i32) -> Vec<OsString> {
    let mut args: Vec<OsString> = vec![
        "--host".into(),
        LOOPBACK.into(),
        "--port".into(),
        port.to_string().into(),
        "--socket-fd".into(),
        socket_fd.to_string().into(),
        "--public-port".into(),
        cli.port.to_string().into(),
        "--log-level".into(),
        cli.log_level.as_arg().into(),
        "--third-party-log-level".into(),
        cli.third_party_log_level.as_arg().into(),
    ];
    if let Some(data_dir) = &cli.data_dir {
        args.extend(["--data-dir".into(), data_dir.into()]);
    }
    if let Some(log_dir) = &cli.log_dir {
        args.extend(["--log-dir".into(), log_dir.into()]);
    }
    if let Some(level) = cli.log_file_level {
        args.extend(["--log-file-level".into(), level.as_arg().into()]);
    }
    if let Some(path) = &cli.llama_server_path {
        args.extend(["--llama-server-path".into(), path.into()]);
    }
    args
}

#[cfg(test)]
mod tests {
    use clap::Parser;

    use super::*;

    #[test]
    fn passes_the_options_on_to_the_backend() {
        let cli = Cli::try_parse_from([
            "ferrofolio",
            "--host",
            "0.0.0.0",
            "--port",
            "7592",
            "--data-dir",
            "/.storage",
            "--log-dir",
            "/.storage/logs",
            "--log-level",
            "DEBUG",
        ])
        .unwrap();
        let args: Vec<String> = backend_args(&cli, 40000, 7)
            .into_iter()
            .map(|arg| arg.into_string().unwrap())
            .collect();
        assert_eq!(
            args,
            [
                "--host",
                "127.0.0.1",
                "--port",
                "40000",
                "--socket-fd",
                "7",
                "--public-port",
                "7592",
                "--log-level",
                "DEBUG",
                "--third-party-log-level",
                "ERROR",
                "--data-dir",
                "/.storage",
                "--log-dir",
                "/.storage/logs",
            ]
        );
    }

    fn cli(python: &str, app: &std::path::Path, backend_port: u16) -> Cli {
        Cli::try_parse_from([
            "ferrofolio",
            "--python",
            python,
            "--backend-app",
            app.to_str().unwrap(),
            "--backend-port",
            &backend_port.to_string(),
        ])
        .unwrap()
    }

    #[test]
    fn refuses_a_backend_port_that_is_in_use() {
        let taken = std::net::TcpListener::bind((LOOPBACK, 0)).unwrap();
        let port = taken.local_addr().unwrap().port();
        let cli = cli("/nonexistent/python", std::path::Path::new("app"), port);
        assert!(matches!(
            BackendProcess::spawn(&cli),
            Err(BackendStartError::Bind { .. })
        ));
    }

    #[tokio::test]
    async fn a_backend_that_exits_during_startup_is_reported() {
        let mut process =
            BackendProcess::spawn(&cli("false", std::path::Path::new("app"), 0)).unwrap();
        assert!(matches!(
            process.wait_until_ready().await,
            Err(BackendStartError::Exited(_))
        ));
    }

    /// Answers one request on the inherited socket, then idles.
    const FAKE_BACKEND: &str = r#"
import socket, sys, time
fd = int(sys.argv[sys.argv.index("--socket-fd") + 1])
time.sleep(0.5)
server = socket.socket(fileno=fd)
connection, _ = server.accept()
connection.recv(4096)
connection.sendall(b"HTTP/1.1 204 No Content\r\nConnection: close\r\n\r\n")
connection.close()
time.sleep(60)
"#;

    /// A new directory only this user can access; creation fails if it exists.
    fn private_temp_dir() -> std::path::PathBuf {
        use std::os::unix::fs::DirBuilderExt;
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let directory =
            std::env::temp_dir().join(format!("ferrofolio-backend-{}-{nanos}", std::process::id()));
        std::fs::DirBuilder::new()
            .mode(0o700)
            .create(&directory)
            .unwrap();
        directory
    }

    #[test]
    fn requires_a_complete_status_line() {
        assert!(is_status_line(b"HTTP/1.1 204 No Content\r\n\r\n"));
        assert!(is_status_line(b"HTTP/1.0 500\r\n\r\n"));
        assert!(!is_status_line(b"HTTP/\r\n\r\n"));
        assert!(!is_status_line(b"HTTP/1.1 2x4 Oops\r\n\r\n"));
        assert!(!is_status_line(b"SSH-2.0-OpenSSH\r\n\r\n"));
    }

    #[tokio::test]
    async fn the_started_backend_serves_the_inherited_socket() {
        let directory = private_temp_dir();
        let app = directory.join("fake_backend.py");
        std::fs::write(&app, FAKE_BACKEND).unwrap();

        let mut process = BackendProcess::spawn(&cli("python3", &app, 0)).unwrap();
        tokio::time::timeout(Duration::from_secs(20), process.wait_until_ready())
            .await
            .expect("the backend did not answer")
            .unwrap();
        process.terminate(Duration::from_secs(5)).await;
        std::fs::remove_dir_all(&directory).unwrap();
    }
}
