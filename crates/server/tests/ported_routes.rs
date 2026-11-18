// SPDX-License-Identifier: AGPL-3.0-only

//! `ported_routes.json` lists the routes served by Rust for the Python test that
//! checks the backend still registers them (tests/unit/infrastructure/controller/
//! test_ported_routes.py): the backend answers CORS preflight requests for them and
//! is the reference of the parity tests. Both lists must agree.

use serde_json::Value;
use server::app::PORTED_ROUTES;

const LISTED: &str = include_str!("../ported_routes.json");

#[test]
fn the_route_list_for_python_matches_the_router() {
    let listed: Vec<Value> = serde_json::from_str(LISTED).unwrap_or_default();
    let listed: Vec<(String, String)> = listed
        .iter()
        .map(|route| {
            (
                route["method"].as_str().unwrap_or_default().to_owned(),
                route["path"].as_str().unwrap_or_default().to_owned(),
            )
        })
        .collect();
    let ported: Vec<(String, String)> = PORTED_ROUTES
        .iter()
        .map(|route| (route.method.to_string(), route.path.to_owned()))
        .collect();
    assert_eq!(listed, ported);
}
