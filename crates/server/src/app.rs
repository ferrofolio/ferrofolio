// SPDX-License-Identifier: AGPL-3.0-only

//! The HTTP application: ported routes plus forwarding of everything else.

use std::num::NonZeroUsize;
use std::sync::Arc;
use std::time::Duration;

use axum::Router;
use axum::extract::{Request, State};
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
    pub fn new(backend: Backend, clock: Arc<dyn Clock>) -> Self {
        let cores = std::thread::available_parallelism().map_or(1, NonZeroUsize::get);
        AppState {
            backend,
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
/// own `Allow` header.
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
        .with_state(state.clone());
    let canonical = ported.clone();
    ported.fallback(move |request: Request| dispatch(state.clone(), canonical.clone(), request))
}

/// Requests whose raw path is not a ported route. The backend decodes
/// percent-escapes in the path before routing, so a ported route spelled with
/// escapes (`/api/v1/calculation/%6Coan`) is still served by Rust. Everything else
/// goes to the backend unchanged.
async fn dispatch(state: AppState, ported: Router, request: Request) -> Response {
    let raw_path = request.uri().path();
    let decoded = percent_decode(raw_path);
    let is_ported = decoded != raw_path
        && PORTED_ROUTES
            .iter()
            .any(|route| route.method == request.method() && route.path == decoded);
    let canonical_uri = match request.uri().query() {
        Some(query) => format!("{decoded}?{query}"),
        None => decoded,
    }
    .parse::<Uri>();
    match canonical_uri {
        Ok(uri) if is_ported => {
            let (mut parts, body) = request.into_parts();
            parts.uri = uri;
            match ported.oneshot(Request::from_parts(parts, body)).await {
                Ok(response) => response,
                Err(never) => match never {},
            }
        }
        _ => forward(State(state), request).await,
    }
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
