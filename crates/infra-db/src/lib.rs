// SPDX-License-Identifier: AGPL-3.0-only

//! The Ferrofolio database, the counterpart of `finanze/infrastructure/repository/db`.
//!
//! The Python backend keeps every user's data in one SQLCipher database, encrypted
//! with the user's password. This crate opens such a database the way the backend
//! does; until the cutover the Python backend owns the schema and its migrations
//! (docs/roadmap.md, section 5), so Rust only reads.

pub mod settings;

use std::path::Path;

use rusqlite::{Connection, ErrorCode, OpenFlags};

/// An unlocked database.
#[derive(Debug)]
pub struct Database {
    connection: Connection,
}

/// Why a database could not be opened. The messages never contain the password.
#[derive(Debug, thiserror::Error)]
pub enum OpenError {
    #[error("there is no database at the given path")]
    NotFound,
    #[error("the database cannot be decrypted: wrong password or not a database")]
    Decryption,
    #[error("database error: {0}")]
    Sqlite(#[from] rusqlite::Error),
}

/// A migration the Python backend has applied, from its `migrations` table.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppliedMigration {
    pub version: i64,
    pub name: String,
}

impl Database {
    /// Opens the database at `path` read-only and unlocks it with `password`, as the
    /// Python backend does: SQLCipher's `PRAGMA key` with the password as passphrase.
    pub fn open_read_only(path: &Path, password: &str) -> Result<Self, OpenError> {
        if !path.is_file() {
            return Err(OpenError::NotFound);
        }
        let connection = Connection::open_with_flags(
            path,
            OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )?;
        connection.pragma_update(None, "key", password)?;
        // SQLCipher only notices a wrong key when it first reads a page.
        match connection.query_row("SELECT count(*) FROM sqlite_master", [], |row| {
            row.get::<_, i64>(0)
        }) {
            Ok(_) => Ok(Database { connection }),
            Err(rusqlite::Error::SqliteFailure(error, _))
                if error.code == ErrorCode::NotADatabase =>
            {
                Err(OpenError::Decryption)
            }
            Err(error) => Err(error.into()),
        }
    }

    /// The migrations the Python backend has applied, in the order it applied them.
    pub fn applied_migrations(&self) -> Result<Vec<AppliedMigration>, rusqlite::Error> {
        let mut statement = self
            .connection
            .prepare("SELECT version, name FROM migrations ORDER BY version")?;
        let rows = statement.query_map([], |row| {
            Ok(AppliedMigration {
                version: row.get(0)?,
                name: row.get(1)?,
            })
        })?;
        rows.collect()
    }
}
