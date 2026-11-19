// SPDX-License-Identifier: AGPL-3.0-only

//! The fixture databases of the routes that read the database
//! (`tests/parity/generate_db_route_fixtures.py`): a login through the router opens
//! each of them for Rust, with and without API sessions, and Rust reads the
//! migrations Python recorded from it.

#![allow(clippy::unwrap_used, clippy::indexing_slicing)]

mod common;

use common::database::{Harness, fixture};
use infra_db::AppliedMigration;
use server::user_data::UnlockError;

#[tokio::test]
async fn a_login_opens_every_fixture_database() {
    let fixtures = fixture("databases.json");
    let databases = fixtures["databases"].as_object().unwrap();
    assert!(databases.contains_key("empty"));
    for (name, database) in databases {
        let recorded: Vec<AppliedMigration> = database["migrations"]
            .as_array()
            .unwrap()
            .iter()
            .map(|migration| AppliedMigration {
                version: migration["version"].as_i64().unwrap(),
                name: migration["name"].as_str().unwrap().to_owned(),
            })
            .collect();
        for sessions in [true, false] {
            let harness =
                Harness::start(database["file"].as_str().unwrap(), sessions, |_| {}).await;
            let locked = harness.read(|database| database.applied_migrations()).await;
            assert!(matches!(locked, Err(UnlockError::Locked)), "{name}");

            let token = harness.login().await;
            assert_eq!(token.is_some(), sessions, "{name}");
            let migrations = harness
                .read(|database| database.applied_migrations())
                .await
                .unwrap()
                .unwrap();
            assert_eq!(migrations, recorded, "{name}, sessions {sessions}");
        }
    }
}
