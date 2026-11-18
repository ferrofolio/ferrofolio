// SPDX-License-Identifier: AGPL-3.0-only

//! Use cases and ports of Ferrofolio, the counterpart of `finanze/application`.
//!
//! Depends on `core-domain` only. Adapters that implement the ports live in the
//! infrastructure crates.

pub mod ports;
pub mod services;
pub mod use_cases;
