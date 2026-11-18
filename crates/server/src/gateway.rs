// SPDX-License-Identifier: AGPL-3.0-only

//! The fallback route: everything that is not served by Rust goes to the backend.

use std::net::SocketAddr;

use axum::extract::{ConnectInfo, Request, State};
use axum::http::header::CACHE_CONTROL;
use axum::http::{HeaderValue, Method, StatusCode};
use axum::response::Response;

use crate::app::AppState;

/// Forwards a request to the backend.
pub async fn forward(State(state): State<AppState>, request: Request) -> Response {
    let client_addr = request
        .extensions()
        .get::<ConnectInfo<SocketAddr>>()
        .map(|ConnectInfo(addr)| *addr);
    let issues_token = issues_session_token(request.method(), request.uri().path());

    let mut response = state.backend.forward_or_error(request, client_addr).await;
    if issues_token && response.status() == StatusCode::OK {
        // The response carries a session token, which no cache may store.
        response
            .headers_mut()
            .insert(CACHE_CONTROL, HeaderValue::from_static("no-store"));
    }
    response
}

/// `POST /api/v1/login` and `POST /api/v1/signup`, which answer with a session
/// token. The backend decodes percent-escapes before routing, so the comparison
/// does too.
fn issues_session_token(method: &Method, raw_path: &str) -> bool {
    method == Method::POST
        && matches!(
            percent_decode(raw_path).as_str(),
            "/api/v1/login" | "/api/v1/signup"
        )
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
    fn recognises_the_token_routes_only_for_post() {
        assert!(issues_session_token(&Method::POST, "/api/v1/login"));
        assert!(issues_session_token(&Method::POST, "/api/v1/signup"));
        assert!(issues_session_token(&Method::POST, "/api/v1/%6Cogin"));
        assert!(!issues_session_token(&Method::GET, "/api/v1/login"));
        assert!(!issues_session_token(&Method::POST, "/api/v1/login/"));
        assert!(!issues_session_token(&Method::POST, "/api/v1/logout"));
    }
}
