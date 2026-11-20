// SPDX-License-Identifier: AGPL-3.0-only

//! `GET /api/v1/flows/periodic`, ported from
//! `finanze/infrastructure/controller/routes/get_periodic_flows.py`.
//!
//! Python reads the flows from the database on every request, as Rust does, so a
//! flow Python creates, changes or deletes is read by the next request to either.
//! A flow stored in a form Python's writer does not produce goes to Python
//! (`ReadError::Unreadable`), which reads it with its own coercions; so does a
//! next date Python's date arithmetic cannot compute.

use axum::extract::{Request, State};
use axum::http::header::CONTENT_TYPE;
use axum::http::request::Parts;
use axum::response::{IntoResponse, Response};
use core_app::ports::ReadError;
use core_app::use_cases::get_periodic_flows::GetPeriodicFlows;
use core_domain::Dezimal;
use core_domain::dates::Date;
use core_domain::earnings_expenses::PeriodicFlow;
use infra_db::Database;
use infra_db::periodic_flows::PeriodicFlowRepository;
use serde_json::{Value, json};

use crate::app::AppState;
use crate::error::ApiError;
use crate::routes::database::{self, Reply};

pub const PATH: &str = "/api/v1/flows/periodic";

/// `GET /api/v1/flows/periodic`: every periodic flow with its next date.
pub async fn get_periodic_flows(State(state): State<AppState>, request: Request) -> Response {
    database::serve(&state, request, prepare).await
}

fn prepare(
    state: &AppState,
    _parts: &Parts,
) -> Result<impl FnOnce(&Database) -> Reply + Send + use<>, ApiError> {
    let clock = state.clock.clone();
    Ok(move |database: &Database| {
        let repository = PeriodicFlowRepository::new(database);
        match GetPeriodicFlows::new(&repository, clock.as_ref()).execute() {
            Ok(flows) => Reply::Answer(respond(&flows)),
            Err(ReadError::Unreadable(reason)) => Reply::PassOn(reason),
            Err(ReadError::Failed(error)) => {
                tracing::error!(%error, "reading the periodic flows failed");
                Reply::Answer(ApiError::Unexpected(error.0).into_response())
            }
        }
    })
}

/// `date.isoformat()`.
pub(crate) fn iso_date(date: Option<Date>) -> Value {
    date.map_or(Value::Null, |date| {
        Value::String(format!(
            "{:04}-{:02}-{:02}",
            date.year(),
            date.month(),
            date.day()
        ))
    })
}

/// An amount as `jsonify` writes a `Dezimal`: a float.
fn amount(amount: Dezimal) -> Value {
    json!(amount.to_f64_for_api())
}

/// One flow as the Python route builds it.
pub fn flow_json(flow: &PeriodicFlow) -> Value {
    json!({
        "id": flow.id,
        "name": flow.name,
        "amount": amount(flow.amount),
        "currency": flow.currency,
        "flow_type": flow.flow_type.value(),
        "frequency": flow.frequency.value(),
        "category": flow.category,
        "label_id": flow.label_id,
        "enabled": flow.enabled,
        "since": iso_date(Some(flow.since)),
        "until": iso_date(flow.until),
        "linked": flow.linked,
        "real_estate_flow": flow.real_estate_flow.as_ref().map(|info| json!({
            "flow_subtype": info.flow_subtype,
            "linked_loan_hash": info.linked_loan_hash,
        })),
        "next_date": iso_date(flow.next_date),
        "max_amount": flow.max_amount.map_or(Value::Null, amount),
        "icon": flow.icon,
    })
}

/// `jsonify([...])`: sorted keys and a newline.
fn respond(flows: &[PeriodicFlow]) -> Response {
    let body = Value::Array(flows.iter().map(flow_json).collect());
    ([(CONTENT_TYPE, "application/json")], format!("{body}\n")).into_response()
}
