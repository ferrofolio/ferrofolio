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

use std::sync::Arc;

use axum::body::{Body, to_bytes};
use axum::http::{Request, StatusCode, header};
use core_app::ports::FixedClock;
use core_domain::dates::Date;
use serde_json::Value;
use server::app::{AppState, router};
use server::proxy::Backend;
use server::sessions::{SessionSettings, Sessions, SystemSessionClock};
use tower::ServiceExt;

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

/// Sends a request and returns the status and the JSON body, if any.
async fn answer(app: &axum::Router, request: Request<Body>) -> (StatusCode, Option<Value>) {
    let response = app.clone().oneshot(request).await.unwrap();
    let status = response.status();
    let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    (status, serde_json::from_slice::<Value>(&bytes).ok())
}

/// JSON equality where numbers compare by their exact value: Python writes `1e+16`
/// where Rust writes `1e16`, which is the same number. The sign of zero counts, as
/// clients can show it.
fn same_json(left: &Value, right: &Value) -> bool {
    match (left, right) {
        // Exact comparison of the number texts; it equates `1e+16` with `1e16`
        // without rounding either, whatever their size.
        (Value::Number(left), Value::Number(right)) => {
            let (left, right) = (left.to_string(), right.to_string());
            canonical_number(&left)
                .is_some_and(|canonical| Some(canonical) == canonical_number(&right))
        }
        (Value::Array(left), Value::Array(right)) => {
            left.len() == right.len()
                && left
                    .iter()
                    .zip(right)
                    .all(|(left, right)| same_json(left, right))
        }
        (Value::Object(left), Value::Object(right)) => {
            left.len() == right.len()
                && left
                    .iter()
                    .all(|(key, value)| right.get(key).is_some_and(|other| same_json(value, other)))
        }
        _ => left == right,
    }
}

/// A JSON number as sign, significant digits and power of ten, the same for every
/// spelling of the same value, keeping the sign of zero.
fn canonical_number(text: &str) -> Option<(bool, String, i64)> {
    let (negative, unsigned) = match text.strip_prefix('-') {
        Some(rest) => (true, rest),
        None => (false, text),
    };
    let (mantissa, exponent) = match unsigned.split_once(['e', 'E']) {
        Some((mantissa, exponent)) => (mantissa, exponent.parse::<i64>().ok()?),
        None => (unsigned, 0),
    };
    let (integral, fraction) = mantissa.split_once('.').unwrap_or((mantissa, ""));
    let digits = format!("{integral}{fraction}");
    let significant = digits.trim_start_matches('0');
    if significant.is_empty() {
        return Some((negative, String::new(), 0));
    }
    let trimmed = significant.trim_end_matches('0');
    let exponent = exponent - i64::try_from(fraction.len()).ok()?
        + i64::try_from(significant.len() - trimmed.len()).ok()?;
    Some((negative, trimmed.to_owned(), exponent))
}

#[test]
fn canonical_numbers_ignore_spelling_only() {
    assert_eq!(canonical_number("1e+16"), canonical_number("1e16"));
    assert_eq!(
        canonical_number("10000000000000000.0"),
        canonical_number("1e16")
    );
    assert_eq!(canonical_number("-0.0"), canonical_number("-0"));
    assert_ne!(canonical_number("-0.0"), canonical_number("0.0"));
    assert_eq!(
        canonical_number("1e+30"),
        canonical_number("1000000000000000000000000000000")
    );
    assert_ne!(
        canonical_number("9007199254740993"),
        canonical_number("9007199254740992.0")
    );
    assert_ne!(canonical_number("-1.5"), canonical_number("1.5"));
}

/// What a case requires of the Rust response, and whether it got it.
fn difference(case: &Value, status: StatusCode, body: &Option<Value>) -> Option<String> {
    let response = &case["response"];
    let status_code = u64::from(status.as_u16());
    let code = |body: &Option<Value>| body.as_ref().and_then(|body| body.get("code")).cloned();

    let (matches, expectation) = match case["compare"].as_str().unwrap() {
        "full" => (
            status_code == response["status"].as_u64().unwrap()
                && body
                    .as_ref()
                    .is_some_and(|body| same_json(body, &response["body"])),
            format!("{} {}", response["status"], response["body"]),
        ),
        "status_code" => (
            status_code == response["status"].as_u64().unwrap()
                && code(body)
                    == Some(response["body"].get("code").cloned().unwrap_or_else(|| {
                        panic!("{}: a status_code case needs a code", case["name"])
                    })),
            format!(
                "{} with code {}",
                response["status"], response["body"]["code"]
            ),
        ),
        "status" => (
            status_code == response["status"].as_u64().unwrap(),
            format!("{}", response["status"]),
        ),
        "deviation" => {
            let deviation = &case["deviation"];
            // Some deviations also state the exact body Rust must return.
            let body_matches = deviation
                .get("body")
                .is_none_or(|expected| body.as_ref().is_some_and(|body| same_json(body, expected)));
            (
                status_code == deviation["status"].as_u64().unwrap()
                    && code(body) == deviation.get("code").cloned()
                    && body_matches,
                format!(
                    "{} with code {}{} (deviation from Python: {})",
                    deviation["status"],
                    deviation["code"],
                    deviation
                        .get("body")
                        .map_or_else(String::new, |body| format!(" and body {body}")),
                    deviation["reason"]
                ),
            )
        }
        other => panic!("unknown comparison {other}"),
    };
    (!matches).then(|| {
        format!(
            "{}\n  expected {expectation}\n  got      {status_code} {}",
            case["name"].as_str().unwrap(),
            body.as_ref()
                .map_or_else(|| "<no JSON>".to_owned(), Value::to_string),
        )
    })
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
