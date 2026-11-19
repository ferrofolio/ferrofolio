// SPDX-License-Identifier: AGPL-3.0-only

//! Replays the status scenarios recorded from the Python backend by
//! `tests/parity/generate_status_fixtures.py` against the Rust router.
//!
//! A fake backend answers each login, signup and logout with the status code
//! Python answered and writes the `profiles.json` Python left behind, as Python
//! does before it answers. Python created each user's database with the scenario's
//! password; an empty file stands in for it, which SQLCipher opens with any key, so
//! that Rust unlocks the data whenever Python did. Opening a real database is tested
//! against the fixture of `infra-db` in `tests/gateway.rs`.

// Test helpers outside #[test] functions may panic on broken fixtures.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use std::collections::HashMap;
use std::ffi::OsString;
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::symlink;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use axum::Router;
use axum::body::{Body, to_bytes};
use axum::extract::{Request, State};
use axum::http::{HeaderValue, Method, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::any;
use core_app::ports::FixedClock;
use core_domain::dates::Date;
use serde_json::{Value, json};
use server::app::{AppState, router};
use server::config::LogLevel;
use server::gateway::unlock_at_startup;
use server::proxy::Backend;
use server::pypath;
use server::routes::status::{
    ServerOptions, StatusConfig, VERSION, platform_type, platform_version, trusts_gateway,
};
use server::sessions::{Credentials, SessionSettings, Sessions, SystemSessionClock};
use server::user_data::DataDir;
use tower::ServiceExt;

const FIXTURES: &str = include_str!("fixtures/status_parity.json");

fn fixtures() -> Value {
    serde_json::from_str(FIXTURES).unwrap()
}

/// A new directory with the tree the generator starts from, without symbolic
/// links in its own path.
fn make_tree() -> PathBuf {
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    let root = std::env::temp_dir().join(format!(
        "ferrofolio-status-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::create_dir_all(root.join("real/sub")).unwrap();
    let root = std::fs::canonicalize(root).unwrap();
    symlink("real", root.join("link")).unwrap();
    symlink(root.join("real"), root.join("absolute-link")).unwrap();
    symlink("link/sub", root.join("chain")).unwrap();
    symlink("loop-b", root.join("loop-a")).unwrap();
    symlink("loop-a", root.join("loop-b")).unwrap();
    std::fs::create_dir(root.join("cwd")).unwrap();
    symlink("../real", root.join("cwd/up")).unwrap();
    root
}

fn with_root(text: &str, root: &Path) -> String {
    text.replace("{root}", root.to_str().unwrap())
}

/// The expected answer with the values of this machine and build in place of
/// the placeholders.
fn expected(value: &Value, root: &Path) -> Value {
    match value {
        Value::String(text) => match text.as_str() {
            "{version}" => Value::from(VERSION),
            "{platform_type}" => platform_type().map_or(Value::Null, Value::from),
            "{platform_version}" => platform_version().map_or(Value::Null, Value::from),
            _ => Value::from(with_root(text, root)),
        },
        Value::Array(items) => items.iter().map(|item| expected(item, root)).collect(),
        Value::Object(fields) => fields
            .iter()
            .map(|(key, item)| (key.clone(), expected(item, root)))
            .collect(),
        other => other.clone(),
    }
}

/// How the fake backend answers the next login, signup or logout.
#[derive(Debug)]
struct Answer {
    status: StatusCode,
    profiles: String,
}

#[derive(Debug)]
struct Fake {
    data_dir: PathBuf,
    answer: Mutex<Option<Answer>>,
    logins: AtomicUsize,
}

/// A backend that answers every request with the next `Answer`, after it has
/// written `profiles.json` and a database for every profile in it.
async fn fake_backend(data_dir: PathBuf) -> (String, Arc<Fake>) {
    let fake = Arc::new(Fake {
        data_dir,
        answer: Mutex::new(None),
        logins: AtomicUsize::new(0),
    });

    async fn answer(State(fake): State<Arc<Fake>>, request: Request) -> Response {
        let path = request.uri().path().to_owned();
        let Some(answer) = fake.answer.lock().unwrap().take() else {
            panic!("the gateway forwarded {} {path}", request.method());
        };
        std::fs::write(fake.data_dir.join("profiles.json"), &answer.profiles).unwrap();
        let profiles: Value = serde_json::from_str(&answer.profiles).unwrap();
        for profile in profiles["profiles"].as_array().unwrap() {
            let directory = fake
                .data_dir
                .join("profiles")
                .join(profile["id"].as_str().unwrap());
            std::fs::create_dir_all(&directory).unwrap();
            let database = directory.join("data.db");
            if !database.exists() {
                std::fs::write(database, b"").unwrap();
            }
        }
        if answer.status == StatusCode::OK {
            let n = fake.logins.fetch_add(1, Ordering::SeqCst) + 1;
            axum::Json(json!({"token": format!("backend-token-{n}")})).into_response()
        } else {
            (answer.status, axum::Json(json!({"message": "recorded"}))).into_response()
        }
    }

    let app = Router::new()
        .route("/{*path}", any(answer))
        .with_state(Arc::clone(&fake));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    (format!("http://{addr}"), fake)
}

fn log_level(value: &Value) -> LogLevel {
    match value.as_str().unwrap() {
        "NONE" => LogLevel::None,
        "DEBUG" => LogLevel::Debug,
        "INFO" => LogLevel::Info,
        "WARNING" => LogLevel::Warning,
        "ERROR" => LogLevel::Error,
        "CRITICAL" => LogLevel::Critical,
        other => panic!("unknown log level {other}"),
    }
}

/// The state of the gateway for a scenario, with the status it configures.
fn gateway(scenario: &Value, backend_url: &str, root: &Path) -> AppState {
    let options = &scenario["options"];
    let data_dir = PathBuf::from(with_root(options["data_dir"].as_str().unwrap(), root));
    let server_options = ServerOptions {
        data_dir: data_dir.clone(),
        port: u16::try_from(options["port"].as_u64().unwrap()).unwrap(),
        log_level: log_level(&options["log_level"]),
        log_dir: PathBuf::from(with_root(options["log_dir"].as_str().unwrap(), root)),
        log_file_level: options["log_file_level"]
            .as_str()
            .map(|_| log_level(&options["log_file_level"])),
        third_party_log_level: log_level(&options["third_party_log_level"]),
    };
    let environment: Vec<(OsString, OsString)> = scenario["environment"]
        .as_object()
        .into_iter()
        .flatten()
        .map(|(key, value)| (key.into(), value.as_str().unwrap().into()))
        .collect();
    let settings = SessionSettings {
        enabled: scenario["api_session"].as_bool().unwrap(),
        remember_ttl: None,
    };
    let mut state = AppState::new(
        Backend::new(backend_url).unwrap(),
        Arc::new(FixedClock(Date::new(2026, 11, 18).unwrap())),
        Sessions::new(settings, Arc::new(SystemSessionClock)),
    );
    state.data_dir = Some(DataDir::new(data_dir));
    let config = StatusConfig::new(
        server_options,
        environment,
        platform_type(),
        platform_version(),
    );
    state.status = Some(Arc::new(config.unwrap()));
    state
}

#[tokio::test]
async fn the_status_answers_like_the_python_backend() {
    for scenario in fixtures()["scenarios"].as_array().unwrap() {
        let name = scenario["name"].as_str().unwrap();
        let root = make_tree();
        let data_dir = PathBuf::from(with_root(
            scenario["options"]["data_dir"].as_str().unwrap(),
            &root,
        ));
        // Python creates the data directory and an empty profiles.json at startup.
        std::fs::create_dir_all(data_dir.join("profiles")).unwrap();
        std::fs::write(
            data_dir.join("profiles.json"),
            scenario["profiles"].as_str().unwrap(),
        )
        .unwrap();
        let (url, fake) = fake_backend(data_dir).await;
        let mut app = router(gateway(scenario, &url, &root));
        let mut tokens: HashMap<String, String> = HashMap::new();

        for (index, step) in scenario["steps"].as_array().unwrap().iter().enumerate() {
            let context = format!("{name}, step {index}: {step}");
            let kind = step["request"].as_str().unwrap();
            if kind == "restart" {
                // A new server on the same data directory, after Python's automatic
                // login with USERNAME and PASSWORD.
                std::fs::write(
                    fake.data_dir.join("profiles.json"),
                    step["profiles"].as_str().unwrap(),
                )
                .unwrap();
                let state = gateway(scenario, &url, &root);
                let login = &step["startup_login"];
                let credentials = Credentials {
                    username: login["username"].as_str().unwrap().to_owned(),
                    password: login["password"].as_str().unwrap().to_owned(),
                };
                unlock_at_startup(&state, credentials).await;
                app = router(state);
                continue;
            }
            let mut builder = if kind == "status" {
                Request::get("/api/v1/status")
            } else {
                Request::post(format!("/api/v1/{kind}"))
                    .header(header::CONTENT_TYPE, "application/json")
            };
            if let Some(session) = step["session"].as_str() {
                let token = tokens
                    .get(session)
                    .cloned()
                    .unwrap_or_else(|| format!("{session}-unknown-token"));
                builder = builder.header(header::AUTHORIZATION, format!("Bearer {token}"));
            }
            for pair in step["headers"].as_array().into_iter().flatten() {
                let value = pair[1].as_str().unwrap();
                // Quart's test client sends the text as UTF-8.
                let value = HeaderValue::from_bytes(value.as_bytes()).unwrap();
                builder = builder.header(pair[0].as_str().unwrap(), value);
            }
            let recorded = &step["response"];
            let recorded_status =
                StatusCode::from_u16(u16::try_from(recorded["status"].as_u64().unwrap()).unwrap())
                    .unwrap();
            let body = if kind == "status" {
                Body::empty()
            } else {
                *fake.answer.lock().unwrap() = Some(Answer {
                    status: recorded_status,
                    profiles: step["profiles"].as_str().unwrap().to_owned(),
                });
                Body::from(step["body"].to_string())
            };

            let response = app
                .clone()
                .oneshot(builder.body(body).unwrap())
                .await
                .unwrap();
            let status = response.status();
            let content_type = response.headers().get(header::CONTENT_TYPE).cloned();
            let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
            assert_eq!(status, recorded_status, "{context}");
            if kind == "status" {
                assert_eq!(content_type.unwrap(), "application/json", "{context}");
                let answer: Value = serde_json::from_slice(&bytes).unwrap();
                assert_eq!(answer, expected(&recorded["body"], &root), "{context}");
                continue;
            }
            assert!(
                fake.answer.lock().unwrap().is_none(),
                "{context}: the request did not reach the backend"
            );
            if status == StatusCode::OK {
                let answer: Value = serde_json::from_slice(&bytes).unwrap();
                let name = step["token"].as_str().unwrap().to_owned();
                tokens.insert(name, answer["token"].as_str().unwrap().to_owned());
            }
        }
        std::fs::remove_dir_all(&root).unwrap();
    }
}

#[test]
fn paths_are_written_and_resolved_like_pathlib() {
    let root = make_tree();
    let cwd = root.join("cwd");
    let cwd = cwd.as_os_str().as_bytes();
    let text = |bytes: Option<Vec<u8>>| String::from_utf8(bytes.unwrap()).unwrap();
    for row in fixtures()["paths"].as_array().unwrap() {
        let raw = with_root(row["raw"].as_str().unwrap(), &root);
        let expect = |key: &str| with_root(row[key].as_str().unwrap(), &root);
        let raw = raw.as_bytes();
        assert_eq!(text(Some(pypath::normalize(raw))), expect("str"), "{row}");
        assert_eq!(
            text(pypath::absolute(raw, Some(cwd))),
            expect("absolute"),
            "{row}"
        );
        assert_eq!(
            text(pypath::resolve(raw, Some(cwd))),
            expect("resolve"),
            "{row}"
        );
    }
    std::fs::remove_dir_all(&root).unwrap();
}

#[test]
fn trusts_the_gateway_like_uvicorn() {
    for row in fixtures()["forwarded_allow_ips"].as_array().unwrap() {
        assert_eq!(
            trusts_gateway(row["value"].as_str()),
            row["trusted"].as_bool().unwrap(),
            "{row}"
        );
    }
}

/// A backend that records the paths it receives and answers `418`.
async fn recording_backend() -> (String, Arc<Mutex<Vec<(Method, String)>>>) {
    let seen = Arc::new(Mutex::new(Vec::new()));
    let recorder = Arc::clone(&seen);
    let app = Router::new().fallback(move |request: Request| {
        let recorder = Arc::clone(&recorder);
        async move {
            recorder
                .lock()
                .unwrap()
                .push((request.method().clone(), request.uri().path().to_owned()));
            StatusCode::IM_A_TEAPOT
        }
    });
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    (format!("http://{addr}"), seen)
}

fn status_gateway(backend_url: &str, configured: bool, settings: SessionSettings) -> Router {
    let root = std::env::temp_dir();
    let mut state = AppState::new(
        Backend::new(backend_url).unwrap(),
        Arc::new(FixedClock(Date::new(2026, 11, 18).unwrap())),
        Sessions::new(settings, Arc::new(SystemSessionClock)),
    );
    if configured {
        let options = ServerOptions {
            data_dir: root.clone(),
            port: 7592,
            log_level: LogLevel::Info,
            log_dir: root.join("logs"),
            log_file_level: None,
            third_party_log_level: LogLevel::Error,
        };
        let config = StatusConfig::new(options, Vec::new(), platform_type(), platform_version());
        state.status = config.map(Arc::new);
    }
    router(state)
}

#[tokio::test]
async fn every_spelling_of_the_status_is_served_by_rust_and_head_by_the_backend() {
    let (url, seen) = recording_backend().await;
    let app = status_gateway(&url, true, SessionSettings::default());
    for path in [
        "/api/v1/status",
        "//api/v1/status",
        "/api/v1/%73tatus",
        "/api/v1/status?x=1",
    ] {
        let method = Method::from_bytes(b"get").unwrap();
        let request = Request::builder()
            .method(method)
            .uri(path)
            .header(header::HOST, "localhost")
            .header(header::ORIGIN, "https://app.example.org")
            .body(Body::empty())
            .unwrap();
        let response = app.clone().oneshot(request).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK, "{path}");
        // quart-cors adds its headers to the answer.
        assert_eq!(
            response.headers()[header::ACCESS_CONTROL_ALLOW_ORIGIN],
            "*",
            "{path}"
        );
        let body: Value =
            serde_json::from_slice(&to_bytes(response.into_body(), usize::MAX).await.unwrap())
                .unwrap();
        assert_eq!(body["status"], "LOCKED", "{path}");
        assert_eq!(body["server"]["publicUrl"], "http://localhost", "{path}");
    }
    assert!(seen.lock().unwrap().is_empty());

    for method in [Method::HEAD, Method::OPTIONS, Method::POST] {
        let request = Request::builder()
            .method(method.clone())
            .uri("/api/v1/status")
            .body(Body::empty())
            .unwrap();
        let response = app.clone().oneshot(request).await.unwrap();
        assert_eq!(response.status(), StatusCode::IM_A_TEAPOT, "{method}");
    }
    assert_eq!(seen.lock().unwrap().len(), 3);
}

#[tokio::test]
async fn with_remote_feature_flags_the_backend_answers_the_status() {
    let (url, seen) = recording_backend().await;
    let app = status_gateway(&url, false, SessionSettings::default());
    let request = Request::get("/api/v1/status").body(Body::empty()).unwrap();
    let response = app.oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::IM_A_TEAPOT);
    assert_eq!(
        *seen.lock().unwrap(),
        [(Method::GET, "/api/v1/status".to_owned())]
    );

    let options = ServerOptions {
        data_dir: PathBuf::from("/data"),
        port: 7592,
        log_level: LogLevel::Info,
        log_dir: PathBuf::from("/data/logs"),
        log_file_level: None,
        third_party_log_level: LogLevel::Error,
    };
    let environment = [(
        OsString::from("FEATURE_FLAG_URL"),
        OsString::from("https://flags.example.org/flags.json"),
    )];
    assert!(StatusConfig::new(options, environment, None, None).is_none());
}

#[tokio::test]
async fn without_api_sessions_the_backend_answers_the_status() {
    // Python alone knows then whether the data is unlocked: it also unlocks and
    // locks it without a login the gateway passes on.
    let (url, seen) = recording_backend().await;
    let settings = SessionSettings {
        enabled: false,
        remember_ttl: None,
    };
    let app = status_gateway(&url, true, settings);
    let request = Request::get("/api/v1/status")
        .header(header::AUTHORIZATION, "Bearer any")
        .body(Body::empty())
        .unwrap();
    let response = app.oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::IM_A_TEAPOT);
    assert_eq!(
        *seen.lock().unwrap(),
        [(Method::GET, "/api/v1/status".to_owned())]
    );
}
