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
use core_domain::Uuid;
use core_domain::commodity::{
    COMMODITY_HISTORIC_CUTOFF, CommodityType, WeightUnit, to_troy_ounces, weight_conversion,
};
use core_domain::constants::{
    CAPITAL_GAINS_BASE_TAX, PUSD_CONTRACT_ADDRESS, PUSD_SYMBOL, SUPPORTED_CURRENCIES,
};
use core_domain::crypto::{
    AddressSource, CryptoCurrencyType, CryptoFetchResult, CryptoFetchResults,
    CryptoFetchedPosition, CryptoPositionType, CryptoWalletConnectionFailureCode, crypto_rate_key,
};
use core_domain::dates::Date;
use core_domain::datetime::DateTime;
use core_domain::dezimal::{ArithmeticError, Dezimal};
use core_domain::earnings_expenses::FlowFrequency;
use core_domain::entity::{
    Entity, EntityOrigin, EntityType, Feature, TransactionKind, is_account_scoped_entity,
};
use core_domain::external_integration::ExternalIntegrationId;
use core_domain::fetch_record::DataSource;
use core_domain::profitability::{ProfitabilityTerms, annualized_profitability};
use core_domain::public_key::{CoinType, ScriptType};
use jiff::tz::Offset;
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

fn optional_text(value: &Value) -> Option<&str> {
    if value.is_null() {
        None
    } else {
        Some(text(value))
    }
}

fn optional_dezimal(value: &Value) -> Option<Dezimal> {
    optional_text(value).map(|text| Dezimal::from_python_str(text).unwrap())
}

/// The name of the decimal signal Python raises for an error.
fn signal(error: ArithmeticError) -> &'static str {
    match error {
        ArithmeticError::Overflow => "Overflow",
        ArithmeticError::DivisionByZero => "DivisionByZero",
        ArithmeticError::InvalidOperation => "InvalidOperation",
    }
}

/// Python's outcome of a case: its `result`, or the signal it raised.
fn expected(case: &Value) -> Result<&Value, &str> {
    match case.get("error") {
        Some(error) => Err(text(error)),
        None => Ok(&case["result"]),
    }
}

fn optional_date(value: &Value) -> Option<Date> {
    optional_text(value).map(|text| text.parse().unwrap())
}

/// An ISO 8601 date and time as `datetime.fromisoformat` reads it: aware with a
/// trailing `+HH:MM` or `-HH:MM`, naive without.
fn datetime(text: &str) -> DateTime {
    let split = text.len().saturating_sub(6);
    let (civil, offset) = text.split_at(split);
    if let Some(sign) = offset
        .chars()
        .next()
        .filter(|sign| *sign == '+' || *sign == '-')
    {
        let hours: i8 = offset[1..3].parse().unwrap();
        let minutes: i8 = offset[4..6].parse().unwrap();
        assert_eq!(minutes, 0, "{text}: only whole hours are supported");
        let hours = if sign == '-' { -hours } else { hours };
        DateTime::aware(civil.parse().unwrap(), Offset::constant(hours))
    } else {
        DateTime::naive(text.parse().unwrap())
    }
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
        ("commodity.CommodityType", values::<CommodityType>()),
        ("commodity.WeightUnit", values::<WeightUnit>()),
        ("crypto.AddressSource", values::<AddressSource>()),
        ("crypto.CryptoCurrencyType", values::<CryptoCurrencyType>()),
        ("crypto.CryptoPositionType", values::<CryptoPositionType>()),
        (
            "crypto.CryptoWalletConnectionFailureCode",
            values::<CryptoWalletConnectionFailureCode>(),
        ),
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

#[test]
fn commodities() {
    assert_eq!(
        COMMODITY_HISTORIC_CUTOFF.to_string(),
        text(&cases("COMMODITY_HISTORIC_CUTOFF"))
    );
    let symbols = cases("commodity_symbols");
    assert_eq!(symbols.as_object().unwrap().len(), CommodityType::ALL.len());
    for commodity_type in CommodityType::ALL {
        assert_eq!(
            commodity_type.symbol(),
            text(&symbols[commodity_type.as_str()])
        );
    }
    let conversions = cases("weight_conversions");
    for from in WeightUnit::ALL {
        for to in WeightUnit::ALL {
            let python = conversions
                .as_array()
                .unwrap()
                .iter()
                .find(|case| case["from"] == from.as_str() && case["to"] == to.as_str());
            assert_eq!(
                weight_conversion(*from, *to),
                python.map(|case| dezimal(&case["factor"])),
                "{from} to {to}"
            );
        }
    }
}

#[test]
fn troy_ounces() {
    for case in cases("to_troy_ounces").as_array().unwrap() {
        let result = to_troy_ounces(dezimal(&case["amount"]), parse_enum(&case["unit"]));
        let expected = expected(case).map(dezimal);
        assert_eq!(result.map_err(signal), expected, "{case}");
    }
}

#[test]
fn crypto_rate_keys() {
    for case in cases("crypto_rate_key").as_array().unwrap() {
        let key = crypto_rate_key(
            parse_enum(&case["type"]),
            optional_text(&case["symbol"]),
            optional_text(&case["contract_address"]),
        );
        assert_eq!(key.as_deref(), optional_text(&case["result"]), "{case}");
    }
}

fn fetched_position(spec: &Value) -> CryptoFetchedPosition {
    let optional_string = |value: &Value| optional_text(value).map(str::to_owned);
    CryptoFetchedPosition {
        id: optional_text(&spec["id"]).map(|id| Uuid::parse_str(id).unwrap()),
        symbol: text(&spec["symbol"]).to_owned(),
        balance: dezimal(&spec["balance"]),
        r#type: parse_enum(&spec["type"]),
        name: optional_string(&spec["name"]),
        contract_address: optional_string(&spec["contract_address"]),
        chain: optional_string(&spec["chain"]),
        protocol: optional_string(&spec["protocol"]),
        position_type: parse_enum(&spec["position_type"]),
        market_value: optional_dezimal(&spec["market_value"]),
        currency: optional_string(&spec["currency"]),
        icon_url: optional_string(&spec["icon_url"]),
    }
}

/// Results as the fixture writes them: pairs of an address and a result or null.
fn fetch_results(spec: &Value) -> CryptoFetchResults {
    let results = spec
        .as_array()
        .unwrap()
        .iter()
        .map(|pair| {
            let result = (!pair[1].is_null()).then(|| CryptoFetchResult {
                address: text(&pair[1]["address"]).to_owned(),
                assets: pair[1]["assets"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(fetched_position)
                    .collect(),
                has_txs: pair[1]["has_txs"].as_bool(),
            });
            (text(&pair[0]).to_owned(), result)
        })
        .collect();
    CryptoFetchResults { results }
}

#[test]
fn adding_crypto_fetch_results() {
    for case in cases("crypto_fetch_results_add").as_array().unwrap() {
        let sum = fetch_results(&case["left"]) + fetch_results(&case["right"]);
        let expected = expected(case).map(fetch_results);
        // Maps compare like Python's dicts, whatever their order; the order of the
        // addresses is checked on its own.
        let order =
            |results: &CryptoFetchResults| results.results.keys().cloned().collect::<Vec<_>>();
        assert_eq!(
            sum.as_ref().map(order).map_err(|error| signal(*error)),
            expected.as_ref().map(order).map_err(|error| *error),
            "{}",
            case["name"]
        );
        assert_eq!(sum.map_err(signal), expected, "{}", case["name"]);
    }
}

fn places(case: &Value) -> u32 {
    u32::try_from(case["places"].as_u64().unwrap()).unwrap()
}

#[test]
fn rounding_like_python_round() {
    for case in cases("dezimal_round").as_array().unwrap() {
        let rounded = dezimal(&case["value"]).round(places(case));
        // The text too: the result has exactly the requested places.
        let rounded = rounded.map(|value| value.to_string());
        let expected = expected(case).map(|value| text(value).to_owned());
        assert_eq!(rounded.map_err(signal), expected, "{case}");
    }
}

#[test]
fn formatting_with_fixed_places() {
    for case in cases("dezimal_to_fixed").as_array().unwrap() {
        assert_eq!(
            dezimal(&case["value"]).to_fixed(places(case)),
            text(&case["result"]),
            "{case}"
        );
    }
}

#[test]
fn annualized_profitabilities() {
    let group = cases("annualized_profitability");
    let today: Date = text(&cases("today")).parse().unwrap();
    for case in group.as_array().unwrap() {
        let terms = ProfitabilityTerms {
            interest_rate: optional_dezimal(&case["interest_rate"]),
            start_dt: optional_text(&case["start_dt"]).map(datetime),
            maturity: optional_date(&case["maturity"]),
            extended_maturity: optional_date(&case["extended_maturity"]),
            extended_interest_rate: optional_dezimal(&case["extended_interest_rate"]),
            late_interest_rate: optional_dezimal(&case["late_interest_rate"]),
        };
        let result = annualized_profitability(&terms, today);
        assert_eq!(
            result.map_err(signal),
            expected(case).map(dezimal),
            "{case}"
        );
    }
}
