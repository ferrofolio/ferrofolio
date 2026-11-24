// SPDX-License-Identifier: AGPL-3.0-only

//! The periodic contributions repository, the reading half of
//! `finanze/infrastructure/repository/auto_contributions/auto_contributions_repository.py`.
//!
//! Python maps each row into a pydantic dataclass, which coerces forms the writer
//! never stores. Rust reads exactly the forms the writer stores and reports any
//! other as [`ReadError::Unreadable`], so the route passes the request on to Python.

use core_app::ports::{AutoContributionsPort, ReadError};
use core_domain::Uuid;
use core_domain::auto_contributions::{
    AutoContributions, ContributionFrequency, ContributionQueryRequest, ContributionTargetSubtype,
    ContributionTargetType, EntityContributions, PeriodicContribution,
};
use core_domain::entity::{Entity, EntityOrigin, EntityType};
use core_domain::fetch_record::DataSource;
use core_domain::pyuuid;
use indexmap::IndexMap;
use rusqlite::types::{Value, ValueRef};

use crate::Database;
use crate::read::{amount, date, flag, required, text};

/// `AutoContributionsQueries.GET_ALL_GROUPED_BY_ENTITY_BASE`, without its filters.
const GET_ALL: &str = "
    SELECT e.id         as entity_id,
           e.name       as entity_name,
           e.natural_id as entity_natural_id,
           e.type       as entity_type,
           e.origin     as entity_origin,
           e.icon_url   as icon_url,
           pc.id        as pc_id,
           pc.*
    FROM periodic_contributions pc
        JOIN entities e ON pc.entity_id = e.id
        LEFT JOIN entity_accounts ea ON pc.entity_account_id = ea.id
    WHERE (pc.entity_account_id IS NULL OR ea.deleted_at IS NULL)
";

/// `AutoContributionsRepository`, for one request on the database a login opened.
#[derive(Debug)]
pub struct AutoContributionsRepository<'a> {
    database: &'a Database,
}

impl<'a> AutoContributionsRepository<'a> {
    pub fn new(database: &'a Database) -> Self {
        AutoContributionsRepository { database }
    }
}

fn unreadable(what: &str) -> ReadError {
    ReadError::Unreadable(what.to_owned())
}

/// `UUID(value)` of an id the writer stores as a string.
fn uuid(value: ValueRef<'_>, what: &str) -> Result<Uuid, ReadError> {
    let text = required(value, what).map_err(|_| unreadable(what))?;
    let canonical = pyuuid::parse(text).map_err(|_| unreadable(what))?;
    Uuid::parse_str(&canonical).map_err(|_| unreadable(what))
}

fn optional_uuid(value: ValueRef<'_>, what: &str) -> Result<Option<Uuid>, ReadError> {
    match text(value, what).map_err(|_| unreadable(what))? {
        None => Ok(None),
        Some(text) => uuid(ValueRef::Text(text.as_bytes()), what).map(Some),
    }
}

/// `_map_row_to_contribution`, for the forms the writer stores.
fn map_row(row: &rusqlite::Row<'_>) -> Result<(Uuid, Entity, PeriodicContribution), ReadError> {
    let entity_id = uuid(
        row.get_ref("entity_id")
            .map_err(|error| unreadable(&error.to_string()))?,
        "entity id",
    )?;
    let entity = Entity {
        id: Some(entity_id),
        name: required(
            row.get_ref("entity_name")
                .map_err(|e| unreadable(&e.to_string()))?,
            "entity name",
        )
        .map_err(|_| unreadable("entity name"))?
        .to_owned(),
        natural_id: text(
            row.get_ref("entity_natural_id")
                .map_err(|e| unreadable(&e.to_string()))?,
            "entity natural id",
        )
        .map_err(|_| unreadable("entity natural id"))?
        .map(str::to_owned),
        r#type: EntityType::from_value(
            required(
                row.get_ref("entity_type")
                    .map_err(|e| unreadable(&e.to_string()))?,
                "entity type",
            )
            .map_err(|_| unreadable("entity type"))?,
        )
        .ok_or_else(|| unreadable("entity type"))?,
        origin: EntityOrigin::from_value(
            required(
                row.get_ref("entity_origin")
                    .map_err(|e| unreadable(&e.to_string()))?,
                "entity origin",
            )
            .map_err(|_| unreadable("entity origin"))?,
        )
        .ok_or_else(|| unreadable("entity origin"))?,
        icon_url: text(
            row.get_ref("icon_url")
                .map_err(|e| unreadable(&e.to_string()))?,
            "entity icon",
        )
        .map_err(|_| unreadable("entity icon"))?
        .map(str::to_owned),
    };
    let get = |name: &str| {
        row.get_ref(name)
            .map_err(|error| unreadable(&error.to_string()))
    };
    let contribution = PeriodicContribution {
        id: uuid(get("pc_id")?, "contribution id")?,
        alias: text(get("alias")?, "alias")
            .map_err(|_| unreadable("alias"))?
            .map(str::to_owned),
        target: required(get("target")?, "target")
            .map_err(|_| unreadable("target"))?
            .to_owned(),
        target_name: required(get("target_name")?, "target name")
            .map_err(|_| unreadable("target name"))?
            .to_owned(),
        target_type: ContributionTargetType::from_value(
            required(get("target_type")?, "target type").map_err(|_| unreadable("target type"))?,
        )
        .ok_or_else(|| unreadable("target type"))?,
        target_subtype: match text(get("target_subtype")?, "target subtype")
            .map_err(|_| unreadable("target subtype"))?
        {
            None => None,
            Some(value) => Some(
                ContributionTargetSubtype::from_value(value)
                    .ok_or_else(|| unreadable("target subtype"))?,
            ),
        },
        amount: amount(
            required(get("amount")?, "amount").map_err(|_| unreadable("amount"))?,
            "amount",
        )
        .map_err(|_| unreadable("amount"))?,
        currency: required(get("currency")?, "currency")
            .map_err(|_| unreadable("currency"))?
            .to_owned(),
        since: date(get("since")?, "since")
            .map_err(|_| unreadable("since"))?
            .ok_or_else(|| unreadable("since"))?,
        until: date(get("until")?, "until").map_err(|_| unreadable("until"))?,
        frequency: ContributionFrequency::from_value(
            required(get("frequency")?, "frequency").map_err(|_| unreadable("frequency"))?,
        )
        .ok_or_else(|| unreadable("frequency"))?,
        active: flag(get("active")?, "active").map_err(|_| unreadable("active"))?,
        source: DataSource::from_value(
            required(get("source")?, "source").map_err(|_| unreadable("source"))?,
        )
        .ok_or_else(|| unreadable("source"))?,
        next_date: None,
        entity: entity.clone(),
        entity_account_id: optional_uuid(get("entity_account_id")?, "entity account id")?,
    };
    Ok((entity_id, entity, contribution))
}

impl AutoContributionsPort for AutoContributionsRepository<'_> {
    fn get_all_grouped_by_entity(
        &self,
        query: &ContributionQueryRequest,
    ) -> Result<EntityContributions, ReadError> {
        let mut conditions: Vec<String> = Vec::new();
        let mut values: Vec<Value> = Vec::new();
        if let Some(real) = query.real {
            if real {
                conditions.push("pc.source = 'REAL'".to_owned());
            } else {
                conditions.push("pc.source IN ('MANUAL', 'SHEETS')".to_owned());
            }
        }
        if let Some(entities) = &query.entities
            && !entities.is_empty()
        {
            conditions.push(format!(
                "pc.entity_id IN ({})",
                vec!["?"; entities.len()].join(", ")
            ));
            values.extend(entities.iter().map(|id| Value::Text(id.clone())));
        }
        if let Some(excluded) = &query.excluded_entities
            && !excluded.is_empty()
        {
            conditions.push(format!(
                "(pc.entity_id NOT IN ({}) OR pc.is_real = FALSE)",
                vec!["?"; excluded.len()].join(", ")
            ));
            values.extend(excluded.iter().map(|id| Value::Text(id.to_string())));
        }
        let sql = if conditions.is_empty() {
            GET_ALL.to_owned()
        } else {
            format!("{GET_ALL} AND {}", conditions.join(" AND "))
        };
        let sqlite = |error: rusqlite::Error| {
            ReadError::Failed(core_app::ports::PortError(error.to_string()))
        };
        let mut statement = self.database.connection.prepare(&sql).map_err(sqlite)?;
        let rows = statement
            .query_map(rusqlite::params_from_iter(values.iter()), |row| {
                Ok(map_row(row))
            })
            .map_err(sqlite)?;
        let mut contributions: IndexMap<String, AutoContributions> = IndexMap::new();
        for row in rows {
            let (entity_id, _, contribution) = row.map_err(sqlite)??;
            contributions
                .entry(entity_id.to_string())
                .or_default()
                .periodic
                .push(contribution);
        }
        Ok(contributions)
    }
}
