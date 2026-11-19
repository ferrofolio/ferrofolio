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
use server::sessions::{SessionClock, SessionSettings, Sessions};
use server::user_data::DataDir;
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

/// A data directory of its own with a `profiles.json` that names `FIXTURE_USER`
/// and a copy of the fixture database as that user's.
fn data_dir() -> DataDir {
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    let directory = std::env::temp_dir().join(format!(
        "ferrofolio-gateway-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    let profile = directory.join("profiles").join(FIXTURE_USER_ID);
    std::fs::create_dir_all(&profile).unwrap();
    std::fs::copy(FIXTURE_DATABASE, profile.join("data.db")).unwrap();
    let profiles = json!({
        "last_logged": FIXTURE_USER_ID,
        "profiles": [{"id": FIXTURE_USER_ID, "name": FIXTURE_USER}],
    });
    std::fs::write(directory.join("profiles.json"), profiles.to_string()).unwrap();
    DataDir::new(directory)
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
                    return axum::Json(json!({"status": "UNLOCKED", "apiSession": false}))
                        .into_response();
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
                match login["username"].as_str() {
                    // Without API sessions Python answers a login without a token.
                    _ if recorded.sessions_disabled.load(Ordering::SeqCst) => {
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
async fn without_sessions_ported_routes_need_the_data_a_login_unlocked() {
    let (url, recorded) = fake_backend().await;
    recorded.sessions_disabled.store(true, Ordering::SeqCst);
    let settings = SessionSettings {
        enabled: false,
        remember_ttl: None,
    };
    let mut state = state(&url, settings, TestClock::new());
    state.data_dir = Some(data_dir());
    let app = router(state.clone());

    let (status, _, _) = send(&app, loan_request(None)).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);

    let request = login_request("/api/v1/login", &fixture_login(FIXTURE_PASSWORD));
    let (status, _, _) = send(&app, request).await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    assert!(state.sessions.is_unlocked());
    let (status, _, body) = send(&app, loan_request(None)).await;
    assert_eq!(status, StatusCode::OK);
    let result: Value = serde_json::from_slice(&body).unwrap();
    assert!(result["current_installment_payment"].is_number());

    let (status, _, _) = send(&app, logout(None)).await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    assert!(!state.sessions.is_unlocked());
    let (status, _, _) = send(&app, loan_request(None)).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    // Neither the session check nor the calculation reached the backend.
    assert_eq!(recorded.paths(), ["/api/v1/login", "/api/v1/logout"]);
}

#[tokio::test]
async fn a_login_unlocks_the_database_until_the_logout() {
    let (url, _) = fake_backend().await;
    let mut state = state(&url, SessionSettings::default(), TestClock::new());
    state.data_dir = Some(data_dir());
    let app = router(state.clone());

    let request = login_request("/api/v1/login", &fixture_login(FIXTURE_PASSWORD));
    let (status, _, body) = send(&app, request).await;
    assert_eq!(status, StatusCode::OK);
    assert!(state.sessions.is_unlocked());
    let token = serde_json::from_slice::<Value>(&body).unwrap()["token"]
        .as_str()
        .unwrap()
        .to_owned();

    let (status, _, _) = send(&app, logout(Some(&token))).await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    assert!(!state.sessions.is_unlocked());
}

#[tokio::test]
async fn a_login_rust_cannot_unlock_with_still_starts_a_session() {
    let (url, _) = fake_backend().await;
    let mut state = state(&url, SessionSettings::default(), TestClock::new());
    state.data_dir = Some(data_dir());
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

    // With sessions, ported routes check the session without the backend.
    let (status, _, _) = send(&app, loan_request(Some("made-up"))).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    // Without them, no login has unlocked the data.
    let app = gateway_without_sessions(&url);
    let (status, _, _) = send(&app, loan_request(None)).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
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
