// SPDX-License-Identifier: AGPL-3.0-only

//! Domain types of Ferrofolio, the counterpart of `finanze/domain`.
//!
//! This crate has no dependencies on other layers. Types are ported from Python as
//! the routes that use them move to Rust (docs/roadmap.md, section 6); each module
//! is named after its Python module.

pub mod calculations;
pub mod commodity;
pub mod constants;
pub mod crypto;
pub mod dates;
pub mod datetime;
pub mod dezimal;
pub mod earnings_expenses;
pub mod entity;
pub mod entity_account;
pub mod exception;
pub mod external_integration;
pub mod fetch_record;
pub mod global_position;
pub mod labeling;
pub mod loan_calculator;
pub mod pyrepr;
pub mod python_printable;
pub mod pyuuid;
pub mod public_key;
pub mod settings;
pub mod str_enum;
pub mod text;
pub mod transactions;
pub mod unicode_digits;

pub use dezimal::Dezimal;
pub use str_enum::StrEnum;
/// Python's `uuid.UUID`, the identifier of entities, positions and records.
pub use uuid::Uuid;
