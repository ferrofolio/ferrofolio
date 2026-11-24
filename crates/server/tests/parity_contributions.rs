// SPDX-License-Identifier: AGPL-3.0-only

//! Parity of `GET /api/v1/contributions` with the Python route, from the fixtures
//! `tests/parity/generate_db_route_fixtures.py` records against the fixture
//! databases, and of the next date for the days recorded from Python.

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
use core_app::use_cases::get_contributions::next_date;
use core_domain::Dezimal;
use core_domain::auto_contributions::{
    ContributionFrequency, ContributionTargetType, PeriodicContribution,
};
use core_domain::dates::Date;
use core_domain::entity::{Entity, EntityOrigin, EntityType};
use core_domain::fetch_record::DataSource;
use serde_json::Value;
use server::app::AppState;

fn fixtures() -> Value {
    fixture("contributions.json")
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
    assert!(replayed >= 10, "the fixtures look truncated ({sessions})");
    assert!(
        failures.is_empty(),
        "{} cases differ from Python:\n{}",
        failures.len(),
        failures.join("\n")
    );
}

#[tokio::test]
async fn the_contributions_route_answers_like_the_python_backend() {
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

#[test]
fn next_dates_are_computed_like_python_computes_them() {
    let fixtures = fixtures();
    let recorded = fixtures["functions"]["next_date"].as_array().unwrap();
    assert!(recorded.len() > 1000, "the fixtures look truncated");
    let mut failures = Vec::new();
    for entry in recorded {
        let contribution = PeriodicContribution {
            id: "550e8400-e29b-41d4-a716-446655440000".parse().unwrap(),
            alias: None,
            target: "target".to_owned(),
            target_name: "Target".to_owned(),
            target_type: ContributionTargetType::Fund,
            target_subtype: None,
            amount: Dezimal::from_python_str("1").unwrap(),
            currency: "EUR".to_owned(),
            since: date(entry[1].as_str().unwrap()),
            until: entry[2].as_str().map(date),
            frequency: ContributionFrequency::from_value(entry[0].as_str().unwrap()).unwrap(),
            active: entry[3].as_bool().unwrap(),
            source: DataSource::Real,
            next_date: None,
            entity: Entity {
                id: None,
                name: "Bank".to_owned(),
                natural_id: None,
                r#type: EntityType::FinancialInstitution,
                origin: EntityOrigin::Native,
                icon_url: None,
            },
            entity_account_id: None,
        };
        let today = date(entry[4].as_str().unwrap());
        let rust = next_date(&contribution, today).map(|next| next.map(|next| next.to_string()));
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
