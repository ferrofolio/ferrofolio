// SPDX-License-Identifier: AGPL-3.0-only

//! Forwarding of requests to the Python backend.
//!
//! Requests are passed on as they are: method, path, query, headers including
//! `Host` and `Authorization`, and a streamed body. Only hop-by-hop headers are
//! removed and the client address is appended to `X-Forwarded-For`, which the
//! backend's uvicorn server trusts from the loopback interface. Responses stream
//! back unchanged, so long-running requests and event streams keep working.

use std::net::SocketAddr;
use std::time::Duration;

use axum::body::{Body, HttpBody};
use axum::http::header::{self, HeaderMap, HeaderName, HeaderValue};
use axum::http::uri::{Authority, PathAndQuery, Scheme};
use axum::http::{Request, Response, Uri, Version};
use axum::response::IntoResponse;
use hyper_util::client::legacy::Client;
use hyper_util::client::legacy::connect::HttpConnector;
use hyper_util::rt::TokioExecutor;

use crate::error::ApiError;

/// Headers that apply to a single connection and must not be forwarded (RFC 9110,
/// section 7.6.1).
const HOP_BY_HOP: [HeaderName; 8] = [
    header::CONNECTION,
    HeaderName::from_static("keep-alive"),
    header::PROXY_AUTHENTICATE,
    header::PROXY_AUTHORIZATION,
    header::TE,
    header::TRAILER,
    header::TRANSFER_ENCODING,
    header::UPGRADE,
];

const X_FORWARDED_FOR: HeaderName = HeaderName::from_static("x-forwarded-for");

/// The Python backend.
#[derive(Debug, Clone)]
pub struct Backend {
    client: Client<HttpConnector, Body>,
    scheme: Scheme,
    authority: Authority,
}

/// The backend could not be reached or did not answer.
#[derive(Debug, thiserror::Error)]
pub enum BackendError {
    #[error("backend request failed: {0}")]
    Connection(#[from] hyper_util::client::legacy::Error),
}

impl IntoResponse for BackendError {
    fn into_response(self) -> axum::response::Response {
        tracing::error!("{self}");
        ApiError::BackendUnavailable.into_response()
    }
}

/// The backend address is not a plain `http://host:port` URL.
#[derive(Debug, thiserror::Error)]
#[error("invalid backend URL, expected http://host:port with a loopback host")]
pub struct InvalidBackendUrl;

impl Backend {
    /// A backend at `http://host:port`.
    ///
    /// Requests carry session tokens and credentials in clear text, so the backend
    /// has to be on the loopback interface, given as an IP address with a port.
    pub fn new(base_url: &str) -> Result<Self, InvalidBackendUrl> {
        let uri: Uri = base_url.parse().map_err(|_| InvalidBackendUrl)?;
        let scheme = uri.scheme().cloned().ok_or(InvalidBackendUrl)?;
        if scheme != Scheme::HTTP {
            return Err(InvalidBackendUrl);
        }
        let authority = uri.authority().cloned().ok_or(InvalidBackendUrl)?;
        // Without a port the client would silently use port 80.
        if authority.port_u16().is_none() || !is_loopback_host(authority.host()) {
            return Err(InvalidBackendUrl);
        }
        if uri
            .path_and_query()
            .is_some_and(|path| path.as_str() != "/")
        {
            return Err(InvalidBackendUrl);
        }

        let mut connector = HttpConnector::new();
        connector.set_connect_timeout(Some(Duration::from_secs(5)));
        connector.set_nodelay(true);
        let client = Client::builder(TokioExecutor::new()).build(connector);
        Ok(Backend {
            client,
            scheme,
            authority,
        })
    }

    /// Forwards a request and returns the backend's response.
    ///
    /// There is deliberately no deadline for the response: the Python backend has
    /// none either, and a fetch from a bank or an event stream can take minutes.
    pub async fn forward(
        &self,
        request: Request<Body>,
        client_addr: Option<SocketAddr>,
    ) -> Result<Response<Body>, BackendError> {
        let (mut parts, body) = request.into_parts();

        let mut target = Uri::builder()
            .scheme(self.scheme.clone())
            .authority(self.authority.clone());
        target = match parts.uri.path_and_query() {
            Some(path_and_query) => target.path_and_query(path_and_query.clone()),
            None => target.path_and_query(PathAndQuery::from_static("/")),
        };

        // HTTP/2 requests carry the host in the URI instead of a Host header.
        if !parts.headers.contains_key(header::HOST)
            && let Some(authority) = parts.uri.authority()
            && let Ok(host) = HeaderValue::from_str(authority.as_str())
        {
            parts.headers.insert(header::HOST, host);
        }

        parts.uri = match target.build() {
            Ok(uri) => uri,
            // Every component is already valid, so this cannot happen.
            Err(_) => return Ok(ApiError::BackendUnavailable.into_response()),
        };
        parts.version = Version::HTTP_11;
        remove_hop_by_hop(&mut parts.headers);
        // Transfer-Encoding is hop-by-hop, so a chunked request loses its framing.
        // Without a length, hyper sends a GET, HEAD or CONNECT with an empty body;
        // asking for chunked framing again forwards the body as Python receives it.
        if !body.is_end_stream()
            && body.size_hint().exact().is_none()
            && !parts.headers.contains_key(header::CONTENT_LENGTH)
        {
            parts.headers.insert(
                header::TRANSFER_ENCODING,
                HeaderValue::from_static("chunked"),
            );
        }
        if let Some(addr) = client_addr {
            append_forwarded_for(&mut parts.headers, addr);
        }

        let response = self
            .client
            .request(Request::from_parts(parts, body))
            .await?;
        let (mut parts, body) = response.into_parts();
        remove_hop_by_hop(&mut parts.headers);
        Ok(Response::from_parts(parts, Body::new(body)))
    }

    /// Forwards a request and turns a connection failure into a 502 response.
    pub async fn forward_or_error(
        &self,
        request: Request<Body>,
        client_addr: Option<SocketAddr>,
    ) -> Response<Body> {
        match self.forward(request, client_addr).await {
            Ok(response) => response,
            Err(error) => error.into_response(),
        }
    }
}

/// A loopback IP address. Names such as `localhost` are not accepted: the client
/// would connect to whatever they resolve to.
fn is_loopback_host(host: &str) -> bool {
    host.trim_start_matches('[')
        .trim_end_matches(']')
        .parse::<std::net::IpAddr>()
        .is_ok_and(|address| address.is_loopback())
}

/// Removes hop-by-hop headers, including those listed in `Connection`.
fn remove_hop_by_hop(headers: &mut HeaderMap) {
    let listed: Vec<HeaderName> = headers
        .get_all(header::CONNECTION)
        .iter()
        .filter_map(|value| value.to_str().ok())
        .flat_map(|value| value.split(','))
        .filter_map(|name| HeaderName::from_bytes(name.trim().as_bytes()).ok())
        .collect();
    for name in listed.iter().chain(HOP_BY_HOP.iter()) {
        headers.remove(name);
    }
}

/// Appends the client address to `X-Forwarded-For`, merging repeated headers.
fn append_forwarded_for(headers: &mut HeaderMap, addr: SocketAddr) {
    let mut entries: Vec<String> = headers
        .get_all(&X_FORWARDED_FOR)
        .iter()
        .filter_map(|value| value.to_str().ok())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
        .collect();
    entries.push(addr.ip().to_string());
    if let Ok(value) = HeaderValue::from_str(&entries.join(", ")) {
        headers.insert(X_FORWARDED_FOR, value);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_only_plain_http_base_urls() {
        assert!(Backend::new("http://127.0.0.1:7593").is_ok());
        assert!(Backend::new("http://127.0.0.1:7593/").is_ok());
        assert!(Backend::new("https://127.0.0.1:7593").is_err());
        assert!(Backend::new("http://127.0.0.1:7593/api").is_err());
        assert!(Backend::new("127.0.0.1:7593").is_err());
        assert!(Backend::new("http://localhost:7593").is_err());
        assert!(Backend::new("http://127.0.0.1").is_err());
        assert!(Backend::new("http://[::1]:7593").is_ok());
        assert!(Backend::new("http://127.0.0.2:7593").is_ok());
        assert!(Backend::new("http://192.168.1.10:7593").is_err());
        assert!(Backend::new("http://backend:7593").is_err());
    }

    #[test]
    fn strips_hop_by_hop_headers() {
        let mut headers = HeaderMap::new();
        headers.insert(
            header::CONNECTION,
            HeaderValue::from_static("keep-alive, x-secret-hop"),
        );
        headers.insert("x-secret-hop", HeaderValue::from_static("1"));
        headers.insert("keep-alive", HeaderValue::from_static("timeout=5"));
        headers.insert(
            header::TRANSFER_ENCODING,
            HeaderValue::from_static("chunked"),
        );
        headers.insert(header::AUTHORIZATION, HeaderValue::from_static("Bearer t"));
        remove_hop_by_hop(&mut headers);
        assert_eq!(headers.len(), 1);
        assert!(headers.contains_key(header::AUTHORIZATION));
    }

    #[test]
    fn appends_the_client_address() {
        let addr: SocketAddr = "203.0.113.7:5000".parse().unwrap();
        let mut headers = HeaderMap::new();
        append_forwarded_for(&mut headers, addr);
        assert_eq!(headers[&X_FORWARDED_FOR], "203.0.113.7");
        append_forwarded_for(&mut headers, addr);
        assert_eq!(headers[&X_FORWARDED_FOR], "203.0.113.7, 203.0.113.7");
    }
}
