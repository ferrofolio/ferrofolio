// SPDX-License-Identifier: AGPL-3.0-only

//! Parity of `GET /api/v1/flows/periodic` with the Python route, from the fixtures
//! `tests/parity/generate_db_route_fixtures.py` records against the fixture
//! databases, of the next date for the days recorded from Python, and of the
//! reading of stored rows.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod common;

use std::sync::Arc;

use common::database::fixture;
use core_app::ports::FixedClock;
use core_app::use_cases::get_periodic_flows::next_date;
use core_domain::Dezimal;
use core_domain::dates::Date;
use core_domain::earnings_expenses::{FlowFrequency, FlowType, PeriodicFlow};
use infra_db::periodic_flows::{StoredFlow, ValueRef, map_row};
use serde_json::Value;
use server::app::AppState;
use server::routes::periodic_flows::flow_json;

fn fixtures() -> Value {
    fixture("periodic_flows.json")
}

fn date(value: &str) -> Date {
    value.parse().unwrap()
}

/// The recording's day as the server's clock.
fn recorded_today(fixtures: &Value) -> impl Fn(&mut AppState) {
    let today = date(fixtures["context"]["today"].as_str().unwrap());
    move |state: &mut AppState| state.clock = Arc::new(FixedClock(today))
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
async fn the_periodic_flows_route_answers_like_the_python_backend() {
    let fixtures = fixtures();
    let (replayed, failures) =
        common::database::replay(&fixtures, true, recorded_today(&fixtures)).await;
    assert_replays(true, replayed, &failures);
}

#[tokio::test]
async fn without_api_sessions_too() {
    let fixtures = fixtures();
    let (replayed, failures) =
        common::database::replay(&fixtures, false, recorded_today(&fixtures)).await;
    assert_replays(false, replayed, &failures);
}

/// Rust computes the next date of every recorded flow and day as Python does, and
/// fails exactly where Python's date arithmetic fails.
#[test]
fn next_dates_are_computed_like_python_computes_them() {
    let fixtures = fixtures();
    let recorded = fixtures["functions"]["next_date"].as_array().unwrap();
    assert!(recorded.len() > 1000, "the fixtures look truncated");
    let mut failures = Vec::new();
    for entry in recorded {
        let flow = PeriodicFlow {
            id: "a".to_owned(),
            name: "Flow".to_owned(),
            amount: Dezimal::from_python_str("1").unwrap(),
            currency: "EUR".to_owned(),
            flow_type: FlowType::Expense,
            frequency: FlowFrequency::from_value(entry[0].as_str().unwrap()).unwrap(),
            category: None,
            enabled: entry[3].as_bool().unwrap(),
            since: date(entry[1].as_str().unwrap()),
            until: entry[2].as_str().map(date),
            icon: None,
            linked: None,
            real_estate_flow: None,
            next_date: None,
            max_amount: None,
            label_id: None,
        };
        let today = date(entry[4].as_str().unwrap());
        let rust = next_date(&flow, today).map(|next| next.map(|next| next.to_string()));
        let python = &entry[5];
        let same = match (&rust, python.get("ok")) {
            (Ok(next), Some(expected)) => next.as_deref() == expected.as_str(),
            (Err(_), None) => true,
            _ => false,
        };
        if !same {
            failures.push(format!("{entry}: Rust {rust:?}"));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
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
    assert!(recorded.len() > 30, "the fixtures look truncated");
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
            frequency: column(&row["frequency"]),
            category: column(&row["category"]),
            enabled: column(&row["enabled"]),
            since: column(&row["since"]),
            until: column(&row["until"]),
            icon: column(&row["icon"]),
            linked: column(&row["linked"]),
            linked_loan_hash: column(&row["linked_loan_hash"]),
            re_flow_subtype: column(&row["re_flow_subtype"]),
            max_amount: column(&row["max_amount"]),
            label_id: column(&row["label_id"]),
        };
        let rust = map_row(&stored).map(|flow| flow_json(&flow));
        match (&rust, python.get("ok")) {
            (Ok(read), Some(expected)) if !common::same_json(read, expected) => {
                failures.push(format!("{row}: Python {expected}, Rust {read}"));
            }
            (Ok(read), None) => {
                failures.push(format!("{row}: Python fails, Rust answers {read}"));
            }
            (Err(reason), _) if kind != "python" => {
                failures.push(format!("{row} ({kind}): Rust cannot read it: {reason}"));
            }
            _ => {}
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}
