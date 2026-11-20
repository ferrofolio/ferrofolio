// SPDX-License-Identifier: AGPL-3.0-only

//! The label repository, the reading half of
//! `finanze/infrastructure/repository/labeling/label_repository.py`.

use std::collections::HashMap;

use core_app::ports::{LabelPort, PortError};
use core_domain::labeling::{Label, LabelCategory, LabelSubcategory};
use core_domain::pyuuid;
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

/// A text column as Python's `sqlite3` returns it: `None` for NULL. The columns of
/// these tables have text affinity, so SQLite stores numbers as text; a blob is
/// read as text, as pydantic reads bytes into a `str`.
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

/// `UUID(value)` of a column that is never NULL in the schema.
fn uuid(value: ValueRef<'_>) -> Result<String, String> {
    match text(value)? {
        Some(text) => pyuuid::parse(&text),
        None => Err(
            "one of the hex, bytes, bytes_le, fields, or int arguments must be given".to_owned(),
        ),
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
        let enum_text = |index: usize| -> Result<String, String> {
            // `LabelCategory(None)` reads as `repr(None)`; the schema forbids it.
            Ok(field(index)?.unwrap_or_else(|| "None".to_owned()))
        };
        let category = LabelCategory::parse(&enum_text(6)?)?;
        let subcategory = LabelSubcategory::parse(&enum_text(7)?)?;
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
