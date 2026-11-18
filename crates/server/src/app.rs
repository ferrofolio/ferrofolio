// SPDX-License-Identifier: AGPL-3.0-only

//! The HTTP application. No route is served by Rust yet: every request is forwarded
//! to the Python backend (docs/roadmap.md, section 5).

use axum::Router;

use crate::gateway::forward;
use crate::proxy::Backend;

/// Shared state of all handlers.
#[derive(Debug, Clone)]
pub struct AppState {
    pub backend: Backend,
}

impl AppState {
    pub fn new(backend: Backend) -> Self {
        AppState { backend }
    }
}

/// Builds the router: everything goes to the backend.
pub fn router(state: AppState) -> Router {
    Router::new().fallback(forward).with_state(state)
}
