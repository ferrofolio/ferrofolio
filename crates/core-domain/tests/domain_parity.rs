// SPDX-License-Identifier: AGPL-3.0-only

//! The ported domain modules against Python.
//!
//! `tests/parity/generate_domain_enum_fixtures.py` records every enum of the ported
//! modules and, for every function and method with logic, inputs and the outputs
//! Python computes. Rust must have the same enums, with the same values in the same
//! order, and compute the same outputs.

// Test helpers outside #[test] functions may panic on broken fixtures.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use std::collections::BTreeMap;

use core_domain::StrEnum;
use core_domain::constants::{
    CAPITAL_GAINS_BASE_TAX, PUSD_CONTRACT_ADDRESS, PUSD_SYMBOL, SUPPORTED_CURRENCIES,
};
use core_domain::dezimal::Dezimal;
use core_domain::earnings_expenses::FlowFrequency;
use core_domain::entity::{
    Entity, EntityOrigin, EntityType, Feature, TransactionKind, is_account_scoped_entity,
};
use core_domain::external_integration::ExternalIntegrationId;
use core_domain::fetch_record::DataSource;
use core_domain::public_key::{CoinType, ScriptType};
use serde_json::Value;

const ENUMS: &str = include_str!("fixtures/domain_enums.json");
const CASES: &str = include_str!("fixtures/domain_cases.json");

fn cases(group: &str) -> Value {
    let all: Value = serde_json::from_str(CASES).unwrap();
    all.get(group)
        .unwrap_or_else(|| panic!("no case group {group}"))
        .clone()
}

fn text(value: &Value) -> &str {
    value
        .as_str()
        .unwrap_or_else(|| panic!("{value} is not a string"))
}

fn dezimal(value: &Value) -> Dezimal {
    Dezimal::from_python_str(text(value)).unwrap()
}

fn parse_enum<E: StrEnum>(value: &Value) -> E {
    E::from_value(text(value)).unwrap_or_else(|| panic!("{value} is not a {}", E::NAME))
}

fn values<E: StrEnum>() -> (&'static str, Vec<&'static str>) {
    (
        E::NAME,
        E::ALL.iter().map(|member| member.as_str()).collect(),
    )
}

/// Every ported enum, keyed as the fixture keys it: Python module and class name.
fn rust_enums() -> BTreeMap<&'static str, (&'static str, Vec<&'static str>)> {
    BTreeMap::from([
        ("earnings_expenses.FlowFrequency", values::<FlowFrequency>()),
        ("entity.EntityOrigin", values::<EntityOrigin>()),
        ("entity.EntityType", values::<EntityType>()),
        ("entity.Feature", values::<Feature>()),
        ("entity.TransactionKind", values::<TransactionKind>()),
        (
            "external_integration.ExternalIntegrationId",
            values::<ExternalIntegrationId>(),
        ),
        ("fetch_record.DataSource", values::<DataSource>()),
        ("public_key.CoinType", values::<CoinType>()),
        ("public_key.ScriptType", values::<ScriptType>()),
    ])
}

#[test]
fn enums_have_python_values_in_python_order() {
    let python: BTreeMap<String, Vec<String>> = serde_json::from_str(ENUMS).unwrap();
    let rust = rust_enums();
    let python_keys: Vec<&str> = python.keys().map(String::as_str).collect();
    let rust_keys: Vec<&str> = rust.keys().copied().collect();
    assert_eq!(
        rust_keys, python_keys,
        "the ported enums differ from Python's"
    );
    for (key, python_values) in &python {
        let (name, rust_values) = &rust[key.as_str()];
        assert_eq!(key.rsplit('.').next(), Some(*name), "{key} is named {name}");
        assert_eq!(rust_values, python_values, "values of {key}");
    }
}

#[test]
fn account_scoped_entities() {
    for case in cases("is_account_scoped_entity").as_array().unwrap() {
        let entity = Entity {
            id: None,
            name: "Entity".to_owned(),
            natural_id: None,
            r#type: parse_enum(&case["type"]),
            origin: EntityOrigin::Native,
            icon_url: None,
        };
        assert_eq!(
            is_account_scoped_entity(&entity),
            case["result"].as_bool().unwrap(),
            "{case}"
        );
    }
}

#[test]
fn constants() {
    let constants = cases("constants");
    assert_eq!(
        CAPITAL_GAINS_BASE_TAX,
        dezimal(&constants["CAPITAL_GAINS_BASE_TAX"])
    );
    let currencies: Vec<&str> = constants["SUPPORTED_CURRENCIES"]
        .as_array()
        .unwrap()
        .iter()
        .map(text)
        .collect();
    assert_eq!(SUPPORTED_CURRENCIES.to_vec(), currencies);
    assert_eq!(PUSD_SYMBOL, text(&constants["PUSD_SYMBOL"]));
    assert_eq!(
        PUSD_CONTRACT_ADDRESS,
        text(&constants["PUSD_CONTRACT_ADDRESS"])
    );
}
