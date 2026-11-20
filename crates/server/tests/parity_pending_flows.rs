// SPDX-License-Identifier: AGPL-3.0-only

//! Parity of `GET /api/v1/flows/pending` with the Python route, from the fixtures
//! `tests/parity/generate_db_route_fixtures.py` records against the fixture
//! databases, of the reading of stored rows and of `args.get(name, type=int)`.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod common;

use common::database::fixture;
use infra_db::pending_flows::{StoredFlow, ValueRef, map_row};
use serde_json::Value;
use server::pycompat::{PyInt, py_int};
use server::routes::pending_flows::flow_json;

fn fixtures() -> Value {
    fixture("pending_flows.json")
}

fn assert_replays(sessions: bool, replayed: usize, failures: &[String]) {
    assert!(replayed >= 60, "the fixtures look truncated ({sessions})");
    assert!(
        failures.is_empty(),
        "{} cases differ from Python:\n{}",
        failures.len(),
        failures.join("\n")
    );
}

#[tokio::test]
async fn the_pending_flows_route_answers_like_the_python_backend() {
    let fixtures = fixtures();
    let (replayed, failures) = common::database::replay(&fixtures, true, |_| {}).await;
    assert_replays(true, replayed, &failures);
}

#[tokio::test]
async fn without_api_sessions_too() {
    let fixtures = fixtures();
    let (replayed, failures) = common::database::replay(&fixtures, false, |_| {}).await;
    assert_replays(false, replayed, &failures);
}

/// A recorded column as SQLite returns it.
fn column(value: &Value) -> ValueRef<'_> {
    match value {
        Value::Null => ValueRef::Null,
        Value::String(text) => ValueRef::Text(text.as_bytes()),
        Value::Number(number) => match number.as_i64() {
            Some(integer) => ValueRef::Integer(integer),
            None => ValueRef::Real(number.as_f64().unwrap()),
        },
        other => panic!("not a column value: {other}"),
    }
}

/// Rust reads every row in the forms Python's writer stores, and the other
/// recorded forms it reads, exactly as Python does. Where Python fails, or
/// coerces a form Rust does not read, Rust reports it unreadable, so that the
/// route passes the request on: it never answers where Python would not.
#[test]
fn stored_rows_are_read_like_python_reads_them() {
    let fixtures = fixtures();
    let recorded = fixtures["functions"]["rows"].as_array().unwrap();
    assert!(recorded.len() > 40, "the fixtures look truncated");
    let mut failures = Vec::new();
    for entry in recorded {
        let row = &entry[0];
        let python = &entry[1];
        let kind = entry[2].as_str().unwrap();
        let stored = StoredFlow {
            id: column(&row["id"]),
            name: column(&row["name"]),
            amount: column(&row["amount"]),
            currency: column(&row["currency"]),
            flow_type: column(&row["flow_type"]),
            category: column(&row["category"]),
            status: column(&row["status"]),
            status_changed_at: column(&row["status_changed_at"]),
            date: column(&row["date"]),
            icon: column(&row["icon"]),
        };
        let rust = map_row(&stored).map(|flow| flow_json(&flow));
        match (&rust, python.get("ok")) {
            (Ok(Some(read)), Some(expected)) if !common::same_json(read, expected) => {
                failures.push(format!("{row}: Python {expected}, Rust {read}"));
            }
            (Ok(Some(read)), None) => {
                failures.push(format!("{row}: Python fails, Rust answers {read}"));
            }
            (Ok(None), _) => failures.push(format!("{row}: Rust cannot write the amount")),
            (Err(reason), _) if kind != "python" => {
                failures.push(format!("{row} ({kind}): Rust cannot read it: {reason}"));
            }
            _ => {}
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

/// `page` and `limit` are read as `int` reads them; an integer outside `i64`,
/// which Rust passes on, is one for Python too, or one it fails to read.
#[test]
fn integers_are_read_like_python_reads_them() {
    let fixtures = fixtures();
    let recorded = fixtures["functions"]["ints"].as_array().unwrap();
    assert!(recorded.len() > 30, "the fixtures look truncated");
    let mut failures = Vec::new();
    for entry in recorded {
        let text = entry[0].as_str().unwrap();
        let python = &entry[1];
        let same = match py_int(text) {
            PyInt::Value(value) => python.as_i64() == Some(value),
            PyInt::Invalid => python.is_null(),
            PyInt::OutOfRange => python == "out of range" || python.is_null(),
        };
        if !same {
            failures.push(format!(
                "{text:?}: Python {python}, Rust {:?}",
                py_int(text)
            ));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}
