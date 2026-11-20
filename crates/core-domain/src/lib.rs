// SPDX-License-Identifier: AGPL-3.0-only

//! Domain types of Ferrofolio, the counterpart of `finanze/domain`.
//!
//! This crate has no dependencies on other layers. Types are ported from Python as
//! the routes that use them move to Rust (docs/roadmap.md, section 6).

pub mod calculations;
pub mod dates;
pub mod datetime;
pub mod dezimal;
pub mod exception;
pub mod global_position;
pub mod labeling;
pub mod loan_calculator;
pub mod pyrepr;
pub mod python_printable;
pub mod pyuuid;
pub mod settings;
pub mod text;
pub mod transactions;
pub mod unicode_digits;

pub use dezimal::Dezimal;
