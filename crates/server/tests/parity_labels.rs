// SPDX-License-Identifier: AGPL-3.0-only

//! Parity of `GET /api/v1/labels` with the Python route, from the fixtures
//! `tests/parity/generate_db_route_fixtures.py` records against the fixture
//! databases, and of the functions behind it for the values recorded from Python.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod common;

use common::database::fixture;
use core_domain::labeling::{LabelCategory, LabelSubcategory};
use core_domain::pyuuid;
use serde_json::{Value, json};

fn fixtures() -> Value {
    fixture("labels.json")
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
async fn the_labels_route_answers_like_the_python_backend() {
    let (replayed, failures) = common::database::replay(&fixtures(), true, |_| {}).await;
    assert_replays(true, replayed, &failures);
}

#[tokio::test]
async fn without_api_sessions_too() {
    let (replayed, failures) = common::database::replay(&fixtures(), false, |_| {}).await;
    assert_replays(false, replayed, &failures);
}

/// `{"ok": value}` or `{"error": message}`, as the generator records an outcome.
fn outcome(result: Result<String, String>) -> Value {
    match result {
        Ok(value) => json!({ "ok": value }),
        Err(message) => json!({ "error": message }),
    }
}

/// Checks a function against the inputs and outcomes recorded from Python.
fn assert_function(name: &str, function: impl Fn(&str) -> Result<String, String>) {
    let fixtures = fixtures();
    let recorded = fixtures["functions"][name].as_array().unwrap();
    assert!(!recorded.is_empty());
    let failures: Vec<String> = recorded
        .iter()
        .filter_map(|entry| {
            let input = entry[0].as_str().unwrap();
            let got = outcome(function(input));
            (got != entry[1]).then(|| format!("{input:?}: expected {} got {got}", entry[1]))
        })
        .collect();
    assert!(failures.is_empty(), "{name}:\n{}", failures.join("\n"));
}

#[test]
fn ids_are_read_like_python_reads_them() {
    assert_function("uuid", pyuuid::parse);
}

#[test]
fn categories_are_read_like_python_reads_them() {
    assert_function("category", |text| {
        LabelCategory::parse(text).map(|category| category.value().to_owned())
    });
    assert_function("subcategory", |text| {
        LabelSubcategory::parse(text).map(|subcategory| subcategory.value().to_owned())
    });
}
