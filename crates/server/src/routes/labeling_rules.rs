// SPDX-License-Identifier: AGPL-3.0-only

//! `GET /api/v1/labeling/rules`, ported from
//! `finanze/infrastructure/controller/routes/get_labeling_rules.py`.
//!
//! Python reads the rules from the database on every request, as Rust does, so a
//! rule Python creates, changes or deletes is read by the next request to either.
//! A rule stored in a form Python's writer does not produce goes to Python
//! (`ReadError::Unreadable`), which reads it with its own coercions.

use axum::extract::{Request, State};
use axum::http::header::CONTENT_TYPE;
use axum::http::request::Parts;
use axum::response::{IntoResponse, Response};
use core_app::ports::ReadError;
use core_app::use_cases::get_labeling_rules::GetLabelingRules;
use core_domain::dates::Date;
use core_domain::labeling::{LabelingRule, LabelingRuleConditions, TextCondition};
use infra_db::Database;
use infra_db::labeling_rules::LabelingRuleRepository;
use serde_json::{Value, json};

use crate::app::AppState;
use crate::error::ApiError;
use crate::routes::database::{self, Reply};

pub const PATH: &str = "/api/v1/labeling/rules";

/// `GET /api/v1/labeling/rules`: every rule, enabled or not.
pub async fn get_labeling_rules(State(state): State<AppState>, request: Request) -> Response {
    database::serve(&state, request, prepare).await
}

fn prepare(
    _state: &AppState,
    _parts: &Parts,
) -> Result<impl FnOnce(&Database) -> Reply + Send + use<>, ApiError> {
    Ok(|database: &Database| {
        let repository = LabelingRuleRepository::new(database);
        match GetLabelingRules::new(&repository).execute() {
            Ok(rules) => Reply::Answer(respond(&rules)),
            Err(ReadError::Unreadable(reason)) => Reply::PassOn(reason),
            Err(ReadError::Failed(error)) => {
                tracing::error!(%error, "reading the labeling rules failed");
                Reply::Answer(ApiError::Unexpected(error.0).into_response())
            }
        }
    })
}

/// `date.isoformat()`.
fn iso_date(date: Option<Date>) -> Value {
    date.map_or(Value::Null, |date| {
        Value::String(format!(
            "{:04}-{:02}-{:02}",
            date.year(),
            date.month(),
            date.day()
        ))
    })
}

fn text_json(text: &TextCondition) -> Value {
    json!({
        "values": text.values,
        "operator": text.operator.value(),
        "field": text.field.value(),
    })
}

/// A group of conditions as `jsonify` writes the dataclass; amounts are floats.
pub fn conditions_json(conditions: &LabelingRuleConditions) -> Value {
    let amount = |amount: Option<core_domain::Dezimal>| {
        amount.map_or(Value::Null, |amount| json!(amount.to_f64_for_api()))
    };
    json!({
        "types": conditions
            .types
            .as_ref()
            .map(|types| types.iter().map(|tx_type| tx_type.value()).collect::<Vec<_>>()),
        "min_amount": amount(conditions.min_amount),
        "max_amount": amount(conditions.max_amount),
        "currency": conditions.currency,
        "from_date": iso_date(conditions.from_date),
        "to_date": iso_date(conditions.to_date),
        "day_from": conditions.day_from,
        "day_to": conditions.day_to,
        "entities": conditions.entities,
        "text": conditions.text.as_ref().map_or(Value::Null, text_json),
        "max_days": conditions.max_days,
        "ibans": conditions.ibans,
    })
}

fn rule_json(rule: &LabelingRule) -> Value {
    json!({
        "id": rule.id,
        "conditions": rule.conditions.iter().map(conditions_json).collect::<Vec<_>>(),
        "label_ids": rule.label_ids,
        "name": rule.name,
        "enabled": rule.enabled,
        "kind": rule.kind.value(),
    })
}

/// `jsonify(LabelingRules(rules=...))`: sorted keys and a newline.
fn respond(rules: &[LabelingRule]) -> Response {
    let body = json!({ "rules": rules.iter().map(rule_json).collect::<Vec<_>>() });
    ([(CONTENT_TYPE, "application/json")], format!("{body}\n")).into_response()
}
