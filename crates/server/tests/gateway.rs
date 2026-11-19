// SPDX-License-Identifier: AGPL-3.0-only

//! The gateway against a fake Python backend: forwarding to Python.

// Test helpers outside #[test] functions may panic on broken fixtures.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::Router;
use axum::body::{Body, Bytes, to_bytes};
use axum::extract::{ConnectInfo, Request, State};
use axum::http::{HeaderMap, Method, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::any;
use serde_json::{Value, json};
use server::app::{AppState, router};
use server::proxy::Backend;
use tower::ServiceExt;

/// The only bearer token the fake backend accepts.
const VALID_TOKEN: &str = "valid-session-token";

/// What the fake backend received.
#[derive(Debug, Default)]
struct Recorded {
    requests: Mutex<Vec<(Method, String, HeaderMap, Bytes)>>,
    /// Releases the second chunk of `/stream`.
    release_second_chunk: Arc<tokio::sync::Notify>,
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
}

/// A backend whose `/api/v1/status` answers `UNLOCKED` only for `VALID_TOKEN`.
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
                let authorized = parts.headers.get(header::AUTHORIZATION)
                    == Some(&format!("Bearer {VALID_TOKEN}").parse().unwrap());
                let status = if authorized { "UNLOCKED" } else { "LOCKED" };
                axum::Json(json!({"status": status, "apiSession": true})).into_response()
            }
            // Spellings that reach Python's login and signup handlers: uvicorn
            // decodes escapes and Quart collapses leading slashes and uppercases
            // the method.
            (
                method,
                "/api/v1/login" | "/api/v1/logi%6E" | "//api/v1/login" | "/%2Fapi/v1/signup",
            ) if method.as_str().eq_ignore_ascii_case("POST") => {
                axum::Json(json!({"token": VALID_TOKEN})).into_response()
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

fn gateway(backend_url: &str) -> Router {
    router(AppState::new(Backend::new(backend_url).unwrap()))
}

async fn send(app: &Router, request: Request<Body>) -> (StatusCode, HeaderMap, Bytes) {
    let response = app.clone().oneshot(request).await.unwrap();
    let status = response.status();
    let headers = response.headers().clone();
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    (status, headers, body)
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
async fn token_responses_are_not_cached() {
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
    for (method, path) in spellings {
        let (status, headers, _) = send(
            &app,
            Request::builder()
                .method(method.clone())
                .uri(path)
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(r#"{"username":"u","password":"p"}"#))
                .unwrap(),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{method} {path}");
        assert_eq!(
            headers[header::CACHE_CONTROL],
            "no-store",
            "{method} {path}"
        );
    }
    assert_eq!(recorded.paths(), spellings.map(|(_, path)| path.to_owned()));

    // Other responses keep the backend's headers.
    let (_, headers, _) = send(
        &app,
        Request::get("/api/v1/other").body(Body::empty()).unwrap(),
    )
    .await;
    assert!(!headers.contains_key(header::CACHE_CONTROL));
}

#[tokio::test]
async fn an_unreachable_backend_answers_502() {
    let (_closed, url) = closed_port();
    let app = gateway(&url);
    let (status, _, body) = send(
        &app,
        Request::get("/api/v1/status").body(Body::empty()).unwrap(),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_GATEWAY);
    assert_eq!(
        serde_json::from_slice::<Value>(&body).unwrap()["code"],
        "BACKEND_UNAVAILABLE"
    );
}
