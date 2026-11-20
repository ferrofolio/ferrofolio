// SPDX-License-Identifier: AGPL-3.0-only

//! Authorisation of the routes served by Rust while API sessions are switched off.
//!
//! With sessions, Rust checks the session itself (`crate::sessions`). Without them,
//! a ported route still needs the data to be unlocked, which only the backend
//! knows. It asks with the client's own `Authorization` header: `GET /api/v1/status`
//! reports `UNLOCKED` exactly when the data is unlocked and the request is
//! authorised, which without sessions is every request
//! (`finanze/application/use_cases/get_status.py`).

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
    /// The user whose data is unlocked, present when the request is authorised.
    /// Read leniently, so that an unexpected shape fails only `unlocked_username`.
    #[serde(default)]
    user: Option<serde_json::Value>,
}

impl Status {
    fn unlocked(&self) -> bool {
        self.status == "UNLOCKED"
    }
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
    Ok(status(backend, headers).await?.unlocked())
}

/// The name of the user whose data the backend has unlocked, or `None` while it
/// is locked. Without API sessions every request is authorised, so the status
/// names the user (`finanze/infrastructure/controller/routes/get_status.py`);
/// with them it names none without a backend token, and this answers `None`.
pub async fn unlocked_username(backend: &Backend) -> Result<Option<String>, BackendError> {
    let status = status(backend, &HeaderMap::new()).await?;
    if !status.unlocked() {
        return Ok(None);
    }
    Ok(status
        .user
        .as_ref()
        .and_then(|user| user.get("username"))
        .and_then(serde_json::Value::as_str)
        .map(str::to_owned))
}

async fn status(backend: &Backend, headers: &HeaderMap) -> Result<Status, BackendError> {
    tokio::time::timeout(STATUS_TIMEOUT, ask_backend(backend, headers))
        .await
        .map_err(|_| BackendError::Timeout)?
}

async fn ask_backend(backend: &Backend, headers: &HeaderMap) -> Result<Status, BackendError> {
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
    serde_json::from_slice(&body).map_err(|_| BackendError::Response)
}
