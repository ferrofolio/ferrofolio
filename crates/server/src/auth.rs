// SPDX-License-Identifier: AGPL-3.0-only

//! Authorisation of the routes served by Rust.
//!
//! The Python backend issues and checks the API session tokens and stays the only
//! place that knows them. A ported route asks it with the client's own
//! `Authorization` header: `GET /api/v1/status` reports `UNLOCKED` exactly when the
//! data is unlocked and the request is authorised, either by a valid session token
//! or because API sessions are switched off (`finanze/application/use_cases/
//! get_status.py`). Rust keeps no session state, so it can never disagree with
//! Python about a token.

use std::time::Duration;

use axum::body::{Body, to_bytes};
use axum::http::header::AUTHORIZATION;
use axum::http::{HeaderMap, Request, StatusCode};
use serde::Deserialize;

use crate::proxy::{Backend, BackendError};

/// Upper bound for the status response.
const STATUS_RESPONSE_LIMIT: usize = 256 * 1024;
/// Time the backend has to answer the status request completely.
const STATUS_TIMEOUT: Duration = Duration::from_secs(30);

#[derive(Deserialize)]
struct Status {
    status: String,
}

/// Whether the backend would accept this request on a route that needs a session.
///
/// A locked backend counts as not authorised: the ported routes always need the
/// data to be unlocked and a session, even though Python's session guard lets
/// every request through while the data is locked (a documented deviation).
pub async fn backend_authorizes(
    backend: &Backend,
    headers: &HeaderMap,
) -> Result<bool, BackendError> {
    tokio::time::timeout(STATUS_TIMEOUT, ask_backend(backend, headers))
        .await
        .map_err(|_| BackendError::Timeout)?
}

async fn ask_backend(backend: &Backend, headers: &HeaderMap) -> Result<bool, BackendError> {
    let mut request = Request::get("/api/v1/status")
        .body(Body::empty())
        .map_err(|_| BackendError::Request)?;
    if let Some(authorization) = headers.get(AUTHORIZATION) {
        request
            .headers_mut()
            .insert(AUTHORIZATION, authorization.clone());
    }
    let response = backend.forward(request, None).await?;
    // Python answers the status with 200 whether or not the request is authorised;
    // anything else is a failure of the backend, not a denial.
    if response.status() != StatusCode::OK {
        return Err(BackendError::Status(response.status()));
    }
    let body = to_bytes(response.into_body(), STATUS_RESPONSE_LIMIT)
        .await
        .map_err(|_| BackendError::Response)?;
    // A status that cannot be read is a failure of the backend, not a denial.
    let status: Status = serde_json::from_slice(&body).map_err(|_| BackendError::Response)?;
    Ok(status.status == "UNLOCKED")
}
