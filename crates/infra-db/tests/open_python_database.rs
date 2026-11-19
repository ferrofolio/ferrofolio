// SPDX-License-Identifier: AGPL-3.0-only

//! Opens the database that `tests/parity/generate_db_fixture.py` created with the
//! Python backend.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};

use infra_db::{AppliedMigration, Database, OpenError};
use serde_json::Value;

const FIXTURES: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures");
/// The password of the fixture (see the generator), not a secret.
const PASSWORD: &str = "fixture pass'word ü";

/// A copy of the fixture in a directory of its own, because opening a database adds
/// WAL files next to it.
fn fixture_copy() -> PathBuf {
    static NEXT: AtomicU32 = AtomicU32::new(0);
    let directory = std::env::temp_dir().join(format!(
        "ferrofolio-infra-db-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::create_dir_all(&directory).unwrap();
    let copy = directory.join("data.db");
    std::fs::copy(Path::new(FIXTURES).join("data.db"), &copy).unwrap();
    copy
}

#[test]
fn reads_the_migrations_python_applied() {
    let recorded: Vec<Value> = serde_json::from_str(
        &std::fs::read_to_string(Path::new(FIXTURES).join("migrations.json")).unwrap(),
    )
    .unwrap();
    let expected: Vec<AppliedMigration> = recorded
        .iter()
        .map(|entry| AppliedMigration {
            version: entry["version"].as_i64().unwrap(),
            name: entry["name"].as_str().unwrap().to_owned(),
        })
        .collect();
    assert!(!expected.is_empty());

    let database = Database::open_read_only(&fixture_copy(), PASSWORD).unwrap();
    assert_eq!(database.applied_migrations().unwrap(), expected);
}

#[test]
fn a_wrong_password_cannot_decrypt() {
    let error = Database::open_read_only(&fixture_copy(), "fixture password").unwrap_err();
    assert!(matches!(error, OpenError::Decryption), "{error}");
    assert!(!error.to_string().contains("fixture"));
}

#[test]
fn a_missing_file_is_reported() {
    let path = fixture_copy().with_file_name("absent.db");
    assert!(matches!(
        Database::open_read_only(&path, PASSWORD),
        Err(OpenError::NotFound)
    ));
}
