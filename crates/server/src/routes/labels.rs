// SPDX-License-Identifier: AGPL-3.0-only

//! `GET /api/v1/labels`, ported from
//! `finanze/infrastructure/controller/routes/get_labels.py`.
//!
//! Python reads the labels and their usage from the database on every request, as
//! Rust does, so a label Python creates, changes or deletes is read by the next
//! request to either.

use axum::extract::{Request, State};
use axum::http::header::CONTENT_TYPE;
use axum::http::request::Parts;
use axum::response::{IntoResponse, Response};
use core_app::use_cases::get_labels::GetLabels;
use core_domain::labeling::Label;
use infra_db::Database;
use infra_db::labeling::LabelRepository;
use serde_json::{Value, json};

use crate::app::AppState;
use crate::error::ApiError;
use crate::routes::database::{self, Reply};

pub const PATH: &str = "/api/v1/labels";

/// `GET /api/v1/labels`: every label with the number of transactions carrying it.
pub async fn get_labels(State(state): State<AppState>, request: Request) -> Response {
    database::serve(&state, request, prepare).await
}

fn prepare(
    _state: &AppState,
    _parts: &Parts,
) -> Result<impl FnOnce(&Database) -> Reply + Send + use<>, ApiError> {
    Ok(|database: &Database| {
        let repository = LabelRepository::new(database);
        let response = match GetLabels::new(&repository).execute() {
            Ok(labels) => respond(&labels),
            Err(error) => {
                tracing::error!(%error, "reading the labels failed");
                ApiError::Unexpected(error.0).into_response()
            }
        };
        Reply::from(response)
    })
}

/// A label as `jsonify` writes the dataclass.
fn label_json(label: &Label) -> Value {
    json!({
        "id": label.id,
        "key": label.key,
        "name": label.name,
        "description": label.description,
        "color": label.color,
        "icon": label.icon,
        "category": label.category.value(),
        "subcategory": label.subcategory.value(),
        "usage": label.usage,
        "protected": label.protected(),
    })
}

/// `jsonify(Labels(labels=...))`: sorted keys and a newline.
fn respond(labels: &[Label]) -> Response {
    let body = json!({ "labels": labels.iter().map(label_json).collect::<Vec<_>>() });
    ([(CONTENT_TYPE, "application/json")], format!("{body}\n")).into_response()
}
