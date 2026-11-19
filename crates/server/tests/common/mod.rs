// SPDX-License-Identifier: AGPL-3.0-only

//! What the parity tests share: comparing Rust's answers with the answers recorded
//! from Python, and the harness for routes that read the database.

// Each test crate uses a part of it; helpers may panic on broken fixtures.
#![allow(
    dead_code,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

pub mod database;

use axum::body::{Body, to_bytes};
use axum::http::{Request, StatusCode};
use serde_json::Value;
use tower::ServiceExt;

/// Sends a request and returns the status and the JSON body, if any.
pub async fn answer(app: &axum::Router, request: Request<Body>) -> (StatusCode, Option<Value>) {
    let response = app.clone().oneshot(request).await.unwrap();
    let status = response.status();
    let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    (status, serde_json::from_slice::<Value>(&bytes).ok())
}

/// JSON equality where numbers compare by their exact value: Python writes `1e+16`
/// where Rust writes `1e16`, which is the same number. The sign of zero counts, as
/// clients can show it.
pub fn same_json(left: &Value, right: &Value) -> bool {
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
pub fn canonical_number(text: &str) -> Option<(bool, String, i64)> {
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
pub fn difference(case: &Value, status: StatusCode, body: &Option<Value>) -> Option<String> {
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
