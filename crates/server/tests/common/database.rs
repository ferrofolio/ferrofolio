// SPDX-License-Identifier: AGPL-3.0-only

//! The harness for routes that read the database.
//!
//! `tests/parity/generate_db_route_fixtures.py` creates the fixture databases with
//! the Python backend and records the routes against them. Here each database is
//! copied into a data directory of its own, as the backend lays one out, and Rust
//! unlocks it through a login as a client sends it, with a stand-in for Python
//! that accepts the login, reports the lock state and the unlocked user on
//! `GET /api/v1/status` and answers every other request it gets with a mark that
//! it was passed on.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};

use axum::body::Body;
use axum::extract::Request;
use axum::http::header::AUTHORIZATION;
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::IntoResponse;
use axum::routing::{get, post};
use serde_json::{Value, json};
use server::app::{AppState, router};
use server::proxy::Backend;
use server::sessions::{SessionSettings, Sessions, SystemSessionClock};
use server::user_data::DataDir;

/// Where the generator writes the databases and the recorded routes.
pub const FIXTURES: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/db_routes");
/// The user of the fixture databases (see the generator).
pub const USERNAME: &str = "parity";
/// The password of the fixture databases, not a secret.
pub const PASSWORD: &str = "parity pass'word ü";
/// The token of a session no one has, as after a logout.
pub const UNKNOWN_TOKEN: &str = "parity-unknown-session-token";
/// The token the stand-in for Python issues at a login.
pub const BACKEND_TOKEN: &str = "parity-backend-token";
const PROFILE_ID: &str = "0d3e7c1a-5b2f-4c8e-9a1b-2c3d4e5f6a7b";

/// A fixture file read as JSON.
pub fn fixture(name: &str) -> Value {
    let text = std::fs::read_to_string(Path::new(FIXTURES).join(name)).unwrap();
    serde_json::from_str(&text).unwrap()
}

/// A temporary directory, removed with everything in it when dropped.
#[derive(Debug)]
pub struct TempDirectory(PathBuf);

impl TempDirectory {
    /// Creates a new, empty directory that belongs to this test alone: a name
    /// already taken, by a run that stopped before it cleaned up for example, is
    /// skipped rather than reused.
    pub fn create(prefix: &str) -> Self {
        static NEXT: AtomicU32 = AtomicU32::new(0);
        loop {
            let directory: PathBuf = std::env::temp_dir().join(format!(
                "{prefix}-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            match std::fs::create_dir(&directory) {
                Ok(()) => return Self(directory),
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
                Err(error) => panic!("cannot create {}: {error}", directory.display()),
            }
        }
    }
}

impl Drop for TempDirectory {
    fn drop(&mut self) {
        // A file that is still open goes when it is closed; a failure leaves
        // only a stray directory in the temporary directory.
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// A data directory with the fixture database `file` as the database of
/// `USERNAME`, in a directory of its own: opening a database adds WAL files.
/// The directory goes when the returned guard is dropped.
pub fn data_dir(file: &str) -> (DataDir, TempDirectory) {
    let guard = TempDirectory::create("ferrofolio-db-parity");
    let directory = guard.0.clone();
    let profile = directory.join("profiles").join(PROFILE_ID);
    std::fs::create_dir_all(&profile).unwrap();
    std::fs::copy(Path::new(FIXTURES).join(file), profile.join("data.db")).unwrap();
    let profiles = json!({"profiles": [{"id": PROFILE_ID, "name": USERNAME}]});
    std::fs::write(directory.join("profiles.json"), profiles.to_string()).unwrap();
    (DataDir::new(directory), guard)
}

/// The stand-in for Python: whether its data is unlocked.
#[derive(Debug, Clone, Default)]
pub struct Python {
    unlocked: Arc<AtomicBool>,
}

impl Python {
    pub fn set_unlocked(&self, unlocked: bool) {
        self.unlocked.store(unlocked, Ordering::SeqCst);
    }

    /// Serves the stand-in and returns its address. With API sessions a login
    /// answers with `BACKEND_TOKEN` and the status counts only that token; without
    /// them a login answers `204` and the status counts every request.
    async fn serve(&self, sessions: bool) -> String {
        let login = {
            let python = self.clone();
            move || async move {
                python.set_unlocked(true);
                if sessions {
                    axum::Json(json!({ "token": BACKEND_TOKEN })).into_response()
                } else {
                    StatusCode::NO_CONTENT.into_response()
                }
            }
        };
        let logout = {
            let python = self.clone();
            move || async move {
                python.set_unlocked(false);
                StatusCode::NO_CONTENT
            }
        };
        let status = {
            let python = self.clone();
            move |headers: HeaderMap| async move {
                let authorized = !sessions
                    || headers.get(AUTHORIZATION).is_some_and(|value| {
                        value.as_bytes() == format!("Bearer {BACKEND_TOKEN}").as_bytes()
                    });
                let unlocked = python.unlocked.load(Ordering::SeqCst) && authorized;
                // Python names the user whose data it unlocked, which Rust checks
                // without API sessions before it opens the database.
                if unlocked {
                    axum::Json(json!({ "status": "UNLOCKED", "user": { "username": USERNAME } }))
                } else {
                    axum::Json(json!({ "status": "LOCKED" }))
                }
            }
        };
        let app = axum::Router::new()
            .route("/api/v1/login", post(login))
            .route("/api/v1/logout", post(logout))
            .route("/api/v1/status", get(status))
            .fallback(|request: Request| async move { axum::Json(forwarded(&request)) });
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        url
    }
}

/// What the stand-in answers to a request Rust passed on: the request as it
/// arrived, with the `Authorization` it carried.
pub fn forwarded<B>(request: &axum::http::Request<B>) -> Value {
    json!({
        "forwarded": format!("{} {}", request.method(), request.uri()),
        "authorization": request
            .headers()
            .get(AUTHORIZATION)
            .and_then(|value| value.to_str().ok()),
    })
}

/// The router with a fixture database, Python's stand-in and the state both share.
pub struct Harness {
    pub app: axum::Router,
    pub state: AppState,
    pub python: Python,
    /// Declared last, so that it goes after the state with the open database.
    _directory: TempDirectory,
}

impl Harness {
    /// The server for the fixture database `file`, with or without API sessions,
    /// before any login; `configure` adjusts the state first.
    pub async fn start(file: &str, sessions: bool, configure: impl FnOnce(&mut AppState)) -> Self {
        let python = Python::default();
        let url = python.serve(sessions).await;
        let settings = SessionSettings {
            enabled: sessions,
            remember_ttl: None,
        };
        let mut state = AppState::new(
            Backend::new(&url).unwrap(),
            Arc::new(server::clock::SystemClock),
            Sessions::new(settings, Arc::new(SystemSessionClock)),
        );
        let (data_dir, directory) = data_dir(file);
        state.data_dir = Some(data_dir);
        configure(&mut state);
        Harness {
            app: router(state.clone()),
            state,
            python,
            _directory: directory,
        }
    }

    /// Adds a second user, `username`, with a copy of the fixture database `file`
    /// and the same password.
    pub fn add_user(&self, username: &str, file: &str) {
        let directory = &self._directory.0;
        // Python names profiles with a UUID, and Rust reads only those.
        let id = "1d3e7c1a-5b2f-4c8e-9a1b-2c3d4e5f6a7c";
        let profile = directory.join("profiles").join(id);
        std::fs::create_dir_all(&profile).unwrap();
        std::fs::copy(Path::new(FIXTURES).join(file), profile.join("data.db")).unwrap();
        let profiles = json!({"profiles": [
            {"id": PROFILE_ID, "name": USERNAME},
            {"id": id, "name": username},
        ]});
        std::fs::write(directory.join("profiles.json"), profiles.to_string()).unwrap();
    }

    /// Logs in through the router, as a client does, and returns the client's
    /// token, if API sessions issue one.
    pub async fn login(&self) -> Option<String> {
        self.login_as(USERNAME).await
    }

    /// Logs in as `username` with the fixture password, as `login` does.
    pub async fn login_as(&self, username: &str) -> Option<String> {
        let body = json!({ "username": username, "password": PASSWORD }).to_string();
        let request = Request::post("/api/v1/login")
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(body))
            .unwrap();
        let (status, body) = super::answer(&self.app, request).await;
        assert!(status.is_success(), "login failed with {status}");
        body.and_then(|body| body["token"].as_str().map(str::to_owned))
    }

    /// Reads the open database with `read` on the blocking pool, as routes do.
    pub async fn read<T: Send + 'static>(
        &self,
        read: impl FnOnce(&infra_db::Database) -> T + Send + 'static,
    ) -> Result<T, server::user_data::UnlockError> {
        let sessions = self.state.sessions.clone();
        tokio::task::spawn_blocking(move || sessions.with_database(read))
            .await
            .unwrap()
    }
}
