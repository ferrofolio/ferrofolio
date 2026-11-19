// SPDX-License-Identifier: AGPL-3.0-only

//! Parity of the ported calculator routes with the Python implementation.
//!
//! The fixtures are recorded from the Python routes by
//! `tests/parity/generate_calculation_fixtures.py`; regenerate them when the Python
//! calculators change. Every case must produce the same status and body here.

// Test helpers outside #[test] functions may panic on broken fixtures.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod common;

use std::sync::Arc;

use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use common::{answer, difference, same_json};
use core_app::ports::FixedClock;
use core_domain::dates::Date;
use serde_json::Value;
use server::app::{AppState, router};
use server::proxy::Backend;
use server::sessions::{SessionSettings, Sessions, SystemSessionClock};

const FIXTURES: &str = include_str!("fixtures/calculations_parity.json");
/// What a client sends once its session has ended with a logout, which locks the
/// data: a token Rust no longer knows.
const ENDED_SESSION_TOKEN: &str = "parity-ended-session-token";

/// The gateway with a backend that only accepts logins, and the token of a session
/// started through it. Ported routes check the session in Rust.
async fn app(today: Date) -> (axum::Router, String) {
    let backend = axum::Router::new().route(
        "/api/v1/login",
        axum::routing::post(|| async { axum::Json(serde_json::json!({ "token": "backend" })) }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    tokio::spawn(async move { axum::serve(listener, backend).await.unwrap() });
    let sessions = Sessions::new(SessionSettings::default(), Arc::new(SystemSessionClock));
    let app = router(AppState::new(
        Backend::new(&url).unwrap(),
        Arc::new(FixedClock(today)),
        sessions,
    ));
    let login = Request::post("/api/v1/login")
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(
            r#"{"username":"parity","password":"parity-password"}"#,
        ))
        .unwrap();
    let (status, body) = answer(&app, login).await;
    assert_eq!(status, StatusCode::OK);
    let token = body.unwrap()["token"].as_str().unwrap().to_owned();
    (app, token)
}

/// Whether a case sends a session token, and whether its data is unlocked, by
/// default exactly when it does.
fn session_state(case: &Value) -> (bool, bool) {
    let session = case.get("session") != Some(&Value::Bool(false));
    let unlocked = case
        .get("unlocked")
        .and_then(Value::as_bool)
        .unwrap_or(session);
    (session, unlocked)
}

/// The request of a case. A case with a session against unlocked data sends
/// `token`, the token of a valid session; one against locked data sends the token
/// of a session that has ended.
fn request(case: &Value, token: &str) -> Request<Body> {
    let path = case["path"].as_str().unwrap();
    let (content_type, body) = match (case.get("raw_body"), case.get("body")) {
        (Some(raw), _) => {
            let padding = case.get("pad_bytes").and_then(Value::as_u64).unwrap_or(0);
            let padding = " ".repeat(usize::try_from(padding).unwrap());
            (
                case["content_type"].as_str().unwrap().to_owned(),
                format!("{}{padding}", raw.as_str().unwrap()),
            )
        }
        (None, Some(body)) => (
            case.get("content_type")
                .and_then(Value::as_str)
                .unwrap_or("application/json")
                .to_owned(),
            body.to_string(),
        ),
        (None, None) => panic!("case without body"),
    };
    let mut builder = Request::post(path).header(header::CONTENT_TYPE, content_type);
    // Cases recorded without a session reach Rust without a token.
    match session_state(case) {
        (true, true) => builder = builder.header(header::AUTHORIZATION, format!("Bearer {token}")),
        (true, false) => {
            builder = builder.header(
                header::AUTHORIZATION,
                format!("Bearer {ENDED_SESSION_TOKEN}"),
            );
        }
        (false, _) => {}
    }
    builder.body(Body::from(body)).unwrap()
}

#[tokio::test]
async fn calculator_routes_answer_like_the_python_backend() {
    let fixtures: Value = serde_json::from_str(FIXTURES).unwrap();
    let today = fixtures["today"].as_str().unwrap().parse::<Date>().unwrap();
    let cases = fixtures["cases"].as_array().unwrap();
    assert!(cases.len() > 200, "the fixtures look truncated");

    let (app, token) = app(today).await;
    let mut failures = Vec::new();
    for case in cases {
        let (status, body) = answer(&app, request(case, &token)).await;
        if let Some(message) = difference(case, status, &body) {
            failures.push(message);
        }
        // A deviation that names an equivalent request: Rust must answer that one
        // the same way, for example the same amount sent as a string.
        if let Some(equivalent) = case
            .get("deviation")
            .and_then(|deviation| deviation.get("equivalent_request"))
        {
            let mut equivalent_case = case.clone();
            equivalent_case["body"] = equivalent.clone();
            if let Some(object) = equivalent_case.as_object_mut() {
                object.remove("raw_body");
                object.remove("content_type");
            }
            let (equivalent_status, equivalent_body) =
                answer(&app, request(&equivalent_case, &token)).await;
            if let Some(message) = difference(case, equivalent_status, &equivalent_body) {
                failures.push(format!("equivalent request of {message}"));
            }
            // And both answers must be the same, not only match the recorded parts.
            let same_body = match (&body, &equivalent_body) {
                (Some(body), Some(equivalent)) => same_json(body, equivalent),
                (None, None) => true,
                _ => false,
            };
            if status != equivalent_status || !same_body {
                failures.push(format!(
                    "{}\n  the equivalent request was answered differently",
                    case["name"].as_str().unwrap()
                ));
            }
        }
    }
    assert!(
        failures.is_empty(),
        "{} of {} cases differ from Python:\n{}",
        failures.len(),
        cases.len(),
        failures.join("\n")
    );
}
