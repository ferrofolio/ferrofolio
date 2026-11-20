// SPDX-License-Identifier: AGPL-3.0-only

//! The label repository, the reading half of
//! `finanze/infrastructure/repository/labeling/label_repository.py`.

use std::collections::HashMap;

use core_app::ports::{LabelPort, PortError};
use core_domain::labeling::{Label, LabelCategory, LabelSubcategory};
use core_domain::{pyrepr, pyuuid};
use rusqlite::types::ValueRef;

use crate::Database;

/// `LabelQueries.GET_ALL`.
const GET_ALL: &str = "
    SELECT id, key, name, description, color, icon, category, subcategory
    FROM labels
    ORDER BY key IS NULL, created_at, name
";

/// `LabelQueries.GET_USAGE`.
const GET_USAGE: &str = "
    SELECT label_id, COUNT(*) AS usage
    FROM account_transaction_labels
    GROUP BY label_id
";

/// `LabelRepository`, for one request on the database a login opened.
#[derive(Debug)]
pub struct LabelRepository<'a> {
    database: &'a Database,
}

impl<'a> LabelRepository<'a> {
    pub fn new(database: &'a Database) -> Self {
        LabelRepository { database }
    }
}

/// A text column of a label as Python's `sqlite3` returns it and pydantic reads it
/// into a `str`: `None` for NULL, a blob decoded as UTF-8. The columns have text
/// affinity, so SQLite stores numbers as text. A blob that is not UTF-8 fails in
/// both, with pydantic's longer message in Python.
fn text(value: ValueRef<'_>) -> Result<Option<String>, String> {
    match value {
        ValueRef::Null => Ok(None),
        ValueRef::Text(bytes) | ValueRef::Blob(bytes) => std::str::from_utf8(bytes)
            .map(|text| Some(text.to_owned()))
            .map_err(|_| "Could not decode to UTF-8".to_owned()),
        ValueRef::Integer(number) => Ok(Some(number.to_string())),
        ValueRef::Real(number) => Ok(Some(number.to_string())),
    }
}

/// `UUID(value)` of a column that is never NULL in the schema. A blob reaches
/// Python's `UUID` as `bytes`, which it cannot read as text.
fn uuid(value: ValueRef<'_>) -> Result<String, String> {
    if let ValueRef::Blob(_) = value {
        return Err("a bytes-like object is required, not 'str'".to_owned());
    }
    match text(value)? {
        Some(text) => pyuuid::parse(&text),
        None => Err(
            "one of the hex, bytes, bytes_le, fields, or int arguments must be given".to_owned(),
        ),
    }
}

/// The value of an enum column as Python's enum reads it: a blob is `bytes`, which
/// matches no member, and NULL is `None`.
fn enum_value<T>(
    value: ValueRef<'_>,
    name: &str,
    parse: impl Fn(&str) -> Result<T, String>,
) -> Result<T, String> {
    match value {
        ValueRef::Blob(bytes) => Err(format!(
            "{} is not a valid {name}",
            pyrepr::repr_bytes(bytes)
        )),
        ValueRef::Null => Err(format!("None is not a valid {name}")),
        other => parse(&text(other)?.unwrap_or_default()),
    }
}

/// `_map_label_row`, which reads the id, then the category, then the subcategory,
/// and fails at the first that Python cannot read.
fn map_label_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<Result<Label, String>> {
    let get = |index: usize| row.get_ref(index);
    let mapped = (|| -> Result<Label, String> {
        let id = uuid(get(0).map_err(|error| error.to_string())?)?;
        let field = |index: usize| -> Result<Option<String>, String> {
            text(get(index).map_err(|error| error.to_string())?)
        };
        let category = enum_value(
            get(6).map_err(|error| error.to_string())?,
            "LabelCategory",
            LabelCategory::parse,
        )?;
        let subcategory = enum_value(
            get(7).map_err(|error| error.to_string())?,
            "LabelSubcategory",
            LabelSubcategory::parse,
        )?;
        Ok(Label {
            id,
            key: field(1)?,
            name: field(2)?,
            description: field(3)?,
            color: field(4)?,
            icon: field(5)?,
            category,
            subcategory,
            usage: None,
        })
    })();
    Ok(mapped)
}

impl LabelPort for LabelRepository<'_> {
    fn get_all(&self) -> Result<Vec<Label>, PortError> {
        let sqlite = |error: rusqlite::Error| PortError(error.to_string());
        let mut statement = self.database.connection.prepare(GET_ALL).map_err(sqlite)?;
        let rows = statement.query_map([], map_label_row).map_err(sqlite)?;
        let mut labels = Vec::new();
        for row in rows {
            labels.push(row.map_err(sqlite)?.map_err(PortError)?);
        }
        Ok(labels)
    }

    fn get_usage(&self) -> Result<HashMap<String, u64>, PortError> {
        let sqlite = |error: rusqlite::Error| PortError(error.to_string());
        let mut statement = self
            .database
            .connection
            .prepare(GET_USAGE)
            .map_err(sqlite)?;
        let rows = statement
            .query_map([], |row| {
                let id = uuid(row.get_ref(0)?);
                let usage: i64 = row.get(1)?;
                Ok((id, usage))
            })
            .map_err(sqlite)?;
        let mut usage = HashMap::new();
        for row in rows {
            let (id, count) = row.map_err(sqlite)?;
            let count = u64::try_from(count).map_err(|error| PortError(error.to_string()))?;
            // A later row of the same UUID, spelled differently, replaces the
            // earlier one, as in Python's dict comprehension.
            usage.insert(id.map_err(PortError)?, count);
        }
        Ok(usage)
    }
}

#[cfg(test)]
mod tests {
    use rusqlite::Connection;
    use rusqlite::types::Value;

    use super::*;

    /// A database with the `labels` table and one label of the given values.
    fn with_label(id: Value, category: Value, subcategory: Value) -> Database {
        let connection = Connection::open_in_memory().unwrap();
        connection
            .execute_batch(
                "CREATE TABLE labels (id CHAR(36), key VARCHAR(64), name TEXT,
                 description TEXT, color VARCHAR(16), icon VARCHAR(64),
                 category VARCHAR(16), subcategory VARCHAR(16), created_at DATETIME);",
            )
            .unwrap();
        connection
            .execute(
                "INSERT INTO labels (id, name, category, subcategory, created_at)
                 VALUES (?, 'n', ?, ?, '2026-01-01')",
                rusqlite::params![id, category, subcategory],
            )
            .unwrap();
        Database { connection }
    }

    fn error(database: &Database) -> String {
        LabelRepository::new(database).get_all().unwrap_err().0
    }

    const ID: &str = "550e8400-e29b-41d4-a716-446655440000";

    #[test]
    fn blobs_fail_as_bytes_fail_in_python() {
        // Messages recorded from Python 3.14.
        let text = |value: &str| Value::Text(value.to_owned());
        let blob = |value: &[u8]| Value::Blob(value.to_vec());
        let database = with_label(blob(ID.as_bytes()), text("INCOME"), text("NONE"));
        assert_eq!(
            error(&database),
            "a bytes-like object is required, not 'str'"
        );
        let database = with_label(text(ID), blob(b"INCOME"), text("NONE"));
        assert_eq!(error(&database), "b'INCOME' is not a valid LabelCategory");
        let database = with_label(text(ID), text("INCOME"), blob(b"it's\x00\xff\\"));
        assert_eq!(
            error(&database),
            "b\"it's\\x00\\xff\\\\\" is not a valid LabelSubcategory"
        );
        let database = with_label(text(ID), text("INCOME"), text("NONE"));
        let labels = LabelRepository::new(&database).get_all().unwrap();
        assert_eq!(labels[0].id, ID);
    }
}
