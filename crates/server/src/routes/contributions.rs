// SPDX-License-Identifier: AGPL-3.0-only

//! `GET /api/v1/contributions`, ported from
//! `finanze/infrastructure/controller/routes/contributions.py`.
//!
//! Python reads the contributions from the database on every request, as Rust
//! does. A stored contribution in a form the writer never produces goes to Python.

use axum::extract::{Request, State};
use axum::http::header::CONTENT_TYPE;
use axum::http::request::Parts;
use axum::response::{IntoResponse, Response};
use core_app::ports::ReadError;
use core_app::use_cases::get_contributions::GetContributions;
use core_domain::Dezimal;
use core_domain::auto_contributions::{
    ContributionQueryRequest, EntityContributions, PeriodicContribution,
};
use core_domain::entity::Entity;
use core_domain::pyuuid;
use infra_db::Database;
use infra_db::auto_contributions::AutoContributionsRepository;
use infra_db::entity::EntityRepository;
use serde_json::{Value, json};

use crate::app::AppState;
use crate::error::ApiError;
use crate::pycompat;
use crate::routes::database::{self, Reply};

pub const PATH: &str = "/api/v1/contributions";

/// `GET /api/v1/contributions`: the periodic contributions with their next date.
pub async fn get_contributions(State(state): State<AppState>, request: Request) -> Response {
    database::serve(&state, request, prepare).await
}

fn prepare(
    state: &AppState,
    parts: &Parts,
) -> Result<impl FnOnce(&Database) -> Reply + Send + use<>, ApiError> {
    let query = parse_query(parts.uri.query().unwrap_or_default());
    let clock = state.clock.clone();
    Ok(move |database: &Database| {
        let query = match query {
            Some(query) => query,
            None => return Reply::PassOn("an entity id pydantic rejects".to_owned()),
        };
        let contributions = AutoContributionsRepository::new(database);
        let entities = EntityRepository::new(database);
        match GetContributions::new(&contributions, &entities, clock.as_ref()).execute(&query) {
            Ok(result) => match contributions_json(&result) {
                Some(body) => Reply::Answer(respond(&body)),
                None => Reply::PassOn("an amount a float cannot hold".to_owned()),
            },
            Err(ReadError::Unreadable(reason)) => Reply::PassOn(reason),
            Err(ReadError::Failed(error)) => {
                tracing::error!(%error, "reading the contributions failed");
                Reply::Answer(ApiError::Unexpected(error.0).into_response())
            }
        }
    })
}

/// The query as the Python route reads it: a list of entity filters and a `real`
/// flag that is `None` for any value other than `true` or `false`. `None` means an
/// entity filter Python's UUID validation rejects, which is left to Python.
fn parse_query(query: &str) -> Option<ContributionQueryRequest> {
    let entities = pycompat::query_values(query, "entity");
    if entities.iter().any(|value| pyuuid::parse(value).is_err()) {
        return None;
    }
    let real = pycompat::query_values(query, "real")
        .first()
        .and_then(|value| match value.to_lowercase().as_str() {
            "true" => Some(true),
            "false" => Some(false),
            _ => None,
        });
    Some(ContributionQueryRequest {
        entities: (!entities.is_empty()).then_some(entities),
        excluded_entities: None,
        real,
    })
}

/// An amount as `jsonify` writes a `Dezimal`: a float, `None` where the float is
/// not finite and Python would write `Infinity`.
fn amount(value: Dezimal) -> Option<Value> {
    let float = value.to_f64_for_api();
    float.is_finite().then(|| json!(float))
}

/// An entity as the dataclass is written by `jsonify`.
fn entity_json(entity: &Entity) -> Value {
    json!({
        "id": entity.id.map(|id| id.to_string()),
        "name": entity.name,
        "natural_id": entity.natural_id,
        "type": entity.r#type.as_str(),
        "origin": entity.origin.as_str(),
        "icon_url": entity.icon_url,
    })
}

/// A contribution as `jsonify` writes the dataclass.
fn contribution_json(contribution: &PeriodicContribution) -> Option<Value> {
    Some(json!({
        "id": contribution.id.to_string(),
        "alias": contribution.alias,
        "target": contribution.target,
        "target_name": contribution.target_name,
        "target_type": contribution.target_type.as_str(),
        "target_subtype": contribution.target_subtype.map(|value| value.as_str()),
        "amount": amount(contribution.amount)?,
        "currency": contribution.currency,
        "since": contribution.since.to_string(),
        "until": contribution.until.map(|date| date.to_string()),
        "frequency": contribution.frequency.as_str(),
        "active": contribution.active,
        "source": contribution.source.as_str(),
        "next_date": contribution.next_date.map(|date| date.to_string()),
        "entity": entity_json(&contribution.entity),
        "entity_account_id": contribution.entity_account_id.map(|id| id.to_string()),
    }))
}

/// `jsonify(result.contributions)`: the entity ids as keys, each with its list.
fn contributions_json(contributions: &EntityContributions) -> Option<Value> {
    let mut body = serde_json::Map::new();
    for (entity_id, entry) in contributions {
        body.insert(
            entity_id.clone(),
            json!({
                "periodic": entry.periodic.iter().map(contribution_json).collect::<Option<Vec<_>>>()?,
            }),
        );
    }
    Some(Value::Object(body))
}

/// `jsonify(...)`: sorted keys and a newline.
fn respond(body: &Value) -> Response {
    ([(CONTENT_TYPE, "application/json")], format!("{body}\n")).into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_the_query_like_the_python_route() {
        let id = "550e8400-e29b-41d4-a716-446655440000";
        let query = parse_query(&format!("entity={id}&entity={id}&real=TRUE&real=false")).unwrap();
        assert_eq!(query.entities, Some(vec![id.to_owned(), id.to_owned()]));
        assert_eq!(query.real, Some(true));
        assert_eq!(parse_query("real=maybe").unwrap().real, None);
        assert_eq!(parse_query("").unwrap().entities, None);
        assert!(parse_query("entity=not-a-uuid").is_none());
    }
}
