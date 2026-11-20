// SPDX-License-Identifier: AGPL-3.0-only

//! The periodic flow repository, the reading half of
//! `finanze/infrastructure/repository/earnings_expenses/periodic_flow_repository.py`.
//!
//! Python maps each row into a pydantic dataclass, which in lax mode coerces many
//! forms the repository's writer never stores: `"true"` or `2` as a flag, an
//! integer as a date, a number as an amount. Rust reads exactly the forms the
//! writer stores, and reports any other as [`ReadError::Unreadable`], upon which
//! the route passes the request on to Python, which answers from the same data.

use core_app::ports::{PeriodicFlowPort, PortError, ReadError};
use core_domain::Dezimal;
use core_domain::dates::Date;
use core_domain::earnings_expenses::{FlowFrequency, FlowType, PeriodicFlow, RealEstateFlowInfo};
use core_domain::pyuuid;
pub use rusqlite::types::ValueRef;

use crate::Database;
use crate::read::iso_date;

/// `PeriodicFlowsQueries.GET_ALL`, word for word: it has no `ORDER BY`, so the
/// flows come in the order SQLite reads them, which the same query on the same
/// file gives Rust too.
const GET_ALL: &str = "
        SELECT f.*, rf.real_estate_id IS NOT NULL AS linked,
            rf.extra_reference AS linked_loan_hash,
            rf.flow_subtype AS re_flow_subtype
        FROM periodic_flows f
            LEFT JOIN real_estate_flows rf ON f.id = rf.periodic_flow_id
    ";

/// `PeriodicFlowRepository`, for one request on the database a login opened.
#[derive(Debug)]
pub struct PeriodicFlowRepository<'a> {
    database: &'a Database,
}

impl<'a> PeriodicFlowRepository<'a> {
    pub fn new(database: &'a Database) -> Self {
        PeriodicFlowRepository { database }
    }
}

/// The columns of one row of the query, as SQLite returns them.
#[derive(Debug, Clone, Copy)]
pub struct StoredFlow<'v> {
    pub id: ValueRef<'v>,
    pub name: ValueRef<'v>,
    pub amount: ValueRef<'v>,
    pub currency: ValueRef<'v>,
    pub flow_type: ValueRef<'v>,
    pub frequency: ValueRef<'v>,
    pub category: ValueRef<'v>,
    pub enabled: ValueRef<'v>,
    pub since: ValueRef<'v>,
    pub until: ValueRef<'v>,
    pub icon: ValueRef<'v>,
    pub linked: ValueRef<'v>,
    pub linked_loan_hash: ValueRef<'v>,
    pub re_flow_subtype: ValueRef<'v>,
    pub max_amount: ValueRef<'v>,
    pub label_id: ValueRef<'v>,
}

type Read<T> = Result<T, String>;

/// A text column, `None` for NULL. Any other type is unreadable: pydantic does
/// not read a number as a `str`, and a blob is not what the writer stores.
fn text<'v>(value: ValueRef<'v>, what: &str) -> Read<Option<&'v str>> {
    match value {
        ValueRef::Null => Ok(None),
        ValueRef::Text(bytes) => std::str::from_utf8(bytes)
            .map(Some)
            .map_err(|_| what.to_owned()),
        _ => Err(what.to_owned()),
    }
}

/// A text column that must hold a value.
fn required<'v>(value: ValueRef<'v>, what: &str) -> Read<&'v str> {
    text(value, what)?.ok_or_else(|| what.to_owned())
}

/// `Dezimal(text)` of an amount the writer stores as `str(Dezimal)`. A number that
/// a float cannot hold, which Python would answer as `Infinity`, is unreadable.
fn amount(text: &str, what: &str) -> Read<Dezimal> {
    match Dezimal::from_python_str(text) {
        Ok(amount) if amount.to_f64_for_api().is_finite() => Ok(amount),
        _ => Err(what.to_owned()),
    }
}

/// A flag the writer stores as a Python `bool`, which SQLite keeps as `0` or `1`.
fn flag(value: ValueRef<'_>, what: &str) -> Read<bool> {
    match value {
        ValueRef::Integer(0) => Ok(false),
        ValueRef::Integer(1) => Ok(true),
        _ => Err(what.to_owned()),
    }
}

/// A date the writer stores as `date.isoformat()`.
fn date(value: ValueRef<'_>, what: &str) -> Read<Option<Date>> {
    match text(value, what)? {
        None => Ok(None),
        Some(text) => iso_date(text).map(Some).ok_or_else(|| what.to_owned()),
    }
}

/// `UUID(row[key]) if row[key] else None`: NULL and the empty text are `None`.
fn optional_uuid(value: ValueRef<'_>, what: &str) -> Read<Option<String>> {
    match text(value, what)? {
        None | Some("") => Ok(None),
        Some(id) => pyuuid::parse(id).map(Some).map_err(|_| what.to_owned()),
    }
}

/// `_map_row_to_periodic_flow`, for the forms the writer stores.
pub fn map_row(row: &StoredFlow<'_>) -> Read<PeriodicFlow> {
    let id = pyuuid::parse(required(row.id, "id")?).map_err(|_| "id".to_owned())?;
    let real_estate_flow = match text(row.re_flow_subtype, "real estate flow subtype")? {
        None => None,
        Some(subtype) => Some(RealEstateFlowInfo {
            flow_subtype: subtype.to_owned(),
            linked_loan_hash: text(row.linked_loan_hash, "linked loan hash")?.map(str::to_owned),
        }),
    };
    // `Dezimal(row["max_amount"]) if row["max_amount"] else None`.
    let max_amount = match text(row.max_amount, "max amount")? {
        None | Some("") => None,
        Some(text) => Some(amount(text, "max amount")?),
    };
    Ok(PeriodicFlow {
        id,
        name: required(row.name, "name")?.to_owned(),
        amount: amount(required(row.amount, "amount")?, "amount")?,
        currency: required(row.currency, "currency")?.to_owned(),
        flow_type: FlowType::parse(required(row.flow_type, "flow type")?)
            .ok_or_else(|| "flow type".to_owned())?,
        frequency: FlowFrequency::parse(required(row.frequency, "frequency")?)
            .ok_or_else(|| "frequency".to_owned())?,
        category: text(row.category, "category")?.map(str::to_owned),
        enabled: flag(row.enabled, "enabled")?,
        since: date(row.since, "since")?.ok_or_else(|| "since".to_owned())?,
        until: date(row.until, "until")?,
        icon: text(row.icon, "icon")?.map(str::to_owned),
        linked: Some(flag(row.linked, "linked")?),
        real_estate_flow,
        next_date: None,
        max_amount,
        label_id: optional_uuid(row.label_id, "label id")?,
    })
}

fn read_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<Read<PeriodicFlow>> {
    let stored = StoredFlow {
        id: row.get_ref("id")?,
        name: row.get_ref("name")?,
        amount: row.get_ref("amount")?,
        currency: row.get_ref("currency")?,
        flow_type: row.get_ref("flow_type")?,
        frequency: row.get_ref("frequency")?,
        category: row.get_ref("category")?,
        enabled: row.get_ref("enabled")?,
        since: row.get_ref("since")?,
        until: row.get_ref("until")?,
        icon: row.get_ref("icon")?,
        linked: row.get_ref("linked")?,
        linked_loan_hash: row.get_ref("linked_loan_hash")?,
        re_flow_subtype: row.get_ref("re_flow_subtype")?,
        max_amount: row.get_ref("max_amount")?,
        label_id: row.get_ref("label_id")?,
    };
    Ok(map_row(&stored))
}

impl PeriodicFlowPort for PeriodicFlowRepository<'_> {
    fn get_all(&self) -> Result<Vec<PeriodicFlow>, ReadError> {
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
    use rusqlite::Connection;

    use super::*;

    const ID: &str = "550e8400-e29b-41d4-a716-446655440000";

    fn database(rows: &[&str]) -> Database {
        let connection = Connection::open_in_memory().unwrap();
        connection
            .execute_batch(
                "CREATE TABLE periodic_flows (id CHAR(36) PRIMARY KEY, name TEXT NOT NULL,
                 amount TEXT NOT NULL, currency CHAR(3) NOT NULL, flow_type VARCHAR(16) NOT NULL,
                 frequency VARCHAR(32) NOT NULL, category TEXT, enabled BOOLEAN NOT NULL DEFAULT TRUE,
                 since DATE NOT NULL, until DATE, icon TEXT, max_amount TEXT, label_id CHAR(36));
                 CREATE TABLE real_estate_flows (real_estate_id CHAR(36) NOT NULL,
                 periodic_flow_id CHAR(36) NOT NULL, flow_subtype VARCHAR(16) NOT NULL,
                 description TEXT NOT NULL, payload JSON NOT NULL, extra_reference VARCHAR(255));",
            )
            .unwrap();
        for row in rows {
            connection.execute_batch(row).unwrap();
        }
        Database { connection }
    }

    fn read(database: &Database) -> Result<Vec<PeriodicFlow>, ReadError> {
        PeriodicFlowRepository::new(database).get_all()
    }

    fn insert(values: &str) -> String {
        format!(
            "INSERT INTO periodic_flows (id, name, amount, currency, flow_type, frequency,
             category, enabled, since, until, icon, max_amount, label_id) VALUES ({values});"
        )
    }

    #[test]
    fn reads_what_the_writer_stores() {
        let database = database(&[
            &insert(&format!(
                "'{}', 'Rent', '850.50', 'EUR', 'EXPENSE', 'MONTHLY', 'home', 1, '2025-01-31',
                 NULL, 'house', '900', '7C9E6679742540DE944BE07FC1F90AE7'",
                ID.to_uppercase()
            )),
            &format!(
                "INSERT INTO real_estate_flows VALUES ('r', '{ID}', 'RENT', 'd', '{{}}', 'hash');"
            ),
        ]);
        let flows = read(&database).unwrap();
        assert_eq!(flows.len(), 1);
        let flow = &flows[0];
        assert_eq!(flow.id, ID);
        assert_eq!(flow.frequency, FlowFrequency::Monthly);
        assert_eq!(flow.since, Date::new(2025, 1, 31).unwrap());
        assert_eq!(flow.linked, Some(false));
        assert_eq!(flow.real_estate_flow, None);
        assert_eq!(
            flow.label_id.as_deref(),
            Some("7c9e6679-7425-40de-944b-e07fc1f90ae7")
        );
        assert_eq!(flow.max_amount, Dezimal::from_python_str("900").ok());
    }

    #[test]
    fn reads_the_real_estate_link() {
        let database = database(&[
            &insert(&format!(
                "'{ID}', 'Rent', '1', 'EUR', 'EARNING', 'YEARLY', NULL, 0, '2025-01-31',
                 '2026-01-01', NULL, '', ''"
            )),
            &format!(
                "INSERT INTO real_estate_flows VALUES ('r', '{ID}', 'RENT', 'd', '{{}}', NULL);"
            ),
        ]);
        let flow = &read(&database).unwrap()[0];
        assert_eq!(flow.linked, Some(true));
        assert!(!flow.enabled);
        assert_eq!(flow.max_amount, None);
        assert_eq!(flow.label_id, None);
        assert_eq!(
            flow.real_estate_flow,
            Some(RealEstateFlowInfo {
                flow_subtype: "RENT".to_owned(),
                linked_loan_hash: None
            })
        );
    }

    #[test]
    fn leaves_other_forms_to_python() {
        for values in [
            format!(
                "'{ID}', 'n', '1', 'EUR', 'EXPENSE', 'MONTHLY', NULL, 'true', '2026-01-01', NULL, NULL, NULL, NULL"
            ),
            format!(
                "'{ID}', 'n', '1', 'EUR', 'EXPENSE', 'MONTHLY', NULL, 2, '2026-01-01', NULL, NULL, NULL, NULL"
            ),
            format!(
                "'{ID}', 'n', '1', 'EUR', 'EXPENSE', 'MONTHLY', NULL, 1, '20260101', NULL, NULL, NULL, NULL"
            ),
            format!(
                "'{ID}', 'n', '1', 'EUR', 'EXPENSE', 'MONTHLY', NULL, 1, '2026-02-30', NULL, NULL, NULL, NULL"
            ),
            format!(
                "'{ID}', 'n', '1', 'EUR', 'EXPENSE', 'MONTHLY', NULL, 1, '2026-01-01', '', NULL, NULL, NULL"
            ),
            format!(
                "'{ID}', 'n', 'NaN', 'EUR', 'EXPENSE', 'MONTHLY', NULL, 1, '2026-01-01', NULL, NULL, NULL, NULL"
            ),
            format!(
                "'{ID}', 'n', '1e400', 'EUR', 'EXPENSE', 'MONTHLY', NULL, 1, '2026-01-01', NULL, NULL, NULL, NULL"
            ),
            format!(
                "'{ID}', 'n', '1', 'EUR', 'expense', 'MONTHLY', NULL, 1, '2026-01-01', NULL, NULL, NULL, NULL"
            ),
            format!(
                "'{ID}', 'n', '1', 'EUR', 'EXPENSE', 'monthly', NULL, 1, '2026-01-01', NULL, NULL, NULL, NULL"
            ),
            format!(
                "'{ID}', 'n', '1', 'EUR', 'EXPENSE', 'MONTHLY', NULL, 1, '2026-01-01', NULL, NULL, NULL, 'x'"
            ),
            "'x', 'n', '1', 'EUR', 'EXPENSE', 'MONTHLY', NULL, 1, '2026-01-01', NULL, NULL, NULL, NULL".to_owned(),
            format!(
                "'{ID}', X'6e', '1', 'EUR', 'EXPENSE', 'MONTHLY', NULL, 1, '2026-01-01', NULL, NULL, NULL, NULL"
            ),
        ] {
            let database = database(&[&insert(&values)]);
            assert!(
                matches!(read(&database), Err(ReadError::Unreadable(_))),
                "{values}"
            );
        }
    }
}
