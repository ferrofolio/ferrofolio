// SPDX-License-Identifier: AGPL-3.0-only

//! The gateway against a fake Python backend: forwarding, sessions, the
//! authorisation of ported routes and the request handling they get.

// Test helpers outside #[test] functions may panic on broken fixtures.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use std::collections::HashSet;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime};

use axum::Router;
use axum::body::{Body, Bytes, to_bytes};
use axum::extract::{ConnectInfo, Request, State};
use axum::http::{HeaderMap, Method, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::any;
use core_app::ports::FixedClock;
use core_domain::Dezimal;
use core_domain::dates::Date;
use serde_json::{Value, json};
use server::app::{AppState, router};
use server::proxy::Backend;
use server::sessions::{Credentials, SessionClock, SessionSettings, Sessions};
use server::user_data::{DataDir, UserDatabase};
use tower::ServiceExt;

const LOAN: &str = "/api/v1/calculation/loan";

/// The database `tests/parity/generate_db_fixture.py` created with the Python
/// backend, and its password, which is not a secret.
const FIXTURE_DATABASE: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../infra-db/tests/fixtures/data.db"
);
const FIXTURE_PASSWORD: &str = "fixture pass'word ü";
const FIXTURE_USER: &str = "fixture-user";
const FIXTURE_USER_ID: &str = "0d3e7c1a-5b2f-4c8e-9a1b-2c3d4e5f6a7b";
/// A second user, whose database is another copy of the fixture database and so
/// opens with `FIXTURE_PASSWORD` too.
const OTHER_USER: &str = "other-user";
const OTHER_USER_ID: &str = "11111111-2222-4333-8444-555555555555";

/// A data directory of its own with a `profiles.json` that names `FIXTURE_USER`
/// and `OTHER_USER`, and a copy of the fixture database as each one's. It is removed when the
/// value is dropped, so keep it until the router and its database are gone.
struct TempDataDir(std::path::PathBuf);

impl TempDataDir {
    fn new() -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        // A directory that exists already, left by a run that stopped before it
        // cleaned up for example, is skipped, so that no test shares one; the
        // guard owns the directory before anything is written to it.
        let data_dir = loop {
            let directory = std::env::temp_dir().join(format!(
                "ferrofolio-gateway-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            match std::fs::create_dir(&directory) {
                Ok(()) => break TempDataDir(directory),
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
                Err(error) => panic!("cannot create {}: {error}", directory.display()),
            }
        };
        for id in [FIXTURE_USER_ID, OTHER_USER_ID] {
            let database = data_dir.0.join("profiles").join(id).join("data.db");
            std::fs::create_dir_all(database.parent().unwrap()).unwrap();
            std::fs::copy(FIXTURE_DATABASE, database).unwrap();
        }
        let profiles = json!({
            "last_logged": FIXTURE_USER_ID,
            "profiles": [
                {"id": FIXTURE_USER_ID, "name": FIXTURE_USER},
                {"id": OTHER_USER_ID, "name": OTHER_USER},
            ],
        });
        std::fs::write(data_dir.0.join("profiles.json"), profiles.to_string()).unwrap();
        data_dir
    }

    fn data_dir(&self) -> DataDir {
        DataDir::new(self.0.clone())
    }

    fn database(&self) -> std::path::PathBuf {
        self.0
            .join("profiles")
            .join(FIXTURE_USER_ID)
            .join("data.db")
    }
}

impl Drop for TempDataDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn fixture_credentials(password: &str) -> Credentials {
    Credentials {
        username: FIXTURE_USER.to_owned(),
        password: password.to_owned(),
    }
}

/// The body of a login of `FIXTURE_USER` with `password`.
fn fixture_login(password: &str) -> String {
    json!({"username": FIXTURE_USER, "password": password}).to_string()
}

/// The token the fake backend issues at its `n`th login, counted from 1.
fn backend_token(n: usize) -> String {
    format!("backend-token-{n}")
}

/// What the fake backend received.
#[derive(Debug, Default)]
struct Recorded {
    requests: Mutex<Vec<(Method, String, HeaderMap, Bytes)>>,
    /// Releases the second chunk of `/stream`.
    release_second_chunk: Arc<tokio::sync::Notify>,
    /// The backend runs without API sessions and is unlocked: its status is
    /// `UNLOCKED` for every request, with or without a token.
    sessions_disabled: AtomicBool,
    /// The logout handler fails after it has ended the backend's sessions.
    logout_fails: AtomicBool,
    /// The logout handler drops the connection without an answer after it has
    /// ended the backend's sessions.
    logout_answer_lost: AtomicBool,
    /// The logout handler, once it has ended the sessions, reports it with
    /// `logout_started` and waits for `release_logout` before it answers.
    logout_waits: AtomicBool,
    logout_started: Arc<tokio::sync::Notify>,
    release_logout: Arc<tokio::sync::Notify>,
    /// The backend's data is locked, so its session guard lets every logout
    /// through, as Python's does.
    guard_open: AtomicBool,
    /// Without API sessions, the backend's data is already unlocked, so a login
    /// is refused with 409.
    already_unlocked: AtomicBool,
    /// Without API sessions, the user whose data the backend has unlocked, which
    /// its status names: set by a login it accepts, cleared by a logout.
    unlocked_user: Mutex<Option<String>>,
    /// A login running at the same time has unlocked the data: the backend
    /// answers every login with 204, without checking the credentials, as
    /// Python does at `AlreadyUnlockedError`.
    overtaken: AtomicBool,
    /// The handlers that replace the database report with `replacement_started`
    /// and wait for `release_replacement` before they answer.
    replacement_waits: AtomicBool,
    replacement_started: Arc<tokio::sync::Notify>,
    release_replacement: Arc<tokio::sync::Notify>,
    /// The handlers that replace the database refuse a request without a token
    /// they issued with `401`, as Python's session guard does while unlocked.
    replacement_guarded: AtomicBool,
    /// The backup import fails with `500` once it has run.
    replacement_fails: AtomicBool,
    /// Logins so far, and the tokens the backend accepts.
    logins: AtomicUsize,
    accepted: Mutex<HashSet<String>>,
}

impl Recorded {
    fn paths(&self) -> Vec<String> {
        self.requests
            .lock()
            .unwrap()
            .iter()
            .map(|(_, path, _, _)| path.clone())
            .collect()
    }

    fn accepts(&self, headers: &HeaderMap) -> bool {
        let accepted = self.accepted.lock().unwrap();
        headers
            .get(header::AUTHORIZATION)
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.strip_prefix("Bearer "))
            .is_some_and(|token| accepted.contains(token))
    }

    /// The `Authorization` header of the last request, if it had one.
    fn last_authorization(&self) -> Option<String> {
        let requests = self.requests.lock().unwrap();
        let (_, _, headers, _) = requests.last().unwrap();
        headers
            .get(header::AUTHORIZATION)
            .map(|value| value.to_str().unwrap().to_owned())
    }
}

/// A backend that issues a new token at every login, `backend_token(n)`, with
/// `204` for a guest and without a token for the username `no-token`. Its status
/// is `UNLOCKED` and its logout accepted only with a token it issued; a logout
/// ends every token, as Python's does.
async fn fake_backend() -> (String, Arc<Recorded>) {
    let recorded = Arc::new(Recorded::default());

    async fn record(State(recorded): State<Arc<Recorded>>, request: Request) -> Response {
        let (parts, body) = request.into_parts();
        let body = to_bytes(body, usize::MAX).await.unwrap();
        let path = parts.uri.path_and_query().unwrap().to_string();
        recorded.requests.lock().unwrap().push((
            parts.method.clone(),
            path.clone(),
            parts.headers.clone(),
            body.clone(),
        ));

        match (parts.method, parts.uri.path()) {
            (Method::GET, "/api/v1/status") => {
                if recorded.sessions_disabled.load(Ordering::SeqCst) {
                    let user = recorded
                        .unlocked_user
                        .lock()
                        .unwrap()
                        .clone()
                        .map(|username| json!({"id": "hashed", "username": username}));
                    let status = json!({"status": "UNLOCKED", "apiSession": false, "user": user});
                    return axum::Json(status).into_response();
                }
                let status = if recorded.accepts(&parts.headers) {
                    "UNLOCKED"
                } else {
                    "LOCKED"
                };
                axum::Json(json!({"status": status, "apiSession": true})).into_response()
            }
            // Spellings that reach Python's login and signup handlers: uvicorn
            // decodes escapes and Quart collapses leading slashes and uppercases
            // the method.
            (
                method,
                "/api/v1/login" | "/api/v1/logi%6E" | "//api/v1/login" | "/api/v1/signup"
                | "/%2Fapi/v1/signup",
            ) if method.as_str().eq_ignore_ascii_case("POST") => {
                let login: Value = serde_json::from_slice(&body).unwrap_or_default();
                let sessions_disabled = recorded.sessions_disabled.load(Ordering::SeqCst);
                match login["username"].as_str() {
                    _ if recorded.overtaken.load(Ordering::SeqCst) => {
                        StatusCode::NO_CONTENT.into_response()
                    }
                    _ if sessions_disabled && recorded.already_unlocked.load(Ordering::SeqCst) => {
                        let body = json!({"message": "User already logged in"});
                        (StatusCode::CONFLICT, axum::Json(body)).into_response()
                    }
                    // A token although sessions are off, which Python never sends.
                    Some("token-without-sessions") if sessions_disabled => {
                        axum::Json(json!({"token": backend_token(1)})).into_response()
                    }
                    // Without API sessions Python answers a login without a token.
                    username if sessions_disabled => {
                        *recorded.unlocked_user.lock().unwrap() = username.map(str::to_owned);
                        StatusCode::NO_CONTENT.into_response()
                    }
                    Some("guest") => StatusCode::NO_CONTENT.into_response(),
                    Some("no-token") => axum::Json(json!({"message": "welcome"})).into_response(),
                    _ => {
                        let token =
                            backend_token(recorded.logins.fetch_add(1, Ordering::SeqCst) + 1);
                        recorded.accepted.lock().unwrap().insert(token.clone());
                        axum::Json(json!({"token": token})).into_response()
                    }
                }
            }
            (Method::POST, "/api/v1/logout") => {
                // Without API sessions Python has no session guard.
                let guard_open = recorded.guard_open.load(Ordering::SeqCst)
                    || recorded.sessions_disabled.load(Ordering::SeqCst);
                if !recorded.accepts(&parts.headers) && !guard_open {
                    let body = json!({"code": "NOT_LOGGED", "message": "Session required"});
                    return (StatusCode::UNAUTHORIZED, axum::Json(body)).into_response();
                }
                // Like Python's handler, it ends its sessions first.
                recorded.accepted.lock().unwrap().clear();
                *recorded.unlocked_user.lock().unwrap() = None;
                if recorded.logout_waits.load(Ordering::SeqCst) {
                    recorded.logout_started.notify_one();
                    recorded.release_logout.notified().await;
                }
                if recorded.logout_answer_lost.load(Ordering::SeqCst) {
                    // Ends the connection without an answer.
                    panic!("the fake backend drops the logout answer");
                }
                if recorded.logout_fails.load(Ordering::SeqCst) {
                    StatusCode::INTERNAL_SERVER_ERROR.into_response()
                } else {
                    StatusCode::NO_CONTENT.into_response()
                }
            }
            (Method::POST, path @ ("/api/v1/cloud/backup/import" | "/api/v1/change-password")) => {
                if recorded.replacement_guarded.load(Ordering::SeqCst)
                    && !recorded.accepts(&parts.headers)
                {
                    let body = json!({"code": "NOT_LOGGED", "message": "Session required"});
                    return (StatusCode::UNAUTHORIZED, axum::Json(body)).into_response();
                }
                if recorded.replacement_waits.load(Ordering::SeqCst) {
                    recorded.replacement_started.notify_one();
                    recorded.release_replacement.notified().await;
                }
                if recorded.replacement_fails.load(Ordering::SeqCst) {
                    StatusCode::INTERNAL_SERVER_ERROR.into_response()
                } else if path == "/api/v1/change-password" {
                    // Python rekeys the database, locks it and answers without a
                    // body.
                    StatusCode::NO_CONTENT.into_response()
                } else {
                    axum::Json(json!({})).into_response()
                }
            }
            (Method::OPTIONS, LOAN) => {
                (StatusCode::OK, [("x-answered-by", "backend")]).into_response()
            }
            (_, "/stream") => Response::new(Body::from_stream(gated_chunks(
                Bytes::from_static(b"first,"),
                Arc::clone(&recorded.release_second_chunk),
                Bytes::from_static(b"second"),
            ))),
            _ => (
                StatusCode::IM_A_TEAPOT,
                [("x-answered-by", "backend"), ("connection", "close")],
                body,
            )
                .into_response(),
        }
    }

    let app = Router::new()
        .route("/{*path}", any(record))
        .route("/", any(record))
        .with_state(Arc::clone(&recorded));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    (format!("http://{addr}"), recorded)
}

/// A stream that yields `first`, then waits for `release` before it yields `second`.
fn gated_chunks(
    first: Bytes,
    release: Arc<tokio::sync::Notify>,
    second: Bytes,
) -> impl futures_core::Stream<Item = Result<Bytes, std::io::Error>> {
    let (sender, receiver) = tokio::sync::mpsc::channel(1);
    tokio::spawn(async move {
        if sender.send(Ok(first)).await.is_err() {
            return;
        }
        release.notified().await;
        let _ = sender.send(Ok(second)).await;
    });
    ReceiverStream(receiver)
}

struct ReceiverStream(tokio::sync::mpsc::Receiver<Result<Bytes, std::io::Error>>);

impl futures_core::Stream for ReceiverStream {
    type Item = Result<Bytes, std::io::Error>;

    fn poll_next(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Option<Self::Item>> {
        self.0.poll_recv(cx)
    }
}

/// A loopback address that refuses connections: the port is bound by the returned
/// socket, which never listens, so no other process can take it during the test.
fn closed_port() -> (tokio::net::TcpSocket, String) {
    let socket = tokio::net::TcpSocket::new_v4().unwrap();
    socket.bind("127.0.0.1:0".parse().unwrap()).unwrap();
    let port = socket.local_addr().unwrap().port();
    (socket, format!("http://127.0.0.1:{port}"))
}

/// A wall clock the tests move.
#[derive(Debug)]
struct TestClock(Mutex<SystemTime>);

impl TestClock {
    fn new() -> Arc<Self> {
        Arc::new(TestClock(Mutex::new(SystemTime::UNIX_EPOCH)))
    }

    fn advance(&self, by: Duration) {
        *self.0.lock().unwrap() += by;
    }
}

impl SessionClock for TestClock {
    fn now(&self) -> SystemTime {
        *self.0.lock().unwrap()
    }
}

fn state(backend_url: &str, settings: SessionSettings, clock: Arc<TestClock>) -> AppState {
    let today = Date::new(2026, 11, 18).unwrap();
    AppState::new(
        Backend::new(backend_url).unwrap(),
        Arc::new(FixedClock(today)),
        Sessions::new(settings, clock),
    )
}

/// The gateway with API sessions.
fn gateway(backend_url: &str) -> Router {
    router(state(
        backend_url,
        SessionSettings::default(),
        TestClock::new(),
    ))
}

/// The gateway without API sessions.
fn gateway_without_sessions(backend_url: &str) -> Router {
    let settings = SessionSettings {
        enabled: false,
        remember_ttl: None,
    };
    router(state(backend_url, settings, TestClock::new()))
}

fn login_request(path: &str, body: &str) -> Request<Body> {
    Request::post(path)
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(body.to_owned()))
        .unwrap()
}

/// Logs in and returns the token of the new session.
async fn login(app: &Router) -> String {
    let request = login_request("/api/v1/login", r#"{"username":"u","password":"p"}"#);
    let (status, _, body) = send(app, request).await;
    assert_eq!(status, StatusCode::OK);
    let body: Value = serde_json::from_slice(&body).unwrap();
    body["token"].as_str().unwrap().to_owned()
}

fn get(path: &str, token: Option<&str>) -> Request<Body> {
    let mut builder = Request::get(path);
    if let Some(token) = token {
        builder = builder.header(header::AUTHORIZATION, format!("Bearer {token}"));
    }
    builder.body(Body::empty()).unwrap()
}

fn logout(token: Option<&str>) -> Request<Body> {
    let mut request = get("/api/v1/logout", token);
    *request.method_mut() = Method::POST;
    request
}

async fn send(app: &Router, request: Request<Body>) -> (StatusCode, HeaderMap, Bytes) {
    let response = app.clone().oneshot(request).await.unwrap();
    let status = response.status();
    let headers = response.headers().clone();
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    (status, headers, body)
}

fn loan_request(token: Option<&str>) -> Request<Body> {
    let mut builder = Request::post(LOAN).header(header::CONTENT_TYPE, "application/json");
    if let Some(token) = token {
        builder = builder.header(header::AUTHORIZATION, format!("Bearer {token}"));
    }
    let body = json!({
        "loan_amount": 200000,
        "interest_rate": 0.03,
        "interest_type": "FIXED",
        "start": "2020-01-15",
        "end": "2050-01-15",
    });
    builder.body(Body::from(body.to_string())).unwrap()
}

#[tokio::test]
async fn forwards_requests_unchanged_except_hop_by_hop_headers() {
    let (url, recorded) = fake_backend().await;
    let app = gateway(&url);

    let mut request = Request::put("/api/v1/positions/abc?filter=a%20b&x=1")
        .header(header::HOST, "finance.example.com")
        .header(header::AUTHORIZATION, "Bearer client-token")
        .header("x-forwarded-proto", "https")
        .header("x-forwarded-for", "198.51.100.1")
        .header(header::CONNECTION, "keep-alive, x-private-hop")
        .header("x-private-hop", "1")
        .header("keep-alive", "timeout=5")
        .header("cloud-authorization", "Bearer cloud")
        .body(Body::from("payload"))
        .unwrap();
    request.extensions_mut().insert(ConnectInfo(
        "203.0.113.9:50000".parse::<SocketAddr>().unwrap(),
    ));

    let (status, headers, body) = send(&app, request).await;
    assert_eq!(status, StatusCode::IM_A_TEAPOT);
    assert_eq!(headers["x-answered-by"], "backend");
    assert!(!headers.contains_key(header::CONNECTION));
    assert_eq!(body, "payload");

    let requests = recorded.requests.lock().unwrap();
    let (method, path, headers, body) = requests.last().unwrap();
    assert_eq!(method, Method::PUT);
    assert_eq!(path, "/api/v1/positions/abc?filter=a%20b&x=1");
    assert_eq!(headers[header::HOST], "finance.example.com");
    assert_eq!(headers[header::AUTHORIZATION], "Bearer client-token");
    assert_eq!(headers["cloud-authorization"], "Bearer cloud");
    assert_eq!(headers["x-forwarded-proto"], "https");
    assert_eq!(headers["x-forwarded-for"], "198.51.100.1, 203.0.113.9");
    assert!(!headers.contains_key("x-private-hop"));
    assert!(!headers.contains_key("keep-alive"));
    assert_eq!(body, "payload");
}

#[tokio::test]
async fn forwards_bodies_of_unknown_length_on_any_method() {
    let (url, recorded) = fake_backend().await;
    let app = gateway(&url);

    for method in [Method::GET, Method::DELETE, Method::POST] {
        // A body without Content-Length, as a chunked request arrives.
        let chunks = chunk_stream(vec![
            Bytes::from_static(b"first,"),
            Bytes::from_static(b"second"),
        ]);
        let request = Request::builder()
            .method(method.clone())
            .uri("/api/v1/echo")
            .body(Body::from_stream(chunks))
            .unwrap();

        let (status, _, body) = send(&app, request).await;
        assert_eq!(status, StatusCode::IM_A_TEAPOT, "{method}");
        assert_eq!(body, "first,second", "{method}");
        let requests = recorded.requests.lock().unwrap();
        let (recorded_method, _, _, recorded_body) = requests.last().unwrap();
        assert_eq!(recorded_method, method);
        assert_eq!(recorded_body, "first,second", "{method}");
    }
}

/// A stream of the given chunks.
fn chunk_stream(
    chunks: Vec<Bytes>,
) -> impl futures_core::Stream<Item = Result<Bytes, std::io::Error>> {
    let (sender, receiver) = tokio::sync::mpsc::channel(chunks.len().max(1));
    for chunk in chunks {
        sender.try_send(Ok(chunk)).unwrap();
    }
    ReceiverStream(receiver)
}

#[tokio::test]
async fn streams_backend_responses() {
    use http_body_util::BodyExt;

    let (url, recorded) = fake_backend().await;
    let app = gateway(&url);

    // The backend holds the second chunk back, so the response and the first chunk
    // only arrive if the gateway passes data on instead of buffering the body. The
    // chunk may arrive in several frames.
    let (status, mut body, first) = tokio::time::timeout(Duration::from_secs(5), async {
        let response = app
            .oneshot(Request::get("/stream").body(Body::empty()).unwrap())
            .await
            .unwrap();
        let status = response.status();
        let mut body = response.into_body();
        let mut first = Vec::new();
        while first.len() < b"first,".len() {
            let frame = body.frame().await.unwrap().unwrap();
            first.extend_from_slice(&frame.into_data().unwrap());
        }
        (status, body, first)
    })
    .await
    .expect("the gateway buffered the response");
    assert_eq!(status, StatusCode::OK);
    assert_eq!(first, b"first,");

    recorded.release_second_chunk.notify_one();
    let rest = tokio::time::timeout(
        Duration::from_secs(5),
        to_bytes(std::mem::take(&mut body), usize::MAX),
    )
    .await
    .expect("the rest of the body did not arrive")
    .unwrap();
    assert_eq!(rest, "second");
}

#[tokio::test]
async fn ported_routes_check_the_session_in_rust() {
    let (url, recorded) = fake_backend().await;
    let app = gateway(&url);

    // The backend's own token is no session of Rust's.
    for token in [None, Some("made-up"), Some("backend-token-1")] {
        let (status, _, body) = send(&app, loan_request(token)).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
        assert_eq!(
            serde_json::from_slice::<Value>(&body).unwrap(),
            json!({"code": "NOT_LOGGED", "message": "Session required"})
        );
    }

    let token = login(&app).await;
    let (status, _, body) = send(&app, loan_request(Some(&token))).await;
    assert_eq!(status, StatusCode::OK);
    let result: Value = serde_json::from_slice(&body).unwrap();
    let payment = &result["current_installment_payment"];
    assert!(payment.is_number());
    assert_eq!(
        Dezimal::from_python_str(&payment.to_string()).unwrap(),
        Dezimal::from_python_str("843.21").unwrap()
    );

    // Only the login reached the backend; neither the session check nor the
    // calculation did.
    assert_eq!(recorded.paths(), ["/api/v1/login"]);
}

#[tokio::test]
async fn ported_routes_need_no_token_while_api_sessions_are_disabled() {
    let (url, recorded) = fake_backend().await;
    recorded.sessions_disabled.store(true, Ordering::SeqCst);
    let app = gateway_without_sessions(&url);

    let (status, _, body) = send(&app, loan_request(None)).await;
    assert_eq!(status, StatusCode::OK);
    let result: Value = serde_json::from_slice(&body).unwrap();
    assert!(result["current_installment_payment"].is_number());
    // Python unlocks and locks the data also without a login Rust sees, so the
    // backend was asked for its status, without a token.
    let requests = recorded.requests.lock().unwrap();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].1, "/api/v1/status");
    assert!(!requests[0].2.contains_key(header::AUTHORIZATION));
}

#[tokio::test]
async fn without_sessions_a_login_unlocks_the_database_until_the_logout() {
    let (url, recorded) = fake_backend().await;
    recorded.sessions_disabled.store(true, Ordering::SeqCst);
    let settings = SessionSettings {
        enabled: false,
        remember_ttl: None,
    };
    let data = TempDataDir::new();
    let mut state = state(&url, settings, TestClock::new());
    state.data_dir = Some(data.data_dir());
    let app = router(state.clone());

    let request = login_request("/api/v1/login", &fixture_login(FIXTURE_PASSWORD));
    let (status, _, _) = send(&app, request).await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    assert!(state.sessions.is_unlocked());
    let (status, _, _) = send(&app, logout(None)).await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    assert!(!state.sessions.is_unlocked());

    // Python unlocked the data without a login Rust saw and refuses the next one
    // with 409; the right password still unlocks the database for Rust.
    *recorded.unlocked_user.lock().unwrap() = Some(FIXTURE_USER.to_owned());
    recorded.already_unlocked.store(true, Ordering::SeqCst);
    let request = login_request("/api/v1/login", &fixture_login("wrong password"));
    let (status, _, _) = send(&app, request).await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert!(!state.sessions.is_unlocked());
    let request = login_request("/api/v1/login", &fixture_login(FIXTURE_PASSWORD));
    let (status, _, _) = send(&app, request).await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert!(state.sessions.is_unlocked());
    drop(app);
}

/// The body of a login of `OTHER_USER` with the password that opens its database.
fn other_login() -> String {
    json!({"username": OTHER_USER, "password": FIXTURE_PASSWORD}).to_string()
}

/// Whether Rust holds the database of `OTHER_USER`: it reads the same data as
/// `FIXTURE_USER`'s, so the test looks at the files Rust has open. `None` where
/// the process cannot be asked, as outside Linux or without procfs: the test then
/// checks only what Rust reports. The directory is compared by its canonical
/// path, which the links in procfs name, also when the temporary directory is
/// reached through a symbolic link.
fn holds_other_users_database(data: &TempDataDir) -> Option<bool> {
    let directory = data.0.join("profiles").join(OTHER_USER_ID);
    let directory = std::fs::canonicalize(&directory).unwrap_or(directory);
    let handles = std::fs::read_dir("/proc/self/fd").ok()?;
    Some(handles.filter_map(Result::ok).any(|entry| {
        std::fs::read_link(entry.path()).is_ok_and(|target| target.starts_with(&directory))
    }))
}

fn assert_holds_other_users_database(data: &TempDataDir, expected: bool) {
    let holds = holds_other_users_database(data);
    // On Linux, where CI runs, procfs must answer, so the check is never skipped.
    if cfg!(target_os = "linux") {
        assert!(holds.is_some(), "/proc/self/fd cannot be read");
    }
    if let Some(holds) = holds {
        assert_eq!(
            holds, expected,
            "Rust's connection to the other user's data.db"
        );
    }
}

#[tokio::test]
async fn without_sessions_a_login_of_another_user_does_not_unlock_the_database() {
    let (url, recorded) = fake_backend().await;
    recorded.sessions_disabled.store(true, Ordering::SeqCst);
    let settings = SessionSettings {
        enabled: false,
        remember_ttl: None,
    };
    let data = TempDataDir::new();
    let mut state = state(&url, settings, TestClock::new());
    state.data_dir = Some(data.data_dir());
    let app = router(state.clone());

    // Python has unlocked `FIXTURE_USER`'s data and refuses a login of another
    // user with 409 without looking at its credentials, which open that user's
    // database all the same.
    *recorded.unlocked_user.lock().unwrap() = Some(FIXTURE_USER.to_owned());
    recorded.already_unlocked.store(true, Ordering::SeqCst);
    let (status, _, _) = send(&app, login_request("/api/v1/login", &other_login())).await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert!(!state.sessions.is_unlocked());

    // Rust holds `FIXTURE_USER`'s database; a refused login of another user
    // does not replace it.
    let request = login_request("/api/v1/login", &fixture_login(FIXTURE_PASSWORD));
    assert_eq!(send(&app, request).await.0, StatusCode::CONFLICT);
    assert!(reads(&state));
    let (status, _, _) = send(&app, login_request("/api/v1/login", &other_login())).await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert!(reads(&state));
    assert_holds_other_users_database(&data, false);

    // A login that another one overtook gets 204 from Python, also unchecked.
    recorded.already_unlocked.store(false, Ordering::SeqCst);
    recorded.overtaken.store(true, Ordering::SeqCst);
    let (status, _, _) = send(&app, login_request("/api/v1/login", &other_login())).await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    assert!(reads(&state));
    assert_holds_other_users_database(&data, false);

    // Once Python reports that user, its login opens its database.
    recorded.overtaken.store(false, Ordering::SeqCst);
    recorded.already_unlocked.store(true, Ordering::SeqCst);
    *recorded.unlocked_user.lock().unwrap() = Some(OTHER_USER.to_owned());
    let (status, _, _) = send(&app, login_request("/api/v1/login", &other_login())).await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert!(reads(&state));
    assert_holds_other_users_database(&data, true);
    drop(app);
}

/// Whether the session with `token` reads Rust's database.
fn session_reads(state: &AppState, token: &str) -> bool {
    let mut headers = HeaderMap::new();
    headers.insert(
        header::AUTHORIZATION,
        format!("Bearer {token}").parse().unwrap(),
    );
    state
        .sessions
        .with_database_of(&headers, |database| database.applied_migrations().is_ok())
        .unwrap_or(false)
}

/// The token of the session a login request starts.
async fn session_of(app: &Router, body: &str) -> String {
    let (status, _, body) = send(app, login_request("/api/v1/login", body)).await;
    assert_eq!(status, StatusCode::OK);
    let body: Value = serde_json::from_slice(&body).unwrap();
    body["token"].as_str().unwrap().to_owned()
}

#[tokio::test]
async fn a_session_reads_only_the_database_of_its_own_user() {
    let (url, _) = fake_backend().await;
    let data = TempDataDir::new();
    let mut state = state(&url, SessionSettings::default(), TestClock::new());
    state.data_dir = Some(data.data_dir());
    let app = router(state.clone());

    let first = session_of(&app, &fixture_login(FIXTURE_PASSWORD)).await;
    assert!(session_reads(&state, &first));

    // Python accepts a login whose database Rust cannot open: its session is
    // valid, but it never reads the database of the user before.
    let missing = json!({"username": "no-profile", "password": FIXTURE_PASSWORD}).to_string();
    let second = session_of(&app, &missing).await;
    assert!(state.sessions.is_valid(&{
        let mut headers = HeaderMap::new();
        headers.insert(
            header::AUTHORIZATION,
            format!("Bearer {second}").parse().unwrap(),
        );
        headers
    }));
    assert!(!session_reads(&state, &second));
    assert!(session_reads(&state, &first));

    // A login of the first user that reaches its session only after another
    // user's login has unlocked that user's database reads nothing either.
    let late = state.sessions.login_attempt();
    let other = session_of(&app, &other_login()).await;
    assert!(session_reads(&state, &other));
    assert!(
        !session_reads(&state, &first),
        "ended by the other user's unlock"
    );
    let late = state
        .sessions
        .start(late, false, "late-backend-token", Some(FIXTURE_USER))
        .unwrap();
    assert!(!session_reads(&state, &late));
    assert!(session_reads(&state, &other));
}

#[tokio::test]
async fn a_read_rust_cannot_serve_goes_to_python_without_waiting_for_a_permit() {
    let (url, _) = fake_backend().await;
    let data = TempDataDir::new();
    let mut state = state(&url, SessionSettings::default(), TestClock::new());
    state.data_dir = Some(data.data_dir());
    state.calculation_wait = Duration::from_millis(50);
    let app = router(state.clone());
    let token = session_of(&app, &fixture_login(FIXTURE_PASSWORD)).await;

    // Every permit is taken and Rust holds no database: the request goes to
    // Python at once instead of being refused as busy.
    let cores = state.calculations.available_permits();
    let _taken = Arc::clone(&state.calculations)
        .acquire_many_owned(u32::try_from(cores).unwrap())
        .await
        .unwrap();
    state.sessions.forget_database();
    let (status, headers, _) = send(&app, get("/api/v1/settings", Some(&token))).await;
    assert_eq!(status, StatusCode::IM_A_TEAPOT);
    assert_eq!(headers["x-answered-by"], "backend");
}

#[tokio::test]
async fn a_read_that_loses_its_database_while_it_waits_goes_to_python() {
    let (url, _) = fake_backend().await;
    let data = TempDataDir::new();
    let mut state = state(&url, SessionSettings::default(), TestClock::new());
    state.data_dir = Some(data.data_dir());
    state.calculation_wait = Duration::from_millis(300);
    let app = router(state.clone());
    let token = session_of(&app, &fixture_login(FIXTURE_PASSWORD)).await;

    // Every permit is taken; Rust drops its database while the request waits.
    let cores = state.calculations.available_permits();
    let _taken = Arc::clone(&state.calculations)
        .acquire_many_owned(u32::try_from(cores).unwrap())
        .await
        .unwrap();
    let request = tokio::spawn({
        let app = app.clone();
        let request = get("/api/v1/settings", Some(&token));
        async move { send(&app, request).await }
    });
    tokio::time::sleep(Duration::from_millis(100)).await;
    state.sessions.forget_database();
    let (status, headers, _) = request.await.unwrap();
    assert_eq!(status, StatusCode::IM_A_TEAPOT);
    assert_eq!(headers["x-answered-by"], "backend");
}

#[tokio::test]
async fn a_login_another_login_overtook_does_not_unlock_the_database() {
    let (url, recorded) = fake_backend().await;
    let data = TempDataDir::new();
    let mut state = state(&url, SessionSettings::default(), TestClock::new());
    state.data_dir = Some(data.data_dir());
    let app = router(state.clone());

    // Python answers 204 without checking the credentials; the login that
    // overtook it answers 200 and unlocks the database for Rust.
    recorded.overtaken.store(true, Ordering::SeqCst);
    let (status, _, _) = send(&app, login_request("/api/v1/login", &other_login())).await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    assert!(!state.sessions.is_unlocked());
    drop(app);
}

#[tokio::test]
async fn the_automatic_login_at_startup_unlocks_the_database() {
    let (url, _) = fake_backend().await;
    let data = TempDataDir::new();
    let mut state = state(&url, SessionSettings::default(), TestClock::new());
    state.data_dir = Some(data.data_dir());

    server::gateway::unlock_at_startup(&state, fixture_credentials("wrong password")).await;
    assert!(!state.sessions.is_unlocked());
    server::gateway::unlock_at_startup(&state, fixture_credentials(FIXTURE_PASSWORD)).await;
    assert!(state.sessions.is_unlocked());
}

/// Whether Rust can read its database now.
fn reads(state: &AppState) -> bool {
    state
        .sessions
        .with_database(|database| database.applied_migrations().is_ok())
        .unwrap_or(false)
}

/// A gateway logged in as `FIXTURE_USER`, whose database Rust has unlocked.
async fn unlocked_gateway(url: &str, data: &TempDataDir) -> (AppState, Router) {
    let mut state = state(url, SessionSettings::default(), TestClock::new());
    state.data_dir = Some(data.data_dir());
    let app = router(state.clone());
    let request = login_request("/api/v1/login", &fixture_login(FIXTURE_PASSWORD));
    assert_eq!(send(&app, request).await.0, StatusCode::OK);
    assert!(reads(&state));
    (state, app)
}

/// Sends a request that replaces the database and waits until the backend holds it.
async fn start_replacement(
    app: &Router,
    recorded: &Recorded,
    path: &str,
) -> tokio::task::JoinHandle<Result<Response, std::convert::Infallible>> {
    let client = tokio::spawn({
        let app = app.clone();
        let request = login_request(path, "{}");
        async move { app.oneshot(request).await }
    });
    tokio::time::timeout(
        Duration::from_secs(10),
        recorded.replacement_started.notified(),
    )
    .await
    .expect("the request never reached the backend");
    client
}

/// Waits until Rust reads its database again.
async fn assert_reopens(state: &AppState) {
    let reopened = async {
        while !reads(state) {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    };
    tokio::time::timeout(Duration::from_secs(10), reopened)
        .await
        .expect("the database stayed closed after Python answered");
}

/// Waits until `condition` holds, or fails the test after ten seconds.
async fn wait_until(mut condition: impl FnMut() -> bool, what: &str) {
    let waiting = async {
        while !condition() {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    };
    tokio::time::timeout(Duration::from_secs(10), waiting)
        .await
        .unwrap_or_else(|_| panic!("{what}"));
}

/// Whether this process holds a connection to the database of `data`, judged by
/// the file descriptors that point at it or at a SQLite side file named after it
/// (`-wal`, `-shm`, `-journal`). `None` where the process cannot be asked, as
/// outside Linux: the test then checks only what Rust reports.
fn holds_connection(data: &TempDataDir) -> Option<bool> {
    #[cfg(target_os = "linux")]
    {
        let path = data.database();
        let path = path.to_string_lossy();
        let side = format!("{path}-");
        let handles = std::fs::read_dir("/proc/self/fd")
            .ok()?
            .filter_map(Result::ok)
            .filter_map(|entry| std::fs::read_link(entry.path()).ok())
            .filter(|target| {
                let target = target.to_string_lossy();
                target == path || target.starts_with(&side)
            })
            .count();
        Some(handles > 0)
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = data;
        None
    }
}

fn assert_holds_connection(data: &TempDataDir, expected: bool) {
    if let Some(holds) = holds_connection(data) {
        assert_eq!(holds, expected, "Rust's connection to data.db");
    }
}

#[tokio::test]
async fn requests_that_replace_the_database_close_rusts_connection_until_python_answers() {
    let (url, recorded) = fake_backend().await;
    recorded.replacement_waits.store(true, Ordering::SeqCst);
    let data = TempDataDir::new();
    let (state, app) = unlocked_gateway(&url, &data).await;
    assert_holds_connection(&data, true);

    // A backup import keeps the password, so Rust opens the file again.
    let client = start_replacement(&app, &recorded, "/api/v1/cloud/backup/import").await;
    assert!(!reads(&state), "the import");
    assert_holds_connection(&data, false);
    recorded.release_replacement.notify_one();
    assert_eq!(client.await.unwrap().unwrap().status(), StatusCode::OK);
    assert_reopens(&state).await;
    assert_holds_connection(&data, true);

    // A change of the password rekeys and locks the database: Rust keeps no
    // connection, and the next login opens the file again.
    let client = start_replacement(&app, &recorded, "/api/v1/change-password").await;
    assert!(!reads(&state), "the change of the password");
    assert_holds_connection(&data, false);
    recorded.release_replacement.notify_one();
    assert_eq!(
        client.await.unwrap().unwrap().status(),
        StatusCode::NO_CONTENT
    );
    assert!(!state.sessions.is_unlocked());
    assert!(!reads(&state), "after the change of the password");
    assert_holds_connection(&data, false);

    let request = login_request("/api/v1/login", &fixture_login(FIXTURE_PASSWORD));
    assert_eq!(send(&app, request).await.0, StatusCode::OK);
    assert!(reads(&state));
}

#[tokio::test]
async fn a_failed_replacement_with_a_session_leaves_rust_without_a_connection() {
    let (url, recorded) = fake_backend().await;
    recorded.replacement_guarded.store(true, Ordering::SeqCst);
    let data = TempDataDir::new();
    let mut state = state(&url, SessionSettings::default(), TestClock::new());
    state.data_dir = Some(data.data_dir());
    let app = router(state.clone());
    let request = login_request("/api/v1/login", &fixture_login(FIXTURE_PASSWORD));
    let (status, _, body) = send(&app, request).await;
    assert_eq!(status, StatusCode::OK);
    let token: Value = serde_json::from_slice(&body).unwrap();
    let token = token["token"].as_str().unwrap().to_owned();
    assert!(reads(&state));

    // Python's guard refuses an import without a session before it touches the
    // database: Rust keeps its connection.
    let request = login_request("/api/v1/cloud/backup/import", "{}");
    assert_eq!(send(&app, request).await.0, StatusCode::UNAUTHORIZED);
    assert!(reads(&state), "after an import the guard refused");

    // An import that fails may have replaced the file first: Rust keeps no
    // connection until the next login.
    recorded.replacement_fails.store(true, Ordering::SeqCst);
    let mut request = login_request("/api/v1/cloud/backup/import", "{}");
    request.headers_mut().insert(
        header::AUTHORIZATION,
        format!("Bearer {token}").parse().unwrap(),
    );
    assert_eq!(
        send(&app, request).await.0,
        StatusCode::INTERNAL_SERVER_ERROR
    );
    assert!(!state.sessions.is_unlocked());
    assert!(!reads(&state), "after a failed import");
    assert_holds_connection(&data, false);
}

#[tokio::test]
async fn requests_that_replace_the_database_queue_only_up_to_their_slots() {
    let (url, recorded) = fake_backend().await;
    recorded.replacement_waits.store(true, Ordering::SeqCst);
    let data = TempDataDir::new();
    let (state, app) = unlocked_gateway(&url, &data).await;

    // One import is with Python and three wait for it.
    let running = start_replacement(&app, &recorded, "/api/v1/cloud/backup/import").await;
    let waiting: Vec<_> = (0..server::routes::REPLACEMENT_SLOTS - 1)
        .map(|_| {
            let app = app.clone();
            let request = login_request("/api/v1/cloud/backup/import", "{}");
            tokio::spawn(async move { app.oneshot(request).await })
        })
        .collect();
    wait_until(
        || state.sessions.waiting_replacements() == server::routes::REPLACEMENT_SLOTS - 1,
        "the waiting imports never reached the lock",
    )
    .await;

    // A further one is refused at once, before its body is read.
    let request = login_request("/api/v1/cloud/backup/import", "{}");
    let (status, headers, body) = send(&app, request).await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(headers[header::RETRY_AFTER], "5");
    let body: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(body["code"], "SERVER_BUSY");

    // The fake backend holds each import until it is released.
    let release = async {
        loop {
            recorded.release_replacement.notify_one();
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    };
    let finished = async {
        assert_eq!(running.await.unwrap().unwrap().status(), StatusCode::OK);
        for client in waiting {
            assert_eq!(client.await.unwrap().unwrap().status(), StatusCode::OK);
        }
    };
    tokio::time::timeout(Duration::from_secs(10), async {
        tokio::select! {
            () = finished => {}
            () = release => {}
        }
    })
    .await
    .expect("the queued imports did not finish");

    // The slots are free again.
    recorded.replacement_waits.store(false, Ordering::SeqCst);
    let request = login_request("/api/v1/cloud/backup/import", "{}");
    assert_eq!(send(&app, request).await.0, StatusCode::OK);
}

#[tokio::test]
async fn a_client_that_goes_away_does_not_reopen_the_database() {
    let (url, recorded) = fake_backend().await;
    recorded.replacement_waits.store(true, Ordering::SeqCst);
    let data = TempDataDir::new();
    let (state, app) = unlocked_gateway(&url, &data).await;

    let client = start_replacement(&app, &recorded, "/api/v1/cloud/backup/import").await;
    client.abort();
    assert!(client.await.unwrap_err().is_cancelled());
    assert!(!reads(&state));
    assert_holds_connection(&data, false);
    recorded.release_replacement.notify_one();
    assert_reopens(&state).await;
}

#[tokio::test]
async fn a_login_during_a_replacement_opens_the_database_only_after_python_answered() {
    let (url, recorded) = fake_backend().await;
    recorded.replacement_waits.store(true, Ordering::SeqCst);
    let data = TempDataDir::new();
    let mut state = state(&url, SessionSettings::default(), TestClock::new());
    state.data_dir = Some(data.data_dir());
    let app = router(state.clone());

    let replacement = start_replacement(&app, &recorded, "/api/v1/cloud/backup/import").await;
    let login = tokio::spawn({
        let app = app.clone();
        let request = login_request("/api/v1/login", &fixture_login(FIXTURE_PASSWORD));
        async move { app.oneshot(request).await }
    });
    // Python has accepted the login, but Rust opens nothing while the import runs.
    wait_until(
        || state.sessions.waiting_openings() == 1,
        "the login never reached the replacement lock",
    )
    .await;
    assert!(!login.is_finished());
    assert!(!state.sessions.is_unlocked());

    recorded.release_replacement.notify_one();
    assert_eq!(replacement.await.unwrap().unwrap().status(), StatusCode::OK);
    let response = tokio::time::timeout(Duration::from_secs(10), login)
        .await
        .expect("the login did not finish after the import")
        .unwrap()
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert!(reads(&state));
}

#[tokio::test]
async fn a_replacement_waits_for_a_reader_of_the_database() {
    let (url, recorded) = fake_backend().await;
    let data = TempDataDir::new();
    let (state, app) = unlocked_gateway(&url, &data).await;

    // A reader holds the database until it is told to finish.
    let (finish, finished) = std::sync::mpsc::channel::<()>();
    let (reading, started) = tokio::sync::oneshot::channel::<()>();
    let reader = tokio::task::spawn_blocking({
        let sessions = state.sessions.clone();
        move || {
            sessions.with_database(|_| {
                reading.send(()).unwrap();
                finished.recv().unwrap();
            })
        }
    });
    tokio::time::timeout(Duration::from_secs(10), started)
        .await
        .expect("the reader never started")
        .unwrap();

    let replacement = tokio::spawn({
        let app = app.clone();
        let request = login_request("/api/v1/cloud/backup/import", "{}");
        async move { app.oneshot(request).await }
    });
    wait_until(
        || state.sessions.waiting_replacements() == 1,
        "the replacement never reached the lock",
    )
    .await;
    assert!(
        !recorded
            .paths()
            .contains(&"/api/v1/cloud/backup/import".to_owned())
    );

    finish.send(()).unwrap();
    assert!(reader.await.unwrap().is_ok());
    let response = tokio::time::timeout(Duration::from_secs(10), replacement)
        .await
        .expect("the import never reached the backend")
        .unwrap()
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert!(
        recorded
            .paths()
            .contains(&"/api/v1/cloud/backup/import".to_owned())
    );
}

#[tokio::test]
async fn a_replaced_database_file_is_opened_again() {
    let data = TempDataDir::new();
    let mut database =
        UserDatabase::open(&data.data_dir(), fixture_credentials(FIXTURE_PASSWORD)).unwrap();
    assert!(database.current().is_ok());

    // A backup import replaces the file. A file that is no database proves that
    // the new one is read rather than the old one, which stays open.
    let replacement = data.0.join("replacement.db");
    std::fs::write(
        &replacement,
        b"not a database, long enough to be read as one page",
    )
    .unwrap();
    std::fs::rename(&replacement, data.database()).unwrap();
    assert!(database.current().is_err());

    std::fs::copy(FIXTURE_DATABASE, &replacement).unwrap();
    std::fs::rename(&replacement, data.database()).unwrap();
    assert!(database.current().is_ok());
    assert!(database.current().unwrap().applied_migrations().is_ok());
}

#[tokio::test]
async fn a_login_unlocks_the_database_until_the_logout() {
    let (url, _) = fake_backend().await;
    let data = TempDataDir::new();
    let mut state = state(&url, SessionSettings::default(), TestClock::new());
    state.data_dir = Some(data.data_dir());
    let app = router(state.clone());

    let request = login_request("/api/v1/login", &fixture_login(FIXTURE_PASSWORD));
    let (status, _, body) = send(&app, request).await;
    assert_eq!(status, StatusCode::OK);
    assert!(state.sessions.is_unlocked());
    assert_eq!(state.sessions.database_openings(), 1);
    let token = serde_json::from_slice::<Value>(&body).unwrap()["token"]
        .as_str()
        .unwrap()
        .to_owned();

    // A further login of the same user, from another client, keeps the open
    // connection instead of deriving the key again.
    let request = login_request("/api/v1/login", &fixture_login(FIXTURE_PASSWORD));
    assert_eq!(send(&app, request).await.0, StatusCode::OK);
    assert!(reads(&state));
    assert_eq!(state.sessions.database_openings(), 1);

    let (status, _, _) = send(&app, logout(Some(&token))).await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    assert!(!state.sessions.is_unlocked());

    // After the logout the next login opens the database again.
    let request = login_request("/api/v1/login", &fixture_login(FIXTURE_PASSWORD));
    assert_eq!(send(&app, request).await.0, StatusCode::OK);
    assert!(reads(&state));
    assert_eq!(state.sessions.database_openings(), 2);
    drop(app);
}

#[tokio::test]
async fn a_login_rust_cannot_unlock_with_still_starts_a_session() {
    let (url, _) = fake_backend().await;
    let data = TempDataDir::new();
    let mut state = state(&url, SessionSettings::default(), TestClock::new());
    state.data_dir = Some(data.data_dir());
    let app = router(state.clone());

    // The fake backend accepts any password; the database does not. An unknown
    // user has no database at all.
    for body in [
        fixture_login("wrong password"),
        json!({"username": "unknown", "password": FIXTURE_PASSWORD}).to_string(),
    ] {
        let (status, _, body) = send(&app, login_request("/api/v1/login", &body)).await;
        assert_eq!(status, StatusCode::OK);
        assert!(!state.sessions.is_unlocked());
        let token = serde_json::from_slice::<Value>(&body).unwrap()["token"]
            .as_str()
            .unwrap()
            .to_owned();
        let (status, _, _) = send(&app, loan_request(Some(&token))).await;
        assert_eq!(status, StatusCode::OK);
    }

    // pydantic reads "false" as false, so this signup is no guest's and unlocks.
    let body = json!({
        "username": FIXTURE_USER,
        "password": FIXTURE_PASSWORD,
        "guest": "false",
    });
    let (status, _, answer) = send(&app, login_request("/api/v1/signup", &body.to_string())).await;
    assert_eq!(status, StatusCode::OK);
    assert!(state.sessions.is_unlocked());
    let token = serde_json::from_slice::<Value>(&answer).unwrap()["token"]
        .as_str()
        .unwrap()
        .to_owned();
    send(&app, logout(Some(&token))).await;
    assert!(!state.sessions.is_unlocked());

    // A guest's signup leaves the data locked, even with the right password.
    let body = json!({
        "username": FIXTURE_USER,
        "password": FIXTURE_PASSWORD,
        "guest": true,
    });
    let (status, _, _) = send(&app, login_request("/api/v1/signup", &body.to_string())).await;
    assert_eq!(status, StatusCode::OK);
    assert!(!state.sessions.is_unlocked());
}

#[tokio::test]
async fn ported_routes_spelled_with_percent_escapes_are_served_by_rust() {
    let (url, recorded) = fake_backend().await;
    let app = gateway(&url);
    let token = login(&app).await;

    // Spellings the backend would route to its loan handler: escapes, repeated
    // leading slashes, an escaped slash and a lowercase method.
    let post = Method::POST;
    let lowercase_post = Method::from_bytes(b"post").unwrap();
    for (method, path) in [
        (&post, "/api/v1/calculation/%6Coan?x=1"),
        (&post, "//api/v1/calculation/loan"),
        (&post, "/%2Fapi/v1/calculation/loan"),
        (&lowercase_post, "/api/v1/calculation/loan"),
    ] {
        let mut request = loan_request(Some(&token));
        *request.method_mut() = method.clone();
        *request.uri_mut() = path.parse().unwrap();
        let (status, _, body) = send(&app, request).await;
        assert_eq!(status, StatusCode::OK, "{method} {path}");
        let result: Value = serde_json::from_slice(&body).unwrap();
        assert!(
            result["current_installment_payment"].is_number(),
            "{method} {path}"
        );
    }
    assert_eq!(recorded.paths(), ["/api/v1/login"]);

    // Served by Rust, these spellings get its body limit too.
    let request = Request::post("//api/v1/calculation/loan")
        .header(header::CONTENT_TYPE, "application/json")
        .header(header::CONTENT_LENGTH, "1048577")
        .header(header::AUTHORIZATION, format!("Bearer {token}"))
        .body(Body::empty())
        .unwrap();
    let (status, _, _) = send(&app, request).await;
    assert_eq!(status, StatusCode::PAYLOAD_TOO_LARGE);
    assert_eq!(recorded.paths(), ["/api/v1/login"]);

    // Other methods still reach the backend with the path as the client sent it.
    let (status, _, _) = send(
        &app,
        Request::options("/api/v1/calculation/%6Coan")
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert_eq!(status, StatusCode::IM_A_TEAPOT);
    assert_eq!(
        recorded.paths(),
        ["/api/v1/login", "/api/v1/calculation/%6Coan"]
    );
}

#[tokio::test]
async fn ported_routes_refuse_a_declared_body_above_the_limit_without_reading_it() {
    let (url, _) = fake_backend().await;
    let app = gateway(&url);
    let token = login(&app).await;
    let first_read = Arc::new(tokio::sync::Notify::new());
    let request = Request::post(LOAN)
        .header(header::CONTENT_TYPE, "application/json")
        .header(header::CONTENT_LENGTH, "1048577")
        .header(header::AUTHORIZATION, format!("Bearer {token}"))
        .body(Body::from_stream(PendingBody(Arc::clone(&first_read))))
        .unwrap();
    let response = tokio::time::timeout(Duration::from_secs(30), app.oneshot(request))
        .await
        .expect("the gateway waited for the body")
        .unwrap();
    assert_eq!(response.status(), StatusCode::PAYLOAD_TOO_LARGE);
    // A poll of the body would have stored a notification, which would complete
    // this wait at once.
    assert!(
        tokio::time::timeout(Duration::ZERO, first_read.notified())
            .await
            .is_err(),
        "the gateway read the body"
    );
}

#[tokio::test]
async fn logins_answer_with_a_token_of_rust_that_is_not_cached() {
    let (url, recorded) = fake_backend().await;
    let app = gateway(&url);
    let post = Method::POST;
    let lowercase_post = Method::from_bytes(b"post").unwrap();
    let spellings = [
        (&post, "/api/v1/login"),
        (&post, "/api/v1/logi%6E"),
        (&lowercase_post, "/api/v1/login"),
        (&post, "//api/v1/login"),
        (&post, "/%2Fapi/v1/signup"),
    ];
    let mut tokens = Vec::new();
    for (method, path) in spellings {
        let mut request = login_request(path, r#"{"username":"u","password":"p"}"#);
        *request.method_mut() = method.clone();
        let (status, headers, body) = send(&app, request).await;
        assert_eq!(status, StatusCode::OK, "{method} {path}");
        assert_eq!(
            headers[header::CACHE_CONTROL],
            "no-store",
            "{method} {path}"
        );
        assert_eq!(
            headers[header::CONTENT_LENGTH],
            body.len().to_string(),
            "{method} {path}"
        );
        // The backend's token never reaches the client.
        let body = std::str::from_utf8(&body).unwrap();
        assert!(!body.contains("backend-token"), "{method} {path}");
        let token = serde_json::from_str::<Value>(body).unwrap()["token"]
            .as_str()
            .unwrap()
            .to_owned();
        assert_eq!(body, format!("{{\"token\":\"{token}\"}}\n"));
        assert_eq!(token.len(), 43);
        tokens.push(token);
    }
    tokens.sort();
    tokens.dedup();
    assert_eq!(tokens.len(), spellings.len());
    assert_eq!(recorded.paths(), spellings.map(|(_, path)| path.to_owned()));

    // Other responses keep the backend's headers.
    let (_, headers, _) = send(&app, get("/api/v1/other", None)).await;
    assert!(!headers.contains_key(header::CACHE_CONTROL));
}

#[tokio::test]
async fn logins_ask_the_backend_for_a_token_that_lasts_until_the_logout() {
    let (url, recorded) = fake_backend().await;
    let app = gateway(&url);
    let body = r#"{"username":"u","password":"p","remember":true}"#;
    let (status, _, _) = send(&app, login_request("/api/v1/login", body)).await;
    assert_eq!(status, StatusCode::OK);

    let requests = recorded.requests.lock().unwrap();
    let (_, _, headers, forwarded) = requests.last().unwrap();
    let expected = r#"{"username":"u","password":"p","remember":true,"remember":false}"#;
    assert_eq!(forwarded, expected);
    assert_eq!(headers[header::CONTENT_LENGTH], expected.len().to_string());
}

#[tokio::test]
async fn a_session_reaches_the_backend_with_the_backend_token() {
    let (url, recorded) = fake_backend().await;
    let app = gateway(&url);
    let token = login(&app).await;

    send(&app, get("/api/v1/positions", Some(&token))).await;
    assert_eq!(
        recorded.last_authorization(),
        Some(format!("Bearer {}", backend_token(1)))
    );
    let (status, _, body) = send(&app, get("/api/v1/status", Some(&token))).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        serde_json::from_slice::<Value>(&body).unwrap()["status"],
        "UNLOCKED"
    );

    // Python's session guard decides about every other request, which reaches it
    // as the client sent it, including the MCP endpoint's own token.
    for path in ["/api/v1/positions", "/mcp", "/api/v1/status"] {
        send(&app, get(path, Some("mcp-or-unknown-token"))).await;
        assert_eq!(
            recorded.last_authorization().as_deref(),
            Some("Bearer mcp-or-unknown-token"),
            "{path}"
        );
        send(&app, get(path, None)).await;
        assert_eq!(recorded.last_authorization(), None, "{path}");
    }
    let (_, _, body) = send(&app, get("/api/v1/status", None)).await;
    assert_eq!(
        serde_json::from_slice::<Value>(&body).unwrap()["status"],
        "LOCKED"
    );
}

#[tokio::test]
async fn a_logout_ends_every_session() {
    let (url, recorded) = fake_backend().await;
    let app = gateway(&url);
    let first = login(&app).await;
    let second = login(&app).await;

    // Each session reaches the backend with the token of its own login.
    send(&app, get("/api/v1/positions", Some(&second))).await;
    assert_eq!(
        recorded.last_authorization(),
        Some(format!("Bearer {}", backend_token(2)))
    );
    let (status, _, _) = send(&app, logout(Some(&first))).await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    assert_eq!(
        recorded.last_authorization(),
        Some(format!("Bearer {}", backend_token(1)))
    );
    assert_ended(&app, &recorded, &[&first, &second]).await;

    // A new login starts a session with a new backend token; the ended ones stay
    // ended.
    let third = login(&app).await;
    let (status, _, body) = send(&app, get("/api/v1/status", Some(&third))).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        serde_json::from_slice::<Value>(&body).unwrap()["status"],
        "UNLOCKED"
    );
    assert_eq!(
        recorded.last_authorization(),
        Some(format!("Bearer {}", backend_token(3)))
    );
    assert_ended(&app, &recorded, &[&first, &second]).await;
}

/// Checks that each token is no session: ported routes refuse it, and the backend
/// gets it as the client sent it.
async fn assert_ended(app: &Router, recorded: &Recorded, tokens: &[&str]) {
    for token in tokens {
        let (status, _, _) = send(app, loan_request(Some(token))).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
        let (_, _, body) = send(app, get("/api/v1/status", Some(token))).await;
        assert_eq!(
            recorded.last_authorization(),
            Some(format!("Bearer {token}"))
        );
        assert_eq!(
            serde_json::from_slice::<Value>(&body).unwrap()["status"],
            "LOCKED"
        );
    }
}

#[tokio::test]
async fn a_logout_whose_answer_is_lost_ends_the_sessions() {
    let (url, recorded) = fake_backend().await;
    recorded.logout_answer_lost.store(true, Ordering::SeqCst);
    let app = gateway(&url);
    let token = login(&app).await;

    let (status, _, _) = send(&app, logout(Some(&token))).await;
    assert_eq!(status, StatusCode::BAD_GATEWAY);
    // Otherwise the next login's backend token would bring the session back.
    login(&app).await;
    assert_ended(&app, &recorded, &[&token]).await;
}

#[tokio::test]
async fn a_logout_with_a_session_ends_the_sessions_before_the_backend_answers() {
    let (url, recorded) = fake_backend().await;
    recorded.logout_waits.store(true, Ordering::SeqCst);
    let app = gateway(&url);
    let token = login(&app).await;

    let client = tokio::spawn({
        let app = app.clone();
        let request = logout(Some(&token));
        async move { app.oneshot(request).await }
    });
    tokio::time::timeout(Duration::from_secs(10), recorded.logout_started.notified())
        .await
        .expect("the logout never reached the backend");
    // Python has revoked its sessions, or is about to: the backend token of a
    // login during the logout must not reach the ended session.
    let (status, _, _) = send(&app, loan_request(Some(&token))).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);

    recorded.release_logout.notify_one();
    let response = tokio::time::timeout(Duration::from_secs(10), client)
        .await
        .expect("the logout did not end")
        .unwrap()
        .unwrap();
    assert_eq!(response.status(), StatusCode::NO_CONTENT);
    login(&app).await;
    assert_ended(&app, &recorded, &[&token]).await;
}

#[tokio::test]
async fn a_logout_ends_the_sessions_also_when_its_client_goes_away() {
    let (url, recorded) = fake_backend().await;
    let app = gateway(&url);
    let token = login(&app).await;
    // Python has locked the data on its own, so its guard lets a logout without a
    // session through, and the handler ends Python's sessions.
    recorded.guard_open.store(true, Ordering::SeqCst);
    recorded.logout_waits.store(true, Ordering::SeqCst);

    let client = tokio::spawn({
        let app = app.clone();
        let request = logout(None);
        async move { app.oneshot(request).await }
    });
    tokio::time::timeout(Duration::from_secs(10), recorded.logout_started.notified())
        .await
        .expect("the logout never reached the backend");
    // The session lasts until Python has answered.
    let (status, _, _) = send(&app, loan_request(Some(&token))).await;
    assert_eq!(status, StatusCode::OK);
    client.abort();
    assert!(client.await.unwrap_err().is_cancelled());
    recorded.release_logout.notify_one();

    let ended = async {
        loop {
            let (status, _, _) = send(&app, loan_request(Some(&token))).await;
            if status == StatusCode::UNAUTHORIZED {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    };
    tokio::time::timeout(Duration::from_secs(10), ended)
        .await
        .expect("the session outlived the logout");
}

#[tokio::test]
async fn a_login_during_a_late_logout_does_not_revive_the_ended_sessions() {
    let (url, recorded) = fake_backend().await;
    let app = gateway(&url);
    let ended = login(&app).await;
    // Python has locked the data on its own and lets a logout without a session
    // through; it ends its sessions at once but answers late.
    recorded.guard_open.store(true, Ordering::SeqCst);
    recorded.logout_waits.store(true, Ordering::SeqCst);
    let client = tokio::spawn({
        let app = app.clone();
        let request = logout(None);
        async move { app.oneshot(request).await }
    });
    tokio::time::timeout(Duration::from_secs(10), recorded.logout_started.notified())
        .await
        .expect("the logout never reached the backend");

    // A login meanwhile gets a new backend token, which the ended session never
    // uses: it keeps the token of its own login, which Python has ended.
    let fresh = login(&app).await;
    let (_, _, body) = send(&app, get("/api/v1/status", Some(&ended))).await;
    assert_eq!(
        recorded.last_authorization(),
        Some(format!("Bearer {}", backend_token(1)))
    );
    assert_eq!(
        serde_json::from_slice::<Value>(&body).unwrap()["status"],
        "LOCKED"
    );
    let (_, _, body) = send(&app, get("/api/v1/status", Some(&fresh))).await;
    assert_eq!(
        serde_json::from_slice::<Value>(&body).unwrap()["status"],
        "UNLOCKED"
    );

    // Once Python answers, Rust ends every session.
    recorded.release_logout.notify_one();
    let response = tokio::time::timeout(Duration::from_secs(10), client)
        .await
        .expect("the logout did not end")
        .unwrap()
        .unwrap();
    assert_eq!(response.status(), StatusCode::NO_CONTENT);
    for token in [&ended, &fresh] {
        let (status, _, _) = send(&app, loan_request(Some(token))).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
    }
}

#[tokio::test]
async fn a_logout_the_backend_refuses_keeps_the_sessions() {
    let (url, _) = fake_backend().await;
    let app = gateway(&url);
    let token = login(&app).await;

    for request in [logout(None), logout(Some("made-up"))] {
        let (status, _, _) = send(&app, request).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
    }
    let (status, _, _) = send(&app, loan_request(Some(&token))).await;
    assert_eq!(status, StatusCode::OK);
}

#[tokio::test]
async fn a_logout_that_fails_in_the_backend_ends_the_sessions() {
    let (url, recorded) = fake_backend().await;
    recorded.logout_fails.store(true, Ordering::SeqCst);
    let app = gateway(&url);
    let token = login(&app).await;

    let (status, _, _) = send(&app, logout(Some(&token))).await;
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
    login(&app).await;
    assert_ended(&app, &recorded, &[&token]).await;
}

#[tokio::test]
async fn a_remembered_session_expires_after_the_remember_period() {
    let (url, recorded) = fake_backend().await;
    let clock = TestClock::new();
    let settings = SessionSettings {
        enabled: true,
        remember_ttl: Some(Duration::from_secs(86_400)),
    };
    let app = router(state(&url, settings, Arc::clone(&clock)));
    let remembered = login_request(
        "/api/v1/login",
        r#"{"username":"u","password":"p","remember":true}"#,
    );
    let (_, _, body) = send(&app, remembered).await;
    let remembered = serde_json::from_slice::<Value>(&body).unwrap()["token"]
        .as_str()
        .unwrap()
        .to_owned();
    let forgotten = login(&app).await;

    clock.advance(Duration::from_secs(86_399));
    let (status, _, _) = send(&app, loan_request(Some(&remembered))).await;
    assert_eq!(status, StatusCode::OK);
    clock.advance(Duration::from_secs(1));
    let (status, _, _) = send(&app, loan_request(Some(&remembered))).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    send(&app, get("/api/v1/positions", Some(&remembered))).await;
    assert_eq!(
        recorded.last_authorization(),
        Some(format!("Bearer {remembered}"))
    );

    // A session that is not remembered lasts until the logout.
    clock.advance(Duration::from_secs(365 * 86_400));
    let (status, _, _) = send(&app, loan_request(Some(&forgotten))).await;
    assert_eq!(status, StatusCode::OK);
}

#[tokio::test]
async fn logins_without_a_token_start_no_session() {
    let (url, _) = fake_backend().await;
    let app = gateway(&url);

    // A guest's signup leaves the data locked and answers without a body.
    let request = login_request("/api/v1/signup", r#"{"username":"guest","guest":true}"#);
    let (status, _, body) = send(&app, request).await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    assert!(body.is_empty());

    // An answer that should carry a token but does not is not passed on.
    let mut request = login_request("/api/v1/login", r#"{"username":"no-token"}"#);
    request
        .headers_mut()
        .insert(header::ORIGIN, "http://localhost:8080".parse().unwrap());
    let (status, headers, body) = send(&app, request).await;
    assert_eq!(status, StatusCode::BAD_GATEWAY);
    assert_eq!(headers[header::ACCESS_CONTROL_ALLOW_ORIGIN], "*");
    assert_eq!(
        serde_json::from_slice::<Value>(&body).unwrap()["code"],
        "BACKEND_UNAVAILABLE"
    );
}

#[tokio::test]
async fn a_login_body_rust_cannot_read_starts_no_session() {
    let (url, recorded) = fake_backend().await;
    let app = gateway(&url);
    // Python's json module reads NaN and accepts the login; serde_json does not, so
    // Rust cannot ask for a token that lasts until the logout.
    let body = r#"{"username":"u","password":"p","remember":NaN}"#;
    let (status, _, answer) = send(&app, login_request("/api/v1/login", body)).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(
        !std::str::from_utf8(&answer)
            .unwrap()
            .contains("backend-token")
    );
    let requests = recorded.requests.lock().unwrap();
    assert_eq!(requests.last().unwrap().3, body);
}

#[tokio::test]
async fn a_login_body_above_the_limit_is_refused() {
    let (url, recorded) = fake_backend().await;
    let app = gateway(&url);
    let padding = " ".repeat(1024 * 1024);
    let request = login_request("/api/v1/login", &format!("{{}}{padding}"));
    let (status, _, _) = send(&app, request).await;
    assert_eq!(status, StatusCode::PAYLOAD_TOO_LARGE);
    assert!(recorded.paths().is_empty());
}

#[tokio::test]
async fn without_sessions_logins_and_tokens_pass_through_unchanged() {
    let (url, recorded) = fake_backend().await;
    recorded.sessions_disabled.store(true, Ordering::SeqCst);
    let app = gateway_without_sessions(&url);
    let body = r#"{"username":"u","password":"p","remember":true}"#;

    let (status, _, answer) = send(&app, login_request("/api/v1/login", body)).await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    assert!(answer.is_empty());
    {
        let requests = recorded.requests.lock().unwrap();
        assert_eq!(requests.last().unwrap().3, body);
    }
    send(&app, get("/api/v1/positions", Some("client-token"))).await;
    assert_eq!(
        recorded.last_authorization().as_deref(),
        Some("Bearer client-token")
    );

    // A token in the answer, which Python never sends without sessions, passes
    // through unchanged and is not cached.
    let body = r#"{"username":"token-without-sessions","password":"p"}"#;
    let (status, headers, answer) = send(&app, login_request("/api/v1/login", body)).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(headers[header::CACHE_CONTROL], "no-store");
    assert_eq!(
        serde_json::from_slice::<Value>(&answer).unwrap()["token"],
        backend_token(1)
    );
}

#[tokio::test]
async fn ported_routes_add_the_cors_headers_of_quart_cors() {
    let (url, _) = fake_backend().await;
    let app = gateway(&url);

    let mut request = loan_request(None);
    request
        .headers_mut()
        .insert(header::ORIGIN, "http://localhost:8080".parse().unwrap());
    let (status, headers, _) = send(&app, request).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert_eq!(headers[header::ACCESS_CONTROL_ALLOW_ORIGIN], "*");
    assert_eq!(
        headers[header::ACCESS_CONTROL_EXPOSE_HEADERS],
        "Content-Disposition"
    );
    assert!(!headers.contains_key(header::VARY));

    let (_, headers, _) = send(&app, loan_request(None)).await;
    assert_eq!(headers[header::VARY], "Origin");
    assert!(!headers.contains_key(header::ACCESS_CONTROL_ALLOW_ORIGIN));
}

#[tokio::test]
async fn other_methods_on_ported_paths_go_to_the_backend() {
    let (url, _) = fake_backend().await;
    let app = gateway(&url);
    let (status, headers, _) = send(
        &app,
        Request::builder()
            .method(Method::OPTIONS)
            .uri(LOAN)
            .header(header::ORIGIN, "http://localhost:8080")
            .header(header::ACCESS_CONTROL_REQUEST_METHOD, "POST")
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(headers["x-answered-by"], "backend");
    // The gateway adds no method list of its own.
    assert!(!headers.contains_key(header::ALLOW));

    let (status, headers, _) = send(&app, Request::get(LOAN).body(Body::empty()).unwrap()).await;
    assert_eq!(status, StatusCode::IM_A_TEAPOT);
    assert_eq!(headers["x-answered-by"], "backend");
    assert!(!headers.contains_key(header::ALLOW));
    // `HEAD` on a path Rust serves with `GET` goes to the backend too.
    let head = Request::head("/api/v1/settings?key=general")
        .body(Body::empty())
        .unwrap();
    let (status, headers, _) = send(&app, head).await;
    assert_eq!(status, StatusCode::IM_A_TEAPOT);
    assert_eq!(headers["x-answered-by"], "backend");
}

#[tokio::test]
async fn an_unreachable_backend_answers_502() {
    let (_closed, url) = closed_port();
    let app = gateway(&url);
    for request in [
        get("/api/v1/status", None),
        login_request("/api/v1/login", r#"{"username":"u","password":"p"}"#),
        logout(None),
    ] {
        let (status, _, body) = send(&app, request).await;
        assert_eq!(status, StatusCode::BAD_GATEWAY);
        assert_eq!(
            serde_json::from_slice::<Value>(&body).unwrap()["code"],
            "BACKEND_UNAVAILABLE"
        );
    }

    // A request that replaces the database is answered by Rust itself, with the
    // CORS headers, so that a cross-origin client can read the error.
    for path in ["/api/v1/cloud/backup/import", "/api/v1/change-password"] {
        let mut request = login_request(path, "{}");
        request
            .headers_mut()
            .insert(header::ORIGIN, "http://localhost:8080".parse().unwrap());
        let (status, headers, body) = send(&app, request).await;
        assert_eq!(status, StatusCode::BAD_GATEWAY, "{path}");
        assert_eq!(headers[header::ACCESS_CONTROL_ALLOW_ORIGIN], "*", "{path}");
        assert_eq!(
            serde_json::from_slice::<Value>(&body).unwrap()["code"],
            "BACKEND_UNAVAILABLE"
        );
    }

    // With sessions, ported routes check the session without the backend.
    let (status, _, _) = send(&app, loan_request(Some("made-up"))).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    // Without them, they cannot tell whether the data is unlocked.
    let app = gateway_without_sessions(&url);
    let (status, _, _) = send(&app, loan_request(None)).await;
    assert_eq!(status, StatusCode::BAD_GATEWAY);
}

/// A body that never ends and reports when the gateway first reads it.
struct PendingBody(Arc<tokio::sync::Notify>);

impl futures_core::Stream for PendingBody {
    type Item = Result<Bytes, std::io::Error>;

    fn poll_next(
        self: std::pin::Pin<&mut Self>,
        _cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Option<Self::Item>> {
        self.0.notify_one();
        std::task::Poll::Pending
    }
}

#[tokio::test]
async fn a_calculation_that_waits_too_long_for_a_permit_is_refused() {
    let (url, _) = fake_backend().await;
    let mut state = state(&url, SessionSettings::default(), TestClock::new());
    state.calculation_wait = Duration::from_millis(50);
    let app = router(state.clone());
    let token = login(&app).await;
    // Every permit is taken by calculations that are still running.
    let permits = state.calculations.available_permits();
    let running = Arc::clone(&state.calculations)
        .acquire_many_owned(u32::try_from(permits).unwrap())
        .await
        .unwrap();

    let (status, headers, body) = send(&app, loan_request(Some(&token))).await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(headers[header::RETRY_AFTER], "5");
    let body: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(body["code"], "SERVER_BUSY");

    drop(running);
    let (status, _, _) = send(&app, loan_request(Some(&token))).await;
    assert_eq!(status, StatusCode::OK);
}

#[tokio::test]
async fn a_slow_body_on_a_ported_route_is_cut_off() {
    let (url, _) = fake_backend().await;
    let app = gateway(&url);
    let token = login(&app).await;
    let first_read = Arc::new(tokio::sync::Notify::new());
    let request = Request::post(LOAN)
        .header(header::CONTENT_TYPE, "application/json")
        .header(header::AUTHORIZATION, format!("Bearer {token}"))
        .body(Body::from_stream(PendingBody(Arc::clone(&first_read))))
        .unwrap();
    let mut response = tokio::spawn(async move { app.oneshot(request).await.unwrap() });
    // Only once the gateway waits on the body may the clock skip ahead to the
    // deadline. The clock still runs in real time here, so the wait for that is
    // bounded.
    tokio::select! {
        () = first_read.notified() => {}
        answered = &mut response => {
            panic!("the gateway answered without reading the body: {:?}", answered.map(|r| r.status()))
        }
        () = tokio::time::sleep(Duration::from_secs(30)) => panic!("the gateway never read the body"),
    }
    tokio::time::pause();
    // The deadline is 60 seconds; the paused clock skips ahead to it.
    let response = tokio::time::timeout(Duration::from_secs(61), response)
        .await
        .expect("the gateway did not cut the body off")
        .unwrap();
    assert_eq!(response.status(), StatusCode::REQUEST_TIMEOUT);
}
