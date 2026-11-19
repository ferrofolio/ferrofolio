// SPDX-License-Identifier: AGPL-3.0-only

//! The Ferrofolio server.
//!
//! It is the entry point of the container: it serves the routes that have been
//! ported to Rust and forwards all others to the Python backend, which it runs as a
//! child process on the loopback interface (docs/roadmap.md, section 5).

pub mod app;
pub mod auth;
pub mod backend;
pub mod clock;
pub mod config;
pub mod error;
pub mod gateway;
pub mod proxy;
pub mod pycompat;
pub mod pypath;
pub mod python_printable;
pub mod routes;
pub mod sessions;
pub mod user_data;

use std::net::SocketAddr;
use std::process::ExitCode;
use std::sync::Arc;
use std::time::Duration;

use tokio::net::TcpSocket;
use tokio::signal::unix::{Signal, SignalKind, signal};
use tokio::sync::oneshot;
use tracing_subscriber::EnvFilter;

use crate::app::{AppState, router};
use crate::backend::BackendProcess;
use crate::clock::SystemClock;
use crate::config::{Cli, LogLevel};
use crate::proxy::Backend;
use crate::routes::status::{ServerOptions, StatusConfig};
use crate::sessions::{Credentials, SessionSettings, Sessions, SystemSessionClock};
use crate::user_data::{DataDir, default_data_dir};

/// Time the server gets to finish open requests on shutdown. Together with
/// `BACKEND_SHUTDOWN_GRACE` and `RUNTIME_SHUTDOWN_GRACE` it stays below Docker's
/// default stop timeout of ten seconds, after which the container is killed.
const SERVER_SHUTDOWN_GRACE: Duration = Duration::from_secs(3);
/// Time the backend gets to stop before it is killed. Its own graceful shutdown
/// takes up to five seconds.
const BACKEND_SHUTDOWN_GRACE: Duration = Duration::from_secs(6);
/// Time calculations still running on blocking threads get once the server has
/// stopped; the process exits after it, within Docker's stop timeout.
pub const RUNTIME_SHUTDOWN_GRACE: Duration = Duration::from_millis(500);

/// Configures console logging from the backend's log level options. Fails when a
/// global subscriber is already installed.
pub fn init_logging(cli: &Cli) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let own = cli.log_level.as_filter();
    let third_party = cli.third_party_log_level.as_filter();
    // NONE switches the console off entirely, third-party messages included.
    let directives = if cli.log_level == LogLevel::None {
        "off".to_owned()
    } else {
        format!("{third_party},server={own},ferrofolio={own},core_app={own},core_domain={own}")
    };
    let filter = EnvFilter::try_new(directives).unwrap_or_else(|_| EnvFilter::new("info"));
    // Colours only on a terminal; container logs stay plain text.
    let ansi = std::io::IsTerminal::is_terminal(&std::io::stdout());
    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_ansi(ansi)
        .try_init()
}

/// SIGTERM (as sent by `docker stop`) and SIGINT, registered once at startup so that
/// no signal is lost between the phases of the server's life.
#[derive(Debug)]
struct ShutdownSignals {
    terminate: Signal,
    interrupt: Signal,
}

impl ShutdownSignals {
    fn register() -> std::io::Result<Self> {
        Ok(ShutdownSignals {
            terminate: signal(SignalKind::terminate())?,
            interrupt: signal(SignalKind::interrupt())?,
        })
    }

    async fn received(&mut self) {
        tokio::select! {
            _ = self.terminate.recv() => {}
            _ = self.interrupt.recv() => {}
        }
    }
}

/// Binds the public address without listening on it yet. A host name can resolve to
/// several addresses; the first one that can be bound is taken.
async fn reserve_public_port(host: &str, port: u16) -> std::io::Result<TcpSocket> {
    let mut last_error =
        std::io::Error::new(std::io::ErrorKind::NotFound, "no address for the host");
    for address in tokio::net::lookup_host((host, port)).await? {
        match bind_without_listening(address) {
            Ok(socket) => return Ok(socket),
            Err(error) => last_error = error,
        }
    }
    Err(last_error)
}

fn bind_without_listening(address: SocketAddr) -> std::io::Result<TcpSocket> {
    let socket = if address.is_ipv4() {
        TcpSocket::new_v4()?
    } else {
        TcpSocket::new_v6()?
    };
    // Like a standard listener, so that a restart can reuse the port at once.
    socket.set_reuseaddr(true)?;
    socket.bind(address)?;
    Ok(socket)
}

/// The credentials of Python's automatic login at startup (`finanze/server.py`,
/// `_init_user`): `USERNAME` and `PASSWORD`, both set and not empty. Python has
/// logged in with them before it answers, or failed to start.
fn startup_credentials() -> Option<Credentials> {
    let variable = |name| {
        std::env::var(name)
            .ok()
            .filter(|value: &String| !value.is_empty())
    };
    Some(Credentials {
        username: variable("USERNAME")?,
        password: variable("PASSWORD")?,
    })
}

/// Runs the server until it receives SIGTERM or SIGINT, or until the backend exits.
pub async fn run(mut cli: Cli) -> ExitCode {
    // Python's default, resolved here and passed on, so that both use the same.
    let data_dir = cli.data_dir.get_or_insert_with(default_data_dir).clone();
    let mut signals = match ShutdownSignals::register() {
        Ok(signals) => signals,
        Err(error) => {
            tracing::error!(%error, "could not register the shutdown signals");
            return ExitCode::FAILURE;
        }
    };
    // Both would bind with SO_REUSEADDR, so the same port is rejected explicitly.
    if cli.backend_port == cli.port {
        tracing::error!(
            port = cli.port,
            "the backend port must differ from the server port"
        );
        return ExitCode::FAILURE;
    }
    // The public port is reserved first, so that the backend's port can never take
    // it and a conflict shows before Python starts. It only accepts connections once
    // the backend is ready, so until then clients are refused, as before.
    // Resolving the host can take a while; a stop request meanwhile ends the server
    // before Python starts.
    let reserved = tokio::select! {
        reserved = reserve_public_port(&cli.host, cli.port) => reserved,
        () = signals.received() => return ExitCode::SUCCESS,
    };
    let public_socket = match reserved {
        Ok(socket) => socket,
        Err(error) => {
            tracing::error!(%error, host = %cli.host, port = cli.port, "could not open the server port");
            return ExitCode::FAILURE;
        }
    };
    let mut process = match BackendProcess::spawn(&cli) {
        Ok(process) => process,
        Err(error) => {
            tracing::error!(%error, "could not start the backend");
            return ExitCode::FAILURE;
        }
    };
    let backend = match Backend::new(&process.url()) {
        Ok(backend) => backend,
        Err(error) => {
            tracing::error!(%error, "invalid backend address");
            process.terminate(BACKEND_SHUTDOWN_GRACE).await;
            return ExitCode::FAILURE;
        }
    };

    tracing::info!("waiting for the Python backend to start");
    let ready = tokio::select! {
        ready = process.wait_until_ready() => Some(ready),
        () = signals.received() => None,
    };
    match ready {
        Some(Ok(())) => {}
        Some(Err(error)) => {
            tracing::error!(%error, "the backend did not start");
            process.terminate(BACKEND_SHUTDOWN_GRACE).await;
            return ExitCode::FAILURE;
        }
        None => {
            process.terminate(BACKEND_SHUTDOWN_GRACE).await;
            return ExitCode::SUCCESS;
        }
    }

    let listener = match public_socket.listen(1024) {
        Ok(listener) => listener,
        Err(error) => {
            tracing::error!(%error, host = %cli.host, port = cli.port, "could not open the server port");
            process.terminate(BACKEND_SHUTDOWN_GRACE).await;
            return ExitCode::FAILURE;
        }
    };
    let sessions = Sessions::new(SessionSettings::from_env(), Arc::new(SystemSessionClock));
    let mut state = AppState::new(backend, Arc::new(SystemClock), sessions);
    state.data_dir = Some(DataDir::new(data_dir.clone()));
    state.status = StatusConfig::from_env(ServerOptions::from_cli(&cli, data_dir)).map(Arc::new);
    if state.status.is_none() {
        tracing::info!(
            "the backend answers the status: it loads the feature flags from FEATURE_FLAG_URL"
        );
    }
    if let Some(credentials) = startup_credentials() {
        gateway::unlock_at_startup(&state, credentials).await;
    }
    let app = router(state).into_make_service_with_connect_info::<SocketAddr>();

    let (stop_server, server_stopped) = oneshot::channel::<()>();
    let mut server = tokio::spawn(async move {
        axum::serve(listener, app)
            .with_graceful_shutdown(async {
                let _ = server_stopped.await;
            })
            .await
    });
    tracing::info!(host = %cli.host, port = cli.port, "Ferrofolio server listening");

    let (exit_code, server_finished) = tokio::select! {
        () = signals.received() => {
            tracing::info!("shutting down");
            (ExitCode::SUCCESS, false)
        }
        status = process.wait() => {
            tracing::error!(?status, "the Python backend exited");
            (ExitCode::FAILURE, false)
        }
        result = &mut server => {
            tracing::error!(?result, "the server stopped");
            (ExitCode::FAILURE, true)
        }
    };

    let _ = stop_server.send(());
    // A finished task must not be polled again.
    if !server_finished
        && tokio::time::timeout(SERVER_SHUTDOWN_GRACE, &mut server)
            .await
            .is_err()
    {
        server.abort();
    }
    process.terminate(BACKEND_SHUTDOWN_GRACE).await;
    exit_code
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn a_busy_first_address_falls_back_to_the_next_one() {
        let port = {
            let probe = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
            probe.local_addr().unwrap().port()
        };
        let addresses: Vec<SocketAddr> = tokio::net::lookup_host(("localhost", port))
            .await
            .unwrap()
            .collect();
        // Needs a host name with two addresses; localhost usually has ::1 and 127.0.0.1.
        let [first, second, ..] = addresses[..] else {
            eprintln!("skipped: localhost has a single address here");
            return;
        };
        let Ok(_busy) = std::net::TcpListener::bind(first) else {
            eprintln!("skipped: {first} cannot be bound here");
            return;
        };
        let socket = reserve_public_port("localhost", port).await.unwrap();
        assert_eq!(socket.local_addr().unwrap(), second);
    }
}
