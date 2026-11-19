// SPDX-License-Identifier: AGPL-3.0-only

//! The fallback route: everything that is not served by Rust goes to the backend.
//!
//! A request with a valid session reaches the backend with the backend token in
//! place of the client's (`sessions`). Login, signup and logout pass through the
//! session handling here on their way to the backend.

use std::net::SocketAddr;

use axum::body::{Body, to_bytes};
use axum::extract::{ConnectInfo, Request, State};
use axum::http::header::{AUTHORIZATION, CACHE_CONTROL, CONTENT_LENGTH, ORIGIN};
use axum::http::{HeaderValue, Method, StatusCode};
use axum::response::{IntoResponse, Response};
use serde_json::{Value, json};

use crate::app::AppState;
use crate::error::ApiError;
use crate::routes::{apply_cors, budgeted_body, is_json};
use crate::sessions::{LoginBody, SessionError};

/// Largest login response Rust reads; Python's carries a token of 43 characters.
const TOKEN_RESPONSE_LIMIT: usize = 64 * 1024;

/// Forwards a request to the backend.
pub async fn forward(State(state): State<AppState>, mut request: Request) -> Response {
    let client_addr = request
        .extensions()
        .get::<ConnectInfo<SocketAddr>>()
        .map(|ConnectInfo(addr)| *addr);
    if let Some(authorization) = state.sessions.backend_authorization(request.headers()) {
        request.headers_mut().insert(AUTHORIZATION, authorization);
    }

    match session_route(request.method(), request.uri().path()) {
        Some(SessionRoute::Login) if state.sessions.enabled() => {
            login(&state, request, client_addr).await
        }
        Some(SessionRoute::Login) => {
            let mut response = state.backend.forward_or_error(request, client_addr).await;
            if response.status() == StatusCode::OK {
                // The response may carry a session token, which no cache may store.
                response
                    .headers_mut()
                    .insert(CACHE_CONTROL, HeaderValue::from_static("no-store"));
            }
            response
        }
        Some(SessionRoute::Logout) => logout(&state, request, client_addr).await,
        None => state.backend.forward_or_error(request, client_addr).await,
    }
}

/// Passes a login or signup on and, when the backend accepts it with a token,
/// answers with a token of a new session instead of the backend's.
async fn login(state: &AppState, request: Request, client_addr: Option<SocketAddr>) -> Response {
    let has_origin = request.headers().contains_key(ORIGIN);
    let refuse = |mut response: Response| {
        apply_cors(response.headers_mut(), has_origin);
        response
    };
    let (mut parts, body) = request.into_parts();
    // Rust reads the body to change one member before it passes it on, with the
    // limits of the ported routes.
    let (body, budget) = match budgeted_body(state, &parts.headers, body).await {
        Ok(read) => read,
        Err(error) => return refuse(error.into_response()),
    };
    let login = LoginBody::new(body, is_json(&parts.headers));
    parts
        .headers
        .insert(CONTENT_LENGTH, HeaderValue::from(login.forwarded.len()));
    let attempt = state.sessions.login_attempt();
    let request = Request::from_parts(parts, Body::from(login.forwarded));
    let response = state.backend.forward(request, client_addr).await;
    drop(budget);
    let response = match response {
        Ok(response) => response,
        Err(error) => return refuse(error.into_response()),
    };
    // Every other answer carries no token: an error, or 204 when the backend is
    // already unlocked by a login running at the same time or the signup is a
    // guest's, which stays locked.
    if response.status() != StatusCode::OK {
        return response;
    }

    let (mut parts, body) = response.into_parts();
    let backend_token = to_bytes(body, TOKEN_RESPONSE_LIMIT)
        .await
        .ok()
        .and_then(|body| token_of(&body));
    let Some(backend_token) = backend_token else {
        // The backend's answer is not passed on: it might hold a token after all.
        tracing::error!("the backend accepted a login without answering with a token");
        return refuse(ApiError::BackendUnavailable.into_response());
    };
    let token = match state.sessions.start(
        attempt,
        login.remember,
        &backend_token,
        login.backend_token_lasting,
    ) {
        Ok(token) => token,
        Err(error @ SessionError::LoggedOut) => {
            tracing::warn!(%error, "a login ended without a session");
            let message = "A logout ended the login, log in again";
            return refuse(ApiError::Unexpected(message.to_owned()).into_response());
        }
        Err(error) => {
            tracing::error!(%error, "could not start a session");
            let message = "Could not start a session";
            return refuse(ApiError::Unexpected(message.to_owned()).into_response());
        }
    };
    // `jsonify` in Quart: compact JSON and a newline.
    let body = format!("{}\n", json!({ "token": token }));
    parts
        .headers
        .insert(CONTENT_LENGTH, HeaderValue::from(body.len()));
    // The response carries a session token, which no cache may store.
    parts
        .headers
        .insert(CACHE_CONTROL, HeaderValue::from_static("no-store"));
    Response::from_parts(parts, Body::from(body))
}

/// The `token` of `{"token": "..."}`, the answer of a login.
fn token_of(body: &[u8]) -> Option<String> {
    match serde_json::from_slice::<Value>(body).ok()? {
        Value::Object(mut fields) => match fields.remove("token")? {
            Value::String(token) => Some(token),
            _ => None,
        },
        _ => None,
    }
}

/// Passes a logout on and ends every session once the backend has ended its own.
/// Python's logout handler revokes its sessions before anything else, so they are
/// gone whenever the handler ran: on success, and on a server error that happened
/// later in the handler. A refusal of its session guard or of its routing leaves
/// them, and so does Rust.
async fn logout(state: &AppState, request: Request, client_addr: Option<SocketAddr>) -> Response {
    let response = match state.backend.forward(request, client_addr).await {
        Ok(response) => response,
        Err(error) => return error.into_response(),
    };
    let status = response.status();
    if status.is_success() || status.is_server_error() {
        state.sessions.revoke_all();
    }
    response
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SessionRoute {
    /// `POST /api/v1/login` and `POST /api/v1/signup`, which answer with a token.
    Login,
    /// `POST /api/v1/logout`.
    Logout,
}

/// The session route a request goes to. The backend decodes percent-escapes before
/// routing, so the comparison does too. It also ignores the case of the method and
/// repeated leading slashes, so that every spelling that may reach the backend's
/// handler is covered: a token the backend issues never reaches a client.
fn session_route(method: &Method, raw_path: &str) -> Option<SessionRoute> {
    if !method.as_str().eq_ignore_ascii_case("POST") {
        return None;
    }
    match percent_decode(raw_path).trim_start_matches('/') {
        "api/v1/login" | "api/v1/signup" => Some(SessionRoute::Login),
        "api/v1/logout" => Some(SessionRoute::Logout),
        _ => None,
    }
}

/// Decodes `%XX` escapes like `urllib.parse.unquote`, which uvicorn applies to the
/// request path: invalid escapes stay as they are and invalid UTF-8 is replaced.
pub(crate) fn percent_decode(raw: &str) -> String {
    let bytes = raw.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while let Some(&byte) = bytes.get(index) {
        if byte == b'%'
            && let (Some(high), Some(low)) = (
                bytes.get(index + 1).and_then(|digit| hex_value(*digit)),
                bytes.get(index + 2).and_then(|digit| hex_value(*digit)),
            )
        {
            decoded.push(high * 16 + low);
            index += 3;
        } else {
            decoded.push(byte);
            index += 1;
        }
    }
    String::from_utf8_lossy(&decoded).into_owned()
}

fn hex_value(digit: u8) -> Option<u8> {
    match digit {
        b'0'..=b'9' => Some(digit - b'0'),
        b'a'..=b'f' => Some(digit - b'a' + 10),
        b'A'..=b'F' => Some(digit - b'A' + 10),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_percent_escapes_like_uvicorn() {
        assert_eq!(percent_decode("/api/v1/logi%6E"), "/api/v1/login");
        assert_eq!(percent_decode("/api/v1%2Flogin"), "/api/v1/login");
        assert_eq!(percent_decode("/a%zz/%4"), "/a%zz/%4");
        assert_eq!(percent_decode("/%C3%A9"), "/é");
        assert_eq!(percent_decode("/%FF"), "/\u{FFFD}");
    }

    #[test]
    fn recognises_the_session_routes_only_for_post() {
        use SessionRoute::{Login, Logout};
        assert_eq!(session_route(&Method::POST, "/api/v1/login"), Some(Login));
        assert_eq!(session_route(&Method::POST, "/api/v1/signup"), Some(Login));
        assert_eq!(session_route(&Method::POST, "/api/v1/logout"), Some(Logout));
        assert_eq!(session_route(&Method::POST, "/api/v1/%6Cogin"), Some(Login));
        // Other spellings that may still reach the backend's handler.
        let lowercase = Method::from_bytes(b"post").unwrap();
        assert_eq!(session_route(&lowercase, "/api/v1/login"), Some(Login));
        assert_eq!(session_route(&Method::POST, "//api/v1/login"), Some(Login));
        assert_eq!(
            session_route(&Method::POST, "/%2Fapi/v1/signup"),
            Some(Login)
        );
        assert_eq!(session_route(&lowercase, "/api/v1/logou%74"), Some(Logout));
        assert_eq!(session_route(&Method::GET, "/api/v1/login"), None);
        assert_eq!(session_route(&Method::GET, "/api/v1/logout"), None);
        assert_eq!(session_route(&Method::POST, "/api/v1/login/"), None);
        assert_eq!(session_route(&Method::POST, "/api/v1/status"), None);
    }

    #[test]
    fn reads_the_token_of_a_login_answer() {
        assert_eq!(token_of(b"{\"token\":\"abc\"}\n"), Some("abc".to_owned()));
        assert_eq!(token_of(b"{\"token\":1}"), None);
        assert_eq!(token_of(b"{}"), None);
        assert_eq!(token_of(b"[\"abc\"]"), None);
        assert_eq!(token_of(b""), None);
    }
}
