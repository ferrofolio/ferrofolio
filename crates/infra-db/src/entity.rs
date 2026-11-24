// SPDX-License-Identifier: AGPL-3.0-only

//! The entity repository, the reading half of
//! `finanze/infrastructure/repository/entity/entity_repository.py`.

use core_app::ports::{EntityPort, ReadError};
use core_domain::Uuid;
use core_domain::pyuuid;

use crate::Database;

/// `EntityQueries.GET_DISABLED_ENTITIES`, word for word.
const GET_DISABLED: &str = "
    WITH latest_manual AS (
        SELECT DISTINCT entity_id
        FROM virtual_data_imports
        WHERE import_id = (
            SELECT import_id
            FROM virtual_data_imports
            ORDER BY date DESC
            LIMIT 1
        )
        AND entity_id IS NOT NULL
    )
    SELECT e.*
    FROM entities e
        LEFT JOIN entity_accounts ea ON e.id = ea.entity_id AND ea.deleted_at IS NULL
        LEFT JOIN external_entities ee ON e.id = ee.entity_id
        LEFT JOIN latest_manual lm ON e.id = lm.entity_id
    WHERE (
        e.origin = 'EXTERNALLY_PROVIDED' AND ee.entity_id IS NULL
    )
    OR (
        e.origin = 'NATIVE'
        AND e.type IN ('FINANCIAL_INSTITUTION', 'CRYPTO_EXCHANGE', 'MARKET_FORECAST_PLATFORM')
        AND ea.entity_id IS NULL
    )
    OR (
        e.origin = 'MANUAL' AND e.type = 'FINANCIAL_INSTITUTION' AND lm.entity_id IS NULL
    )
";

/// `EntityRepository`, for one request on the database a login opened.
#[derive(Debug)]
pub struct EntityRepository<'a> {
    database: &'a Database,
}

impl<'a> EntityRepository<'a> {
    pub fn new(database: &'a Database) -> Self {
        EntityRepository { database }
    }
}

fn id(text: &str) -> Result<Uuid, ReadError> {
    let unreadable = || ReadError::Unreadable("entity id".to_owned());
    let canonical = pyuuid::parse(text).map_err(|_| unreadable())?;
    Uuid::parse_str(&canonical).map_err(|_| unreadable())
}

impl EntityPort for EntityRepository<'_> {
    fn get_disabled_entities(&self) -> Result<Vec<Uuid>, ReadError> {
        let sqlite = |error: rusqlite::Error| {
            ReadError::Failed(core_app::ports::PortError(error.to_string()))
        };
        let mut statement = self
            .database
            .connection
            .prepare(GET_DISABLED)
            .map_err(sqlite)?;
        let rows = statement
            .query_map([], |row| row.get::<_, String>("id"))
            .map_err(sqlite)?;
        let mut entities = Vec::new();
        for row in rows {
            entities.push(id(&row.map_err(sqlite)?)?);
        }
        Ok(entities)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::Connection;

    #[test]
    fn reads_only_disabled_entities() {
        let connection = Connection::open_in_memory().unwrap();
        connection
            .execute_batch(
                "CREATE TABLE entities (id TEXT, name TEXT, natural_id TEXT, type TEXT, origin TEXT, icon_url TEXT);
                 CREATE TABLE entity_accounts (id TEXT, entity_id TEXT, deleted_at TEXT);
                 CREATE TABLE external_entities (id TEXT, entity_id TEXT);
                 CREATE TABLE virtual_data_imports (import_id TEXT, entity_id TEXT, date TEXT);",
            )
            .unwrap();
        let insert = |id: &str, origin: &str, kind: &str| {
            connection
                .execute(
                    "INSERT INTO entities VALUES (?, ?, NULL, ?, ?, NULL)",
                    rusqlite::params![id, "e", kind, origin],
                )
                .unwrap();
        };
        insert(
            "550e8400-e29b-41d4-a716-446655440001",
            "NATIVE",
            "FINANCIAL_INSTITUTION",
        );
        insert(
            "550e8400-e29b-41d4-a716-446655440002",
            "NATIVE",
            "CRYPTO_WALLET",
        );
        insert(
            "550e8400-e29b-41d4-a716-446655440003",
            "MANUAL",
            "FINANCIAL_INSTITUTION",
        );
        let database = Database { connection };
        let repository = EntityRepository::new(&database);
        assert_eq!(
            repository.get_disabled_entities().unwrap(),
            vec![
                "550e8400-e29b-41d4-a716-446655440001"
                    .parse::<Uuid>()
                    .unwrap(),
                "550e8400-e29b-41d4-a716-446655440003"
                    .parse::<Uuid>()
                    .unwrap(),
            ]
        );
    }
}
