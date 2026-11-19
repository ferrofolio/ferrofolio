// SPDX-License-Identifier: AGPL-3.0-only

//! Ported routes that read the user's database (docs/rust-backend.md, "Routes that
//! read the database").
//!
//! Until the cutover Python stays the authority on whether the data is unlocked: it
//! locks and unlocks it without a request Rust sees (a failed restore, a guest's
//! backup import), and Rust can have no database open while Python has (a logout
//! racing a login, a request that replaces the database). Such a route therefore
//! answers from Rust's connection only when Python reports the request as authorised
//! on unlocked data and Rust can read its database; in every other case it passes
//! the request on to Python, which answers as it always has. Rust so never answers
//! with data Python would not give, nor as locked where Python would answer.

use std::sync::Arc;

use axum::extract::{Request, State};
use axum::http::HeaderMap;
use axum::http::header::{AUTHORIZATION, ORIGIN};
use axum::http::request::Parts;
use axum::response::{IntoResponse, Response};
use infra_db::Database;

use crate::app::AppState;
use crate::auth::backend_authorizes;
use crate::error::ApiError;
use crate::gateway::forward;
use crate::proxy::BackendError;
use crate::routes::apply_cors;
use crate::user_data::UnlockError;

/// Serves a request with Rust's connection to the database, or passes it on.
///
/// `prepare` reads the request, as the Python route does before it touches the
/// database, and returns the work on the database or the error the route answers
/// with, such as `400` for an invalid parameter. The work runs on the blocking pool once a permit
/// is free, through `Sessions::with_database`.
pub(crate) async fn serve<P, W>(state: &AppState, request: Request, prepare: P) -> Response
where
    P: FnOnce(&AppState, &Parts) -> Result<W, ApiError>,
    W: FnOnce(&Database) -> Response + Send + 'static,
{
    let has_origin = request.headers().contains_key(ORIGIN);
    let answer = |mut response: Response| {
        apply_cors(response.headers_mut(), has_origin);
        response
    };
    match python_authorizes(state, request.headers()).await {
        Ok(true) => {}
        Ok(false) => return forward(State(state.clone()), request).await,
        Err(error) => return answer(error.into_response()),
    }
    let (parts, body) = request.into_parts();
    let work = match prepare(state, &parts) {
        Ok(work) => work,
        Err(error) => return answer(error.into_response()),
    };
    let wait = Arc::clone(&state.calculations).acquire_owned();
    let permit = match tokio::time::timeout(state.calculation_wait, wait).await {
        Ok(Ok(permit)) => permit,
        // The semaphore is never closed, so acquiring only waits.
        Ok(Err(_)) => {
            return answer(ApiError::Unexpected("Request failed".to_owned()).into_response());
        }
        Err(_) => return answer(ApiError::Busy.into_response()),
    };
    let sessions = state.sessions.clone();
    let read = tokio::task::spawn_blocking(move || {
        let _permit = permit;
        sessions.with_database(work)
    })
    .await;
    match read {
        Ok(Ok(response)) => answer(response),
        Ok(Err(error)) => {
            match error {
                // Locked for Rust, or a request that replaces the database is with
                // Python: Python answers from its own state.
                UnlockError::Locked | UnlockError::Replacing => {
                    tracing::debug!(%error, "passing a request on that Rust cannot read");
                }
                _ => tracing::warn!(%error, "passing a request on that Rust cannot read"),
            }
            forward(State(state.clone()), Request::from_parts(parts, body)).await
        }
        Err(error) => {
            tracing::error!(%error, "reading the database did not complete");
            answer(ApiError::Unexpected("Request failed".to_owned()).into_response())
        }
    }
}

/// Whether Python would answer the request from unlocked data: `GET /api/v1/status`
/// with the `Authorization` the request reaches Python with, the backend token of
/// a valid session or, without API sessions, the client's own header. With API
/// sessions a request without a valid session goes to Python unasked.
async fn python_authorizes(state: &AppState, headers: &HeaderMap) -> Result<bool, BackendError> {
    if !state.sessions.enabled() {
        return backend_authorizes(&state.backend, headers).await;
    }
    let Some(authorization) = state.sessions.backend_authorization(headers) else {
        return Ok(false);
    };
    let mut backend_headers = HeaderMap::new();
    backend_headers.insert(AUTHORIZATION, authorization);
    backend_authorizes(&state.backend, &backend_headers).await
}
