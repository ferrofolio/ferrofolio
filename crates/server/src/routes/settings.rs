// SPDX-License-Identifier: AGPL-3.0-only

//! `GET /api/v1/settings`, ported from
//! `finanze/infrastructure/controller/routes/get_settings.py`.
//!
//! Python reads the settings from the database on every request, as Rust does, so
//! a change Python writes with `POST /api/v1/settings`, which stays in Python, is
//! read by the next request to either.

use std::collections::BTreeSet;
use std::sync::Arc;

use axum::extract::{Request, State};
use axum::http::header::CONTENT_TYPE;
use axum::http::request::Parts;
use axum::response::{IntoResponse, Response};
use core_app::use_cases::get_settings::GetSettings;
use core_domain::settings::{PartialSettings, SettingsSection};
use infra_db::Database;
use infra_db::settings::SettingsRepository;

use crate::app::AppState;
use crate::error::ApiError;
use crate::pycompat;
use crate::routes::database;

pub const PATH: &str = "/api/v1/settings";

/// `GET /api/v1/settings`: the sections named by `key`, all without one.
pub async fn get_settings(State(state): State<AppState>, request: Request) -> Response {
    database::serve(&state, request, prepare).await
}

fn prepare(
    state: &AppState,
    parts: &Parts,
) -> Result<impl FnOnce(&Database) -> Response + Send + use<>, ApiError> {
    let sections = sections(parts.uri.query().unwrap_or_default())?;
    let base = Arc::clone(&state.settings_base);
    let time_zone = state.time_zone.clone();
    Ok(move |database: &Database| {
        let repository = SettingsRepository::new(database, &base, &time_zone);
        match GetSettings::new(&repository).execute(&sections) {
            Ok(settings) => respond(&settings),
            Err(error) => {
                tracing::error!(%error, "reading the settings failed");
                ApiError::Unexpected(error.0).into_response()
            }
        }
    })
}

/// `{SettingsSection(key) for key in request.args.getlist("key")}`: the first
/// value that is not a section fails with Python's `ValueError` message.
fn sections(query: &str) -> Result<BTreeSet<SettingsSection>, ApiError> {
    pycompat::query_values(query, "key")
        .iter()
        .map(|value| {
            SettingsSection::from_value(value).ok_or_else(|| {
                ApiError::InvalidRequest(format!(
                    "{} is not a valid SettingsSection",
                    pycompat::repr_str(value)
                ))
            })
        })
        .collect()
}

/// `jsonify` of the settings without the sections not asked for: sorted keys and a
/// newline.
fn respond(settings: &PartialSettings) -> Response {
    match serde_json::to_value(settings) {
        Ok(body) => ([(CONTENT_TYPE, "application/json")], format!("{body}\n")).into_response(),
        Err(error) => {
            tracing::error!(%error, "could not serialize the settings");
            ApiError::Unexpected("Could not serialize the settings".to_owned()).into_response()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_the_sections_like_the_python_route() {
        let all = sections("").unwrap();
        assert!(all.is_empty());
        let read = sections("key=ai&other=1&key=%67eneral&key=ai").unwrap();
        assert_eq!(
            read.into_iter().collect::<Vec<_>>(),
            [SettingsSection::General, SettingsSection::Ai]
        );
        assert_eq!(
            sections("key=general&key=it%27s&key=x"),
            Err(ApiError::InvalidRequest(
                "\"it's\" is not a valid SettingsSection".to_owned()
            ))
        );
    }
}
