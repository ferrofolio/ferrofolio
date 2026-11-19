// SPDX-License-Identifier: AGPL-3.0-only

//! The fallback route: everything that is not served by Rust goes to the backend.
//!
//! A request with a valid session reaches the backend with the backend token in
//! place of the client's (`sessions`). Login, signup and logout pass through the
//! session handling here on their way to the backend; a login the backend accepts
//! also unlocks the user's database for Rust.

use std::convert::Infallible;
use std::net::SocketAddr;
use std::os::unix::ffi::OsStringExt;
use std::pin::Pin;
use std::task::{Context, Poll};

use axum::body::{Body, Bytes, to_bytes};
use axum::extract::{ConnectInfo, Request, State};
use axum::http::header::{AUTHORIZATION, CACHE_CONTROL, CONTENT_LENGTH, ORIGIN};
use axum::http::{HeaderValue, Method, StatusCode};
use axum::response::{IntoResponse, Response};
use hyper::body::{Body as HttpBody, Frame, SizeHint};
use serde_json::{Value, json};
use tokio::sync::OwnedSemaphorePermit;

use crate::app::AppState;
use crate::error::ApiError;
use crate::routes::{apply_cors, budgeted_body, is_json};
use crate::sessions::{Credentials, LoginAttempt, LoginBody, SessionError};
use crate::user_data::{User, UserDatabase};

/// Largest login response Rust reads; Python's carries a token of 43 characters.
const TOKEN_RESPONSE_LIMIT: usize = 64 * 1024;

/// Forwards a request to the backend.
pub async fn forward(State(state): State<AppState>, mut request: Request) -> Response {
    let client_addr = request
        .extensions()
        .get::<ConnectInfo<SocketAddr>>()
        .map(|ConnectInfo(addr)| *addr);
    let backend_authorization = state.sessions.backend_authorization(request.headers());
    let has_session = backend_authorization.is_some();
    if let Some(authorization) = backend_authorization {
        request.headers_mut().insert(AUTHORIZATION, authorization);
    }

    match session_route(request.method(), request.uri().path()) {
        Some(SessionRoute::Login) => login(&state, request, false, client_addr).await,
        Some(SessionRoute::Signup) => login(&state, request, true, client_addr).await,
        Some(SessionRoute::Logout) => logout(&state, request, has_session, client_addr).await,
        Some(SessionRoute::Import) => replace_database(&state, request, client_addr, false).await,
        Some(SessionRoute::ChangePassword) => {
            replace_database(&state, request, client_addr, true).await
        }
        None => state.backend.forward_or_error(request, client_addr).await,
    }
}

/// Passes a login or signup on and, when the backend accepts it with a token,
/// answers with a token of a new session instead of the backend's. Once the
/// backend has accepted it, Rust opens the user's database too.
async fn login(
    state: &AppState,
    request: Request,
    signup: bool,
    client_addr: Option<SocketAddr>,
) -> Response {
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
    let login = LoginBody::new(body, is_json(&parts.headers), state.sessions.enabled());
    parts
        .headers
        .insert(CONTENT_LENGTH, HeaderValue::from(login.forwarded.len()));
    let attempt = state.sessions.login_attempt();
    let LoginBody {
        forwarded,
        remember,
        credentials,
        username,
        guest,
    } = login;
    let body = Body::new(BudgetedBody {
        data: Some(forwarded),
        budget: Some(budget),
    });
    let response = match state
        .backend
        .forward(Request::from_parts(parts, body), client_addr)
        .await
    {
        Ok(response) => response,
        Err(error) => return refuse(error.into_response()),
    };
    // Without API sessions Python refuses a login while the data is unlocked with
    // 409; it may have been unlocked without a login Rust saw, by the automatic
    // login at startup or a guest's backup import. The password is checked by
    // opening the database.
    if response.status() == StatusCode::CONFLICT && !state.sessions.enabled() && !signup {
        unlock(state, attempt, credentials).await;
        return response;
    }
    if !matches!(response.status(), StatusCode::OK | StatusCode::NO_CONTENT) {
        return response;
    }
    // A guest's signup leaves the data locked; every other login or signup the
    // backend accepts has unlocked it. Python holds the named user from now on,
    // which the status reports; the database for Rust is opened separately, under
    // the replacement lock, so that it never meets a concurrent replacement.
    let unlocks = !(signup && guest);
    let user = read_user(state, username).await;
    if let Err(error) = state.sessions.accept_login(attempt, unlocks, user) {
        tracing::warn!(%error, "a login did not record Python's user and lock state");
    }
    if unlocks {
        unlock(state, attempt, credentials).await;
    }
    // Without a token: the backend runs without API sessions, a login running at
    // the same time has unlocked the data already, or a guest signed up.
    if response.status() == StatusCode::NO_CONTENT || !state.sessions.enabled() {
        let mut response = response;
        if response.status() == StatusCode::OK {
            // The response may carry a session token, which no cache may store.
            response
                .headers_mut()
                .insert(CACHE_CONTROL, HeaderValue::from_static("no-store"));
        }
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
    // Without the rewritten body the backend token may expire before the session
    // would, so a body Rust cannot read starts none.
    let Some(remember) = remember else {
        tracing::warn!("the backend accepted a login body Rust cannot read");
        let message = "Failed to decode JSON object";
        return refuse(ApiError::InvalidRequest(message.to_owned()).into_response());
    };
    let token = match state.sessions.start(attempt, remember, &backend_token) {
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

/// Opens the database of the user a login names, read-only and with the login's
/// password, as the backend has just done, and keeps it; the backend owns the
/// schema and has migrated it before it answered. Without it, the login still
/// succeeds.
async fn unlock(state: &AppState, attempt: LoginAttempt, credentials: Option<Credentials>) {
    let (Some(data_dir), Some(credentials)) = (state.data_dir.clone(), credentials) else {
        return;
    };
    let sessions = state.sessions.clone();
    // Deriving the key takes a while, so it runs on the blocking pool.
    let unlocked = tokio::task::spawn_blocking(move || {
        sessions.open_and_unlock(attempt, || UserDatabase::open(&data_dir, credentials))
    })
    .await;
    match unlocked {
        Ok(Ok(())) => tracing::info!("unlocked the database read-only"),
        Ok(Err(error)) => tracing::warn!(%error, "a login did not unlock the database for Rust"),
        Err(error) => tracing::error!(%error, "opening the database did not complete"),
    }
}

/// The user a login or signup names, as Python's `DBManager` holds them once it has
/// accepted the request: the first profile with that name in `profiles.json`.
/// Reading the file takes a while, so it runs on the blocking pool.
async fn read_user(state: &AppState, username: Option<String>) -> Option<User> {
    let (Some(data_dir), Some(username)) = (state.data_dir.clone(), username) else {
        return None;
    };
    tokio::task::spawn_blocking(move || {
        let profile = data_dir.profile(&username)?;
        let cwd = std::env::current_dir()
            .ok()
            .map(|directory| directory.into_os_string().into_vec());
        data_dir.user_of(&profile, cwd.as_deref())
    })
    .await
    .ok()
    .flatten()
}

/// Unlocks the data for Rust as Python's automatic login at startup (`USERNAME`
/// and `PASSWORD`) did, once Python has logged in with them.
pub async fn unlock_at_startup(state: &AppState, credentials: Credentials) {
    let attempt = state.sessions.login_attempt();
    let user = read_user(state, Some(credentials.username.clone())).await;
    if let Err(error) = state.sessions.accept_login(attempt, true, user) {
        tracing::warn!(%error, "the automatic login did not record Python's user and lock state");
    }
    unlock(state, attempt, Some(credentials)).await;
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

/// A login body on its way to the backend, holding its share of the body budget
/// until it has been handed on, not while the backend works on the login.
struct BudgetedBody {
    data: Option<Bytes>,
    budget: Option<OwnedSemaphorePermit>,
}

impl HttpBody for BudgetedBody {
    type Data = Bytes;
    type Error = Infallible;

    fn poll_frame(
        self: Pin<&mut Self>,
        _cx: &mut Context<'_>,
    ) -> Poll<Option<Result<Frame<Bytes>, Infallible>>> {
        let this = self.get_mut();
        let frame = this.data.take().map(|data| Ok(Frame::data(data)));
        this.budget = None;
        Poll::Ready(frame)
    }

    fn is_end_stream(&self) -> bool {
        self.data.is_none()
    }

    fn size_hint(&self) -> SizeHint {
        let length = self.data.as_ref().map_or(0, Bytes::len);
        SizeHint::with_exact(u64::try_from(length).unwrap_or(u64::MAX))
    }
}

/// Passes a logout on and ends every session unless the backend refused it.
///
/// Python's logout handler revokes its sessions before anything else, so they may
/// be gone whenever the request reached it: on success, on a server error later in
/// the handler and when the answer is lost or late. A session Rust kept after
/// Python ended its own would come back with the backend token of the next login.
///
/// A logout with a valid session carries the backend token, which Python's guard
/// accepts, so Rust ends every session before it passes the logout on; a login
/// still with Python then starts none either. Any other logout ends them once
/// Python has answered, unless it refused with a `3xx` or `4xx`, as its guard does
/// while the data is unlocked. The logout runs to its end in a task of its own, so
/// that a client that goes away cannot stop Rust from ending the sessions.
async fn logout(
    state: &AppState,
    request: Request,
    has_session: bool,
    client_addr: Option<SocketAddr>,
) -> Response {
    if has_session {
        state.sessions.revoke_all();
    }
    let backend = state.backend.clone();
    let sessions = state.sessions.clone();
    let logout = tokio::spawn(async move {
        let response = backend.forward(request, client_addr).await;
        let refused = response.as_ref().is_ok_and(|response| {
            response.status().is_redirection() || response.status().is_client_error()
        });
        if !refused {
            sessions.revoke_all();
        }
        response
    });
    match logout.await {
        Ok(Ok(response)) => response,
        Ok(Err(error)) => error.into_response(),
        Err(error) => {
            tracing::error!(%error, "a logout did not complete");
            state.sessions.revoke_all();
            ApiError::Unexpected("Logout failed".to_owned()).into_response()
        }
    }
}

/// Passes on a request with which Python replaces or rekeys the database (a backup
/// import, a change of the password), with Rust's connection to the database
/// closed until Python has answered (`user_data::UserDatabase`). The body is read
/// first, so that a client that stalls while sending it holds no lock and Rust
/// keeps reading its database meanwhile. It runs to its end in a task of its own,
/// so that a client that goes away cannot reopen the connection while Python is
/// still at work. When the connection to Python fails, or after a change of the
/// password has succeeded, Rust keeps no connection at all: Python may still write
/// the file, and the credentials of a password change open it no more. The next
/// login opens it again.
async fn replace_database(
    state: &AppState,
    request: Request,
    client_addr: Option<SocketAddr>,
    rekeys: bool,
) -> Response {
    let (parts, body) = request.into_parts();
    let has_origin = parts.headers.contains_key(ORIGIN);
    let (body, budget) = match budgeted_body(state, &parts.headers, body).await {
        Ok(read) => read,
        Err(error) => {
            let mut response = error.into_response();
            apply_cors(response.headers_mut(), has_origin);
            return response;
        }
    };
    let backend = state.backend.clone();
    let sessions = state.sessions.clone();
    let replacement = tokio::spawn(async move {
        let guard = sessions.close_for_replacement().await;
        let response = match backend
            .forward(Request::from_parts(parts, Body::from(body)), client_addr)
            .await
        {
            Ok(response) => response,
            Err(error) => {
                sessions.forget_database();
                drop(budget);
                return error.into_response();
            }
        };
        if rekeys && response.status().is_success() {
            sessions.forget_database();
        }
        drop(guard);
        drop(budget);
        response
    });
    replacement.await.unwrap_or_else(|error| {
        tracing::error!(%error, "a request that replaces the database did not complete");
        ApiError::Unexpected("Request failed".to_owned()).into_response()
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SessionRoute {
    /// `POST /api/v1/login`, which answers with a token.
    Login,
    /// `POST /api/v1/signup`, which answers with a token unless it is a guest's.
    Signup,
    /// `POST /api/v1/logout`.
    Logout,
    /// `POST /api/v1/cloud/backup/import`, with which Python replaces the
    /// database and keeps its password.
    Import,
    /// `POST /api/v1/change-password`, with which Python rekeys the database and
    /// locks it.
    ChangePassword,
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
        "api/v1/login" => Some(SessionRoute::Login),
        "api/v1/signup" => Some(SessionRoute::Signup),
        "api/v1/logout" => Some(SessionRoute::Logout),
        "api/v1/cloud/backup/import" => Some(SessionRoute::Import),
        "api/v1/change-password" => Some(SessionRoute::ChangePassword),
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
        use SessionRoute::{ChangePassword, Import, Login, Logout, Signup};
        assert_eq!(session_route(&Method::POST, "/api/v1/login"), Some(Login));
        assert_eq!(session_route(&Method::POST, "/api/v1/signup"), Some(Signup));
        assert_eq!(session_route(&Method::POST, "/api/v1/logout"), Some(Logout));
        assert_eq!(session_route(&Method::POST, "/api/v1/%6Cogin"), Some(Login));
        // Other spellings that may still reach the backend's handler.
        let lowercase = Method::from_bytes(b"post").unwrap();
        assert_eq!(session_route(&lowercase, "/api/v1/login"), Some(Login));
        assert_eq!(session_route(&Method::POST, "//api/v1/login"), Some(Login));
        assert_eq!(
            session_route(&Method::POST, "/%2Fapi/v1/signup"),
            Some(Signup)
        );
        assert_eq!(session_route(&lowercase, "/api/v1/logou%74"), Some(Logout));
        assert_eq!(session_route(&Method::GET, "/api/v1/login"), None);
        assert_eq!(session_route(&Method::GET, "/api/v1/logout"), None);
        assert_eq!(
            session_route(&Method::POST, "/api/v1/cloud/backup/import"),
            Some(Import)
        );
        assert_eq!(
            session_route(&lowercase, "//api/v1/change-%70assword"),
            Some(ChangePassword)
        );
        assert_eq!(
            session_route(&Method::GET, "/api/v1/cloud/backup/import"),
            None
        );
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
