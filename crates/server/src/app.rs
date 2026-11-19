// SPDX-License-Identifier: AGPL-3.0-only

//! The HTTP application: ported routes plus forwarding of everything else.

use std::num::NonZeroUsize;
use std::sync::Arc;
use std::time::Duration;

use axum::Router;
use axum::extract::Request;
use axum::http::{Method, Uri};
use axum::response::Response;
use axum::routing::{MethodFilter, MethodRouter, any};
use core_app::ports::Clock;
use core_app::services::loan_calculator::LoanCalculator;
use core_app::use_cases::calculate_loan::CalculateLoan;
use core_app::use_cases::calculate_savings::CalculateSavings;
use tokio::sync::Semaphore;
use tower::ServiceExt;

use crate::gateway::{forward, percent_decode};
use crate::proxy::Backend;
use crate::routes::{self, calculations};
use crate::sessions::Sessions;
use crate::user_data::DataDir;

/// A route served by Rust: its method and path, and how its handler is mounted.
#[derive(Debug, Clone)]
pub struct PortedRoute {
    pub method: Method,
    pub path: &'static str,
    mount: fn(MethodFilter) -> MethodRouter<AppState>,
}

/// Routes served by Rust. Every other request is forwarded to the Python backend.
///
/// Each entry removes one route from the forwarding table (docs/roadmap.md,
/// section 5); `ported_routes.json` lists the same routes for the Python test that
/// checks the backend still declares them. The Python implementation stays as the
/// reference for the parity tests until the cutover.
pub const PORTED_ROUTES: &[PortedRoute] = &[
    PortedRoute {
        method: Method::POST,
        path: calculations::LOAN_PATH,
        mount: |filter| any(forward).on(filter, routes::calculate_loan),
    },
    PortedRoute {
        method: Method::POST,
        path: calculations::SAVINGS_PATH,
        mount: |filter| any(forward).on(filter, routes::calculate_savings),
    },
];

/// Shared state of all handlers.
#[derive(Debug, Clone)]
pub struct AppState {
    pub backend: Backend,
    pub sessions: Sessions,
    /// The data directory, where a login finds the user's database. Without it,
    /// logins unlock no database for Rust.
    pub data_dir: Option<DataDir>,
    pub calculate_loan: CalculateLoan,
    pub calculate_savings: CalculateSavings,
    /// Admission of the CPU-bound work of ported routes: one permit per core, so
    /// that a burst of requests queues instead of filling the blocking thread pool.
    pub calculations: Arc<Semaphore>,
    /// How long a request waits for a calculation permit
    /// (`routes::CALCULATION_WAIT`).
    pub calculation_wait: Duration,
    /// The request bodies ported routes hold in memory at once, one permit per KiB
    /// (`routes::BODY_BUDGET_KIB`).
    pub body_budget: Arc<Semaphore>,
}

impl AppState {
    pub fn new(backend: Backend, clock: Arc<dyn Clock>, sessions: Sessions) -> Self {
        let cores = std::thread::available_parallelism().map_or(1, NonZeroUsize::get);
        AppState {
            backend,
            sessions,
            data_dir: None,
            calculate_loan: CalculateLoan::new(Arc::new(LoanCalculator::new(clock))),
            calculate_savings: CalculateSavings,
            calculations: Arc::new(Semaphore::new(cores)),
            calculation_wait: routes::CALCULATION_WAIT,
            body_budget: Arc::new(Semaphore::new(routes::BODY_BUDGET_KIB as usize)),
        }
    }
}

/// Builds the router. Other methods on a ported path go to the backend, which
/// answers CORS preflight requests and `405 Method Not Allowed` as before, with its
/// own `Allow` header; so does every path that is not ported.
pub fn router(state: AppState) -> Router {
    let ported: Router = PORTED_ROUTES
        .iter()
        .fold(Router::new(), |router, route| {
            match MethodFilter::try_from(route.method.clone()) {
                Ok(filter) => router.route(route.path, (route.mount)(filter)),
                // Only standard methods are ported; a test checks every entry.
                Err(_) => router,
            }
        })
        .fallback(forward)
        .with_state(state);
    Router::new().fallback(move |request: Request| route(ported.clone(), request))
}

/// Routes a request. The backend decodes percent-escapes in the path, collapses
/// leading slashes and ignores the case of the method before it routes, so every
/// spelling that would reach the Python handler of a ported route, such as
/// `/api/v1/calculation/%6Coan`, `//api/v1/calculation/loan` or a lowercase `post`,
/// is served by Rust under the route's own method and path, with its limits.
/// Everything else keeps the method and path the client sent.
async fn route(ported: Router, mut request: Request) -> Response {
    if let Some((method, uri)) = canonical_ported(&request) {
        *request.method_mut() = method;
        *request.uri_mut() = uri;
    }
    match ported.oneshot(request).await {
        Ok(response) => response,
        Err(never) => match never {},
    }
}

/// The ported route's method and the request's URI with the route's path, if the
/// backend would route the request to a ported route.
fn canonical_ported(request: &Request) -> Option<(Method, Uri)> {
    let decoded = percent_decode(request.uri().path());
    let path = format!("/{}", decoded.trim_start_matches('/'));
    let route = PORTED_ROUTES.iter().find(|route| {
        route.path == path
            && route
                .method
                .as_str()
                .eq_ignore_ascii_case(request.method().as_str())
    })?;
    let uri = match request.uri().query() {
        Some(query) => format!("{path}?{query}"),
        None => path,
    };
    Some((route.method.clone(), uri.parse().ok()?))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_ported_route_has_a_standard_method() {
        for route in PORTED_ROUTES {
            assert!(
                MethodFilter::try_from(route.method.clone()).is_ok(),
                "{}",
                route.path
            );
        }
    }
}
