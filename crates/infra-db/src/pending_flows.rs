// SPDX-License-Identifier: AGPL-3.0-only

//! The pending flow repository, the reading half of
//! `finanze/infrastructure/repository/earnings_expenses/pending_flow_repository.py`.
//!
//! As for the periodic flows, Python maps each row into a pydantic dataclass,
//! which coerces forms the writer never stores, such as a number of seconds as the
//! time of the last status change. Rust reads exactly the forms the writer stores
//! and reports any other as [`ReadError::Unreadable`], upon which the route passes
//! the request on to Python.

use core_app::ports::{PendingFlowPort, PortError, ReadError};
use core_domain::datetime::DateTime;
use core_domain::earnings_expenses::{FlowStatus, FlowType, PendingFlow};
use core_domain::pyuuid;
pub use rusqlite::types::ValueRef;

use crate::Database;
use crate::read::{Read, amount, date, required, text};

/// `PendingFlowsQueries.GET_ALL`, word for word: without an `ORDER BY` the flows
/// come in the order SQLite reads them, the same for Rust on the same file.
const GET_ALL: &str = "SELECT * FROM pending_flows";

/// `PendingFlowRepository`, for one request on the database a login opened.
#[derive(Debug)]
pub struct PendingFlowRepository<'a> {
    database: &'a Database,
}

impl<'a> PendingFlowRepository<'a> {
    pub fn new(database: &'a Database) -> Self {
        PendingFlowRepository { database }
    }
}

/// The columns of one row of `pending_flows`, as SQLite returns them.
#[derive(Debug, Clone, Copy)]
pub struct StoredFlow<'v> {
    pub id: ValueRef<'v>,
    pub name: ValueRef<'v>,
    pub amount: ValueRef<'v>,
    pub currency: ValueRef<'v>,
    pub flow_type: ValueRef<'v>,
    pub category: ValueRef<'v>,
    pub status: ValueRef<'v>,
    pub status_changed_at: ValueRef<'v>,
    pub date: ValueRef<'v>,
    pub icon: ValueRef<'v>,
}

/// A time the writer stores as `datetime.isoformat()` of an aware or naive
/// `datetime`. Only a text that `isoformat()` gives back unchanged, with an offset
/// of whole minutes, is read: pydantic reads that text to the same `datetime`,
/// while it reads other forms, such as `Z`, `.000000` or a space as the
/// separator, to one that `isoformat()` writes differently.
fn timestamp(value: ValueRef<'_>, what: &str) -> Read<Option<DateTime>> {
    let Some(text) = text(value, what)? else {
        return Ok(None);
    };
    let parsed = DateTime::from_isoformat(text).map_err(|_| what.to_owned())?;
    let whole_minutes = parsed
        .offset()
        .is_none_or(|offset| offset.microseconds() % 60_000_000 == 0);
    if whole_minutes && parsed.isoformat() == text {
        Ok(Some(parsed))
    } else {
        Err(what.to_owned())
    }
}

/// `_map_row_to_pending_flow`, for the forms the writer stores.
pub fn map_row(row: &StoredFlow<'_>) -> Read<PendingFlow> {
    let id = pyuuid::parse(required(row.id, "id")?).map_err(|_| "id".to_owned())?;
    Ok(PendingFlow {
        id,
        name: required(row.name, "name")?.to_owned(),
        amount: amount(required(row.amount, "amount")?, "amount")?,
        currency: required(row.currency, "currency")?.to_owned(),
        flow_type: FlowType::parse(required(row.flow_type, "flow type")?)
            .ok_or_else(|| "flow type".to_owned())?,
        category: text(row.category, "category")?.map(str::to_owned),
        status: FlowStatus::parse(required(row.status, "status")?)
            .ok_or_else(|| "status".to_owned())?,
        date: date(row.date, "date")?,
        icon: text(row.icon, "icon")?.map(str::to_owned),
        status_changed_at: timestamp(row.status_changed_at, "status changed at")?,
    })
}

fn read_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<Read<PendingFlow>> {
    let stored = StoredFlow {
        id: row.get_ref("id")?,
        name: row.get_ref("name")?,
        amount: row.get_ref("amount")?,
        currency: row.get_ref("currency")?,
        flow_type: row.get_ref("flow_type")?,
        category: row.get_ref("category")?,
        status: row.get_ref("status")?,
        status_changed_at: row.get_ref("status_changed_at")?,
        date: row.get_ref("date")?,
        icon: row.get_ref("icon")?,
    };
    Ok(map_row(&stored))
}

impl PendingFlowPort for PendingFlowRepository<'_> {
    fn get_all(&self) -> Result<Vec<PendingFlow>, ReadError> {
        let sqlite = |error: rusqlite::Error| ReadError::Failed(PortError(error.to_string()));
        let mut statement = self.database.connection.prepare(GET_ALL).map_err(sqlite)?;
        let rows = statement.query_map([], read_row).map_err(sqlite)?;
        let mut flows = Vec::new();
        for row in rows {
            flows.push(row.map_err(sqlite)?.map_err(ReadError::Unreadable)?);
        }
        Ok(flows)
    }
}

#[cfg(test)]
mod tests {
    use core_domain::Dezimal;
    use core_domain::dates::Date;
    use rusqlite::Connection;

    use super::*;

    const ID: &str = "550e8400-e29b-41d4-a716-446655440000";

    fn database(rows: &[String]) -> Database {
        let connection = Connection::open_in_memory().unwrap();
        connection
            .execute_batch(
                "CREATE TABLE pending_flows (id CHAR(36) PRIMARY KEY, name TEXT NOT NULL,
                 amount TEXT NOT NULL, currency CHAR(3) NOT NULL, flow_type VARCHAR(16) NOT NULL,
                 category TEXT, status VARCHAR(16) NOT NULL DEFAULT 'ACTIVE',
                 status_changed_at DATETIME, date DATE, icon TEXT);",
            )
            .unwrap();
        for values in rows {
            connection
                .execute_batch(&format!(
                    "INSERT INTO pending_flows (id, name, amount, currency, flow_type, category,
                     status, status_changed_at, date, icon) VALUES ({values});"
                ))
                .unwrap();
        }
        Database { connection }
    }

    fn read(database: &Database) -> Result<Vec<PendingFlow>, ReadError> {
        PendingFlowRepository::new(database).get_all()
    }

    #[test]
    fn reads_what_the_writer_stores() {
        let database = database(&[
            format!(
                "'{}', 'Bonus', '1500.50', 'EUR', 'EARNING', 'work', 'COMPLETED',
                 '2026-11-20T14:36:09.123456+01:00', '2026-12-01', 'gift'",
                ID.to_uppercase()
            ),
            format!("'{ID}', 'Tax', '1', 'EUR', 'EXPENSE', NULL, 'ACTIVE', NULL, NULL, NULL"),
        ]);
        let flows = read(&database).unwrap();
        let flow = &flows[0];
        assert_eq!(flow.id, ID);
        assert_eq!(flow.amount, Dezimal::from_python_str("1500.5").unwrap());
        assert_eq!(flow.status, FlowStatus::Completed);
        assert_eq!(flow.date, Date::new(2026, 12, 1).ok());
        assert_eq!(
            flow.status_changed_at
                .as_ref()
                .map(DateTime::isoformat)
                .as_deref(),
            Some("2026-11-20T14:36:09.123456+01:00")
        );
        assert_eq!(flows[1].status_changed_at, None);
        assert_eq!(flows[1].category, None);
    }

    #[test]
    fn leaves_other_forms_to_python() {
        let row = |status: &str, changed: &str, date: &str| {
            format!("'{ID}', 'n', '1', 'EUR', 'EXPENSE', NULL, {status}, {changed}, {date}, NULL")
        };
        for values in [
            row("'active'", "NULL", "NULL"),
            row("'ACTIVE'", "'2026-11-20 13:36:09+00:00'", "NULL"),
            row("'ACTIVE'", "'2026-11-20T13:36:09Z'", "NULL"),
            row("'ACTIVE'", "'2026-11-20T13:36:09.000000'", "NULL"),
            row("'ACTIVE'", "'2026-11-20T13:36:09-00:00'", "NULL"),
            row("'ACTIVE'", "'2026-11-20T13:36:09+01:00:30'", "NULL"),
            row("'ACTIVE'", "'1700000000'", "NULL"),
            row("'ACTIVE'", "1700000000", "NULL"),
            row("'ACTIVE'", "''", "NULL"),
            row("'ACTIVE'", "NULL", "'20261201'"),
            row("'ACTIVE'", "NULL", "''"),
            format!("'{ID}', 'n', 'NaN', 'EUR', 'EXPENSE', NULL, 'ACTIVE', NULL, NULL, NULL"),
            "'x', 'n', '1', 'EUR', 'EXPENSE', NULL, 'ACTIVE', NULL, NULL, NULL".to_owned(),
        ] {
            let database = database(std::slice::from_ref(&values));
            assert!(
                matches!(read(&database), Err(ReadError::Unreadable(_))),
                "{values}"
            );
        }
    }
}
