// SPDX-License-Identifier: AGPL-3.0-only

//! Error responses in the shapes the Python backend uses.

use axum::Json;
use axum::http::header::RETRY_AFTER;
use axum::http::{HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use serde_json::json;

/// An error response of a route served by Rust.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ApiError {
    /// 401 from the session guard (`api_session_guard.py`).
    NotLogged,
    /// 400 `INVALID_REQUEST` with a message.
    InvalidRequest(String),
    /// 400 with only a message, as the loan route reports missing fields.
    BadRequestMessage(String),
    /// 408 when a request body does not arrive in time.
    RequestTimeout,
    /// 413 when the body exceeds the size the backend accepts.
    PayloadTooLarge,
    /// 500 `UNEXPECTED_ERROR` (`exception_handler.py`).
    Unexpected(String),
    /// 502 when the Python backend cannot be reached.
    BackendUnavailable,
    /// 503 when no calculation permit became free in time.
    Busy,
    /// 504 when the Python backend did not answer in time.
    BackendTimeout,
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let (status, body) = match self {
            ApiError::NotLogged => (
                StatusCode::UNAUTHORIZED,
                json!({"code": "NOT_LOGGED", "message": "Session required"}),
            ),
            ApiError::InvalidRequest(message) => (
                StatusCode::BAD_REQUEST,
                json!({"code": "INVALID_REQUEST", "message": message}),
            ),
            ApiError::BadRequestMessage(message) => {
                (StatusCode::BAD_REQUEST, json!({"message": message}))
            }
            ApiError::RequestTimeout => (
                StatusCode::REQUEST_TIMEOUT,
                json!({"code": "REQUEST_TIMEOUT", "message": "The request body did not arrive in time"}),
            ),
            ApiError::PayloadTooLarge => (
                StatusCode::PAYLOAD_TOO_LARGE,
                json!({"code": "PAYLOAD_TOO_LARGE", "message": "Request body too large"}),
            ),
            ApiError::Unexpected(message) => (
                StatusCode::INTERNAL_SERVER_ERROR,
                json!({"code": "UNEXPECTED_ERROR", "message": message}),
            ),
            ApiError::BackendUnavailable => (
                StatusCode::BAD_GATEWAY,
                json!({"code": "BACKEND_UNAVAILABLE", "message": "The backend is not reachable"}),
            ),
            ApiError::BackendTimeout => (
                StatusCode::GATEWAY_TIMEOUT,
                json!({"code": "BACKEND_TIMEOUT", "message": "The backend did not answer in time"}),
            ),
            ApiError::Busy => {
                let body =
                    json!({"code": "SERVER_BUSY", "message": "Too many calculations are running"});
                let mut response = (StatusCode::SERVICE_UNAVAILABLE, Json(body)).into_response();
                response
                    .headers_mut()
                    .insert(RETRY_AFTER, HeaderValue::from_static("5"));
                return response;
            }
        };
        (status, Json(body)).into_response()
    }
}
