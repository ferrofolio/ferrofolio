// SPDX-License-Identifier: AGPL-3.0-only

//! Error responses in the shapes the Python backend uses.

use axum::Json;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde_json::json;

/// An error response of the server itself.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ApiError {
    /// 502 when the Python backend cannot be reached.
    BackendUnavailable,
    /// 504 when the Python backend did not answer in time.
    BackendTimeout,
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let (status, body) = match self {
            ApiError::BackendUnavailable => (
                StatusCode::BAD_GATEWAY,
                json!({"code": "BACKEND_UNAVAILABLE", "message": "The backend is not reachable"}),
            ),
            ApiError::BackendTimeout => (
                StatusCode::GATEWAY_TIMEOUT,
                json!({"code": "BACKEND_TIMEOUT", "message": "The backend did not answer in time"}),
            ),
        };
        (status, Json(body)).into_response()
    }
}
