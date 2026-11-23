// SPDX-License-Identifier: AGPL-3.0-only

//! Routes served by Rust instead of the Python backend.
//!
//! Each route keeps its Python behaviour: the session check of the API session
//! guard, made by Rust against its own sessions (`crate::sessions`), the JSON body
//! handling of Quart and the CORS headers of quart-cors.
//! Other methods on a ported path, including CORS preflight requests, are still
//! answered by the backend, which keeps the same route.

pub mod calculations;
pub mod database;
pub mod labeling_rules;
pub mod labels;
pub mod pending_flows;
pub mod periodic_flows;
pub mod settings;

use std::sync::Arc;

use axum::body::{Body, Bytes, to_bytes};
use axum::extract::{Request, State};
use axum::http::header::{
    ACCESS_CONTROL_ALLOW_ORIGIN, ACCESS_CONTROL_EXPOSE_HEADERS, CONTENT_LENGTH, CONTENT_TYPE,
    ORIGIN, VARY,
};
use axum::http::{HeaderMap, HeaderValue};
use axum::response::{IntoResponse, Response};
use serde_json::{Map, Value};
use tokio::sync::OwnedSemaphorePermit;

use crate::app::AppState;
use crate::auth::backend_authorizes;
use crate::error::ApiError;
use crate::pycompat::truthy;

/// Largest request body of a ported route. Python accepts 50 MB
/// (`MAX_CONTENT_LENGTH`), but a decoded JSON value takes many times the memory of
/// its text, and a calculator request is a few hundred bytes; a larger body is
/// refused with `413` (docs/rust-backend.md, known deviations).
pub const MAX_BODY_BYTES: usize = 1024 * 1024;
/// Bytes of request bodies, in KiB, that ported routes hold at once, from the
/// start of reading a body until it is decoded: room for 64 bodies of the largest
/// size. Requests beyond it wait. A body without `Content-Length` reserves the
/// largest size.
pub const BODY_BUDGET_KIB: u32 = 64 * 1024;
/// Requests that replace the database (a change of the
/// password) at once, the one Python works on and those waiting for it. Python
/// runs them one after another, and they are rare, so a few suffice; further ones
/// are refused with `503` rather than holding the body budget while they wait.
pub const REPLACEMENT_SLOTS: usize = 4;
/// Time a client has to send the body of a ported route or a login.
const BODY_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(60);
/// Longest wait for a free calculation permit before a request is refused with
/// `503`. Within the input limits the slowest calculation, 50 savings scenarios
/// solved over 200 years, holds its permit for a few seconds, so this lets a queue
/// of such requests drain while a flood of them cannot make requests wait without
/// end.
pub const CALCULATION_WAIT: std::time::Duration = std::time::Duration::from_secs(30);

/// `POST /api/v1/calculation/loan`.
pub async fn calculate_loan(State(state): State<AppState>, request: Request) -> Response {
    native(&state, request, calculations::calculate_loan).await
}

/// `POST /api/v1/calculations/savings`.
pub async fn calculate_savings(State(state): State<AppState>, request: Request) -> Response {
    native(&state, request, calculations::calculate_savings).await
}

/// Runs a ported route with the request handling the Python routes get.
async fn native(
    state: &AppState,
    request: Request,
    handler: fn(&AppState, &Map<String, Value>) -> Response,
) -> Response {
    let has_origin = request.headers().contains_key(ORIGIN);
    // With API sessions a session exists only while the data is unlocked. Without
    // them only the backend knows: it also unlocks and locks the data without a
    // login Rust passes on (docs/rust-backend.md, "Sessions").
    let authorization = if state.sessions.enabled() {
        Ok(state.sessions.is_valid(request.headers()))
    } else {
        backend_authorizes(&state.backend, request.headers()).await
    };
    let authorized = match authorization {
        Ok(authorized) => authorized,
        Err(error) => {
            let mut response = error.into_response();
            apply_cors(response.headers_mut(), has_origin);
            return response;
        }
    };
    let mut response = if authorized {
        match json_body(state, request).await {
            Ok((body, budget)) => compute(state, body, budget, handler).await,
            Err(error) => error.into_response(),
        }
    } else {
        ApiError::NotLogged.into_response()
    };
    apply_cors(response.headers_mut(), has_origin);
    response
}

/// Decodes the body and runs the handler. Both are CPU-bound for large inputs, so
/// they run on the blocking pool, outside the threads that serve and forward
/// requests, once a permit is free. The permit is taken only after the body is
/// read, so that a slow client cannot hold one.
async fn compute(
    state: &AppState,
    body: Option<Bytes>,
    budget: Option<OwnedSemaphorePermit>,
    handler: fn(&AppState, &Map<String, Value>) -> Response,
) -> Response {
    let failed = || ApiError::Unexpected("Calculation failed".to_owned()).into_response();
    let wait = Arc::clone(&state.calculations).acquire_owned();
    let permit = match tokio::time::timeout(state.calculation_wait, wait).await {
        Ok(Ok(permit)) => permit,
        // The semaphore is never closed, so acquiring only waits.
        Ok(Err(_)) => return failed(),
        Err(_) => return ApiError::Busy.into_response(),
    };
    let state = state.clone();
    tokio::task::spawn_blocking(move || {
        let _permit = permit;
        let fields = decode_object(body.as_deref());
        // The raw body is no longer needed once it is decoded.
        drop(body);
        drop(budget);
        match fields {
            Ok(fields) => handler(&state, &fields),
            Err(error) => error.into_response(),
        }
    })
    .await
    .unwrap_or_else(|error| {
        tracing::error!(%error, "a calculation did not complete");
        failed()
    })
}

/// The body of a request with a JSON content type, with the share of the body
/// budget it holds. Quart does not read a body that is not JSON, which gives `None`.
async fn json_body(
    state: &AppState,
    request: Request,
) -> Result<(Option<Bytes>, Option<OwnedSemaphorePermit>), ApiError> {
    if !is_json(request.headers()) {
        return Ok((None, None));
    }
    let (parts, body) = request.into_parts();
    let (bytes, budget) = budgeted_body(state, &parts.headers, body).await?;
    Ok((Some(bytes), Some(budget)))
}

/// Reads a body of at most `MAX_BODY_BYTES` within `BODY_TIMEOUT`, with the share
/// of the body budget it holds; the caller keeps the share while it holds the body.
pub(crate) async fn budgeted_body(
    state: &AppState,
    headers: &HeaderMap,
    body: Body,
) -> Result<(Bytes, OwnedSemaphorePermit), ApiError> {
    let declared = headers
        .get(CONTENT_LENGTH)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse::<usize>().ok());
    // Like Quart, a declared length above the limit is refused without reading.
    if declared.is_some_and(|length| length > MAX_BODY_BYTES) {
        return Err(ApiError::PayloadTooLarge);
    }
    let reserved = declared.unwrap_or(MAX_BODY_BYTES);
    let kib = u32::try_from(reserved.div_ceil(1024)).unwrap_or(BODY_BUDGET_KIB);
    // One deadline covers the wait for the budget and the read, so that waiting
    // behind other uploads does not extend the time a client has for its body.
    let intake = async {
        // The semaphore is never closed, so acquiring only waits.
        let budget = Arc::clone(&state.body_budget)
            .acquire_many_owned(kib.min(BODY_BUDGET_KIB))
            .await
            .map_err(|_| ApiError::Unexpected("Calculation failed".to_owned()))?;
        let bytes = read_body(body, reserved).await?;
        Ok::<_, ApiError>((bytes, budget))
    };
    tokio::time::timeout(BODY_TIMEOUT, intake)
        .await
        .map_err(|_| ApiError::RequestTimeout)?
}

/// `await request.get_json() or {}` in Quart: a body that is not JSON is ignored,
/// and empty or false values become an empty object.
fn decode_object(body: Option<&[u8]>) -> Result<Map<String, Value>, ApiError> {
    let Some(bytes) = body else {
        return Ok(Map::new());
    };
    let value: Value = serde_json::from_slice(bytes)
        .map_err(|_| ApiError::InvalidRequest("Failed to decode JSON object".to_owned()))?;
    match value {
        Value::Object(fields) => Ok(fields),
        other if !truthy(&other) => Ok(Map::new()),
        _ => Err(ApiError::InvalidRequest(
            "Request body must be a JSON object".to_owned(),
        )),
    }
}

/// Reads a body up to `limit` bytes.
pub async fn read_body(body: Body, limit: usize) -> Result<axum::body::Bytes, ApiError> {
    to_bytes(body, limit).await.map_err(|error| {
        let too_large = error
            .into_inner()
            .downcast_ref::<http_body_util::LengthLimitError>()
            .is_some();
        if too_large {
            ApiError::PayloadTooLarge
        } else {
            ApiError::InvalidRequest("Failed to read request body".to_owned())
        }
    })
}

/// Whether Quart reads the body of a request with these headers as JSON.
pub(crate) fn is_json(headers: &HeaderMap) -> bool {
    headers
        .get(CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .is_some_and(is_json_mimetype)
}

/// Quart's `is_json`: `application/json` or `application/*+json`.
fn is_json_mimetype(content_type: &str) -> bool {
    let mimetype = content_type
        .split(';')
        .next()
        .unwrap_or_default()
        .trim()
        .to_ascii_lowercase();
    mimetype == "application/json"
        || (mimetype.starts_with("application/") && mimetype.ends_with("+json"))
}

/// The headers quart-cors adds with `cors(app, expose_headers=["Content-Disposition"])`:
/// any origin is allowed, and responses to requests without an `Origin` header vary
/// by origin.
pub(crate) fn apply_cors(headers: &mut HeaderMap, has_origin: bool) {
    if has_origin {
        headers.insert(ACCESS_CONTROL_ALLOW_ORIGIN, HeaderValue::from_static("*"));
        headers.insert(
            ACCESS_CONTROL_EXPOSE_HEADERS,
            HeaderValue::from_static("Content-Disposition"),
        );
    } else {
        headers.append(VARY, HeaderValue::from_static("Origin"));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recognises_json_content_types() {
        assert!(is_json_mimetype("application/json"));
        assert!(is_json_mimetype("Application/JSON; charset=utf-8"));
        assert!(is_json_mimetype("application/merge-patch+json"));
        assert!(!is_json_mimetype("text/plain"));
        assert!(!is_json_mimetype("application/x-www-form-urlencoded"));
    }
}
