// SPDX-License-Identifier: AGPL-3.0-only

//! Parity of `GET /api/v1/labeling/rules` with the Python route, from the fixtures
//! `tests/parity/generate_db_route_fixtures.py` records against the fixture
//! databases, and of the reading of stored conditions for the values recorded
//! from Python.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod common;

use common::database::fixture;
use infra_db::labeling_rules::deserialize_conditions;
use serde_json::Value;
use server::routes::labeling_rules::conditions_json;

fn fixtures() -> Value {
    fixture("labeling_rules.json")
}

fn assert_replays(sessions: bool, replayed: usize, failures: &[String]) {
    assert!(replayed >= 4, "the fixtures look truncated ({sessions})");
    assert!(
        failures.is_empty(),
        "{} cases differ from Python:\n{}",
        failures.len(),
        failures.join("\n")
    );
}

#[tokio::test]
async fn the_labeling_rules_route_answers_like_the_python_backend() {
    let (replayed, failures) = common::database::replay(&fixtures(), true, |_| {}).await;
    assert_replays(true, replayed, &failures);
}

#[tokio::test]
async fn without_api_sessions_too() {
    let (replayed, failures) = common::database::replay(&fixtures(), false, |_| {}).await;
    assert_replays(false, replayed, &failures);
}

/// Rust reads every form of stored conditions that Python's writer produces, and
/// the other recorded forms it reads, exactly as Python does. Where Python fails,
/// or coerces a form Rust does not read, Rust reports it unreadable, so that the
/// route passes the request on: it never answers where Python would not.
#[test]
fn stored_conditions_are_read_like_python_reads_them() {
    let fixtures = fixtures();
    let recorded = fixtures["functions"]["conditions"].as_array().unwrap();
    assert!(recorded.len() > 40, "the fixtures look truncated");
    let mut failures = Vec::new();
    for entry in recorded {
        let raw = entry[0].as_str().unwrap();
        let python = &entry[1];
        let kind = entry[2].as_str().unwrap();
        let rust = deserialize_conditions(raw)
            .map(|groups| Value::Array(groups.iter().map(conditions_json).collect()));
        match (&rust, python.get("ok")) {
            (Ok(read), Some(expected)) if !common::same_json(read, expected) => {
                failures.push(format!("{raw:?}: Python {expected}, Rust {read}"));
            }
            (Ok(read), None) => {
                failures.push(format!("{raw:?}: Python fails, Rust answers {read}"));
            }
            (Err(reason), _) if kind != "python" => {
                failures.push(format!("{raw:?} ({kind}): Rust cannot read it: {reason}"));
            }
            _ => {}
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}
