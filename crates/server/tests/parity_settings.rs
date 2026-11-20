// SPDX-License-Identifier: AGPL-3.0-only

//! Parity of `GET /api/v1/settings` with the Python route, from the fixtures
//! `tests/parity/generate_db_route_fixtures.py` records against the fixture
//! databases, and the answers of the route while Rust and Python disagree about
//! the lock state.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod common;

use std::sync::Arc;

use axum::body::Body;
use axum::http::header::AUTHORIZATION;
use axum::http::{Request, StatusCode};
use common::database::{BACKEND_TOKEN, Harness, fixture};
use common::{answer, same_json};
use core_domain::settings::SettingsSection;
use infra_db::settings::base_config::BaseConfig;
use infra_db::settings::{last_update, mapper, parse_timestamp, resolve_section};
use jiff::tz::TimeZone;
use serde_json::{Value, json};

fn fixtures() -> Value {
    fixture("settings.json")
}

fn time_zone(fixtures: &Value) -> TimeZone {
    TimeZone::posix(fixtures["time_zone"].as_str().unwrap()).unwrap()
}

fn base(fixtures: &Value) -> BaseConfig {
    let last_update = fixtures["context"]["default_last_update"].as_str().unwrap();
    BaseConfig::new(last_update.to_owned())
}

/// A server with the fixture database `file`, set up like the recording.
async fn harness(fixtures: &Value, file: &str, sessions: bool) -> Harness {
    let (base, time_zone) = (base(fixtures), time_zone(fixtures));
    Harness::start(file, sessions, move |state| {
        state.settings_base = Arc::new(base);
        state.time_zone = time_zone;
    })
    .await
}

/// Replays every case with API sessions, and those with a valid session on
/// unlocked data, and those on locked data, also without them.
async fn replay(sessions: bool) -> (usize, Vec<String>) {
    let fixtures = fixtures();
    let (base, time_zone) = (base(&fixtures), time_zone(&fixtures));
    common::database::replay(&fixtures, sessions, move |state| {
        state.settings_base = Arc::new(base.clone());
        state.time_zone = time_zone.clone();
    })
    .await
}

#[tokio::test]
async fn the_settings_route_answers_like_the_python_backend() {
    let (replayed, failures) = replay(true).await;
    assert!(replayed > 30, "the fixtures look truncated");
    assert!(
        failures.is_empty(),
        "{} cases differ from Python:\n{}",
        failures.len(),
        failures.join("\n")
    );
}

#[tokio::test]
async fn without_api_sessions_too() {
    let (replayed, failures) = replay(false).await;
    assert!(replayed > 25, "the fixtures look truncated");
    assert!(
        failures.is_empty(),
        "{} cases differ from Python:\n{}",
        failures.len(),
        failures.join("\n")
    );
}

fn get_settings(token: Option<&str>) -> Request<Body> {
    let mut builder = Request::get("/api/v1/settings?key=general");
    if let Some(token) = token {
        builder = builder.header(AUTHORIZATION, format!("Bearer {token}"));
    }
    builder.body(Body::empty()).unwrap()
}

/// With API sessions, Python locks the data on its own when a restore fails and
/// keeps its sessions, as Rust keeps its own: a request with a valid session goes
/// to Python with the session's backend token, and Rust reads nothing.
#[tokio::test]
async fn passes_requests_on_while_python_has_locked_the_data_on_its_own() {
    let fixtures = fixtures();
    let harness = harness(&fixtures, "settings.db", true).await;
    let token = harness.login().await.unwrap();
    harness.python.set_unlocked(false);
    let (status, body) = answer(&harness.app, get_settings(Some(&token))).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        body.unwrap(),
        json!({
            "forwarded": "GET /api/v1/settings?key=general",
            "authorization": format!("Bearer {BACKEND_TOKEN}"),
        })
    );
}

/// Python locks the data on its own when a restore fails, and a login of another
/// user then unlocks that user's data: the sessions of the first user end in
/// Rust, so none of them reads the other user's database or reaches Python with
/// its backend token.
#[tokio::test]
async fn a_login_of_another_user_ends_the_sessions_of_the_first() {
    let fixtures = fixtures();
    let harness = harness(&fixtures, "settings.db", true).await;
    // The other user's database holds no settings, the first user's does, so the
    // answers tell the two databases apart.
    harness.add_user("other", "empty.db");
    let first = harness.login().await.unwrap();
    harness.python.set_unlocked(false);
    let second = harness.login_as("other").await.unwrap();
    assert!(!harness.state.sessions.is_valid(&{
        let mut headers = axum::http::HeaderMap::new();
        headers.insert(AUTHORIZATION, format!("Bearer {first}").parse().unwrap());
        headers
    }));
    let (status, body) = answer(&harness.app, get_settings(Some(&first))).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        body.unwrap(),
        json!({
            "forwarded": "GET /api/v1/settings?key=general",
            "authorization": format!("Bearer {first}"),
        })
    );
    // The second user's own session reads the second user's database.
    let (status, body) = answer(&harness.app, get_settings(Some(&second))).await;
    assert_eq!(status, StatusCode::OK);
    let body = body.unwrap();
    assert!(body.get("forwarded").is_none());
    assert_eq!(body["general"]["defaultCurrency"], "EUR");
}

/// A request Rust serves under the spelling of the route but passes on after all
/// reaches Python with the method and path the client sent.
#[tokio::test]
async fn passes_requests_on_as_the_client_sent_them() {
    let fixtures = fixtures();
    let harness = harness(&fixtures, "settings.db", false).await;
    harness.python.set_unlocked(true);
    let request = Request::builder()
        .method("get")
        .uri("//api/v1/%73ettings?key=general")
        .body(Body::empty())
        .unwrap();
    let (status, body) = answer(&harness.app, request).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        body.unwrap(),
        json!({"forwarded": "get //api/v1/%73ettings?key=general", "authorization": null})
    );
}

/// Without API sessions, Python can have the data unlocked while Rust has no
/// database open, after a logout that raced a login or a guest's backup import:
/// Python answers, with its own data.
#[tokio::test]
async fn passes_requests_on_while_rust_has_no_database_open() {
    let fixtures = fixtures();
    let harness = harness(&fixtures, "settings.db", false).await;
    harness.python.set_unlocked(true);
    let (status, body) = answer(&harness.app, get_settings(None)).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        body.unwrap(),
        json!({"forwarded": "GET /api/v1/settings?key=general", "authorization": null})
    );
}

/// While a request that replaces or rekeys the database is with Python, Rust's
/// connection is closed and Python answers; afterwards Rust reads again.
#[tokio::test]
async fn passes_requests_on_while_python_replaces_the_database() {
    let fixtures = fixtures();
    for sessions in [true, false] {
        let harness = harness(&fixtures, "settings.db", sessions).await;
        let token = harness.login().await;
        let guard = harness.state.sessions.close_for_replacement().await;
        let (_, body) = answer(&harness.app, get_settings(token.as_deref())).await;
        assert!(
            body.unwrap().get("forwarded").is_some(),
            "sessions {sessions}"
        );
        drop(guard);
        let (status, body) = answer(&harness.app, get_settings(token.as_deref())).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body.unwrap()["general"]["defaultCurrency"], "USD");
    }
}

/// A status Python cannot give fails the request, as for the calculators.
#[tokio::test]
async fn fails_when_python_cannot_report_the_status() {
    let fixtures = fixtures();
    let harness = harness(&fixtures, "settings.db", false).await;
    let mut state = harness.state.clone();
    state.backend = server::proxy::Backend::new("http://127.0.0.1:9").unwrap();
    let app = server::app::router(state);
    let (status, body) = answer(&app, get_settings(None)).await;
    assert_eq!(status, StatusCode::BAD_GATEWAY);
    assert_eq!(body.unwrap()["code"], "BACKEND_UNAVAILABLE");
}

#[test]
fn reads_stored_sections_like_python() {
    let fixtures = fixtures();
    let base = base(&fixtures);
    let entries = fixtures["functions"]["sections"].as_array().unwrap();
    assert!(entries.len() > 100);
    let mut failures = Vec::new();
    for entry in entries {
        let section = SettingsSection::from_value(entry["section"].as_str().unwrap()).unwrap();
        let text = entry["stored"].as_str().unwrap();
        let stored = mapper::deserialize_section(text.as_bytes());
        let valid = stored
            .as_ref()
            .filter(|value| !value.is_null())
            .is_some_and(|value| mapper::parse_section(section, value).is_ok());
        let value = serde_json::to_value(resolve_section(section, stored.as_ref(), &base)).unwrap();
        let expected = entry.get("deviation").unwrap_or(entry);
        if expected["valid"] != valid || !same_json(&expected["value"], &value) {
            failures.push(format!(
                "{} {text}\n  expected {} {}\n  got      {valid} {value}",
                entry["section"], expected["valid"], expected["value"]
            ));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn parses_stored_times_like_python() {
    let fixtures = fixtures();
    let time_zone = time_zone(&fixtures);
    let mut failures = Vec::new();
    for entry in fixtures["functions"]["timestamps"].as_array().unwrap() {
        // Only text is a time; the repository passes nothing else on.
        let got = match entry["stored"].as_str() {
            Some(text) => match parse_timestamp(text, &time_zone) {
                Ok(parsed) => json!({"parsed": parsed.map(|time| time.isoformat())}),
                Err(error) => json!({"error": error.to_string()}),
            },
            None => json!({"parsed": null}),
        };
        let expected = match entry.get("error") {
            Some(error) => json!({"error": error}),
            None => json!({"parsed": entry["parsed"]}),
        };
        if got != expected {
            failures.push(format!(
                "{}: expected {expected}, got {got}",
                entry["stored"]
            ));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn picks_the_last_update_like_python() {
    let fixtures = fixtures();
    let time_zone = time_zone(&fixtures);
    let base = base(&fixtures);
    for entry in fixtures["functions"]["last_updates"].as_array().unwrap() {
        let stored = entry["stored"].as_array().unwrap();
        let got = match last_update(stored.iter().map(Value::as_str), &time_zone) {
            Ok(Some(time)) => json!({"last_update": time.isoformat()}),
            Ok(None) => json!({"last_update": base.last_update()}),
            Err(error) => json!({"error": error.to_string()}),
        };
        let expected = match entry.get("error") {
            Some(error) => json!({"error": error}),
            None => json!({"last_update": entry["last_update"]}),
        };
        assert_eq!(got, expected, "{}", entry["stored"]);
    }
}
