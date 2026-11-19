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
use core_domain::exception::PositionError;
use core_domain::external_integration::ExternalIntegrationId;
use core_domain::fetch_record::DataSource;
use core_domain::global_position::{
    AccountType, AssetType, CardType, CreditDetail, CryptoCurrencyPosition,
    CryptoInitialInvestmentType, Deposit, DerivativeContractType, EquityType, FactoringDetail,
    FundDetail, FundType, InstallmentFrequency, InterestType, Loan, LoanType, MarginType,
    PositionDirection, ProductType, RealEstateCFDetail, StockDetail, compute_investment_hash,
    compute_loan_hash,
};
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

/// An error as the fixture names it, with the fields of a `MissingFieldsError`.
type NamedError = (String, Option<Vec<String>>);

/// The initial investment and the average buy price of a position.
type InvestmentFigures = (Option<Dezimal>, Option<Dezimal>);

/// Python's error for a case that raised: a signal, `MissingFieldsError` with its
/// fields, or `ValueError`.
fn python_error(case: &Value) -> Option<NamedError> {
    let error = text(case.get("error")?).to_owned();
    let fields = case.get("missing_fields").map(|fields| {
        fields
            .as_array()
            .unwrap()
            .iter()
            .map(|field| text(field).to_owned())
            .collect()
    });
    Some((error, fields))
}

fn position_error(error: &PositionError) -> NamedError {
    match error {
        PositionError::MissingFields(missing) => (
            "MissingFieldsError".to_owned(),
            Some(missing.missing_fields.clone()),
        ),
        PositionError::Arithmetic(signal_error) => (signal(*signal_error).to_owned(), None),
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
        ("global_position.AccountType", values::<AccountType>()),
        ("global_position.AssetType", values::<AssetType>()),
        ("global_position.CardType", values::<CardType>()),
        (
            "global_position.CryptoInitialInvestmentType",
            values::<CryptoInitialInvestmentType>(),
        ),
        (
            "global_position.DerivativeContractType",
            values::<DerivativeContractType>(),
        ),
        ("global_position.EquityType", values::<EquityType>()),
        ("global_position.FundType", values::<FundType>()),
        (
            "global_position.InstallmentFrequency",
            values::<InstallmentFrequency>(),
        ),
        ("global_position.InterestType", values::<InterestType>()),
        ("global_position.LoanType", values::<LoanType>()),
        ("global_position.MarginType", values::<MarginType>()),
        (
            "global_position.PositionDirection",
            values::<PositionDirection>(),
        ),
        ("global_position.ProductType", values::<ProductType>()),
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

#[test]
fn installment_frequencies() {
    let group = cases("installment_frequency");
    assert_eq!(
        group.as_array().unwrap().len(),
        InstallmentFrequency::ALL.len()
    );
    for case in group.as_array().unwrap() {
        let frequency: InstallmentFrequency = parse_enum(&case["value"]);
        assert_eq!(
            u64::from(frequency.payments_per_year()),
            case["payments_per_year"].as_u64().unwrap()
        );
        assert_eq!(
            frequency.flow_frequency().as_str(),
            text(&case["flow_frequency"])
        );
    }
}

fn hash_outcome(
    result: Result<String, core_domain::dezimal::ParseDezimalError>,
) -> Result<String, String> {
    result.map_err(|_| "ValueError".to_owned())
}

fn expected_text(case: &Value) -> Result<String, String> {
    expected(case)
        .map(|value| text(value).to_owned())
        .map_err(str::to_owned)
}

#[test]
fn loan_hashes() {
    for case in cases("compute_loan_hash").as_array().unwrap() {
        let hash = compute_loan_hash(
            text(&case["entity_id"]),
            text(&case["amount"]),
            text(&case["date"]),
        );
        assert_eq!(hash_outcome(hash), expected_text(case), "{case}");
    }
}

#[test]
fn investment_hashes() {
    for case in cases("compute_investment_hash").as_array().unwrap() {
        let hash = compute_investment_hash(
            text(&case["entity_id"]),
            text(&case["amount"]),
            text(&case["date"]),
            text(&case["name"]),
        );
        assert_eq!(hash_outcome(hash), expected_text(case), "{case}");
    }
}

#[test]
fn loans() {
    for case in cases("loan").as_array().unwrap() {
        let loan = Loan {
            id: None,
            r#type: LoanType::Mortgage,
            currency: "EUR".to_owned(),
            current_installment: Dezimal::from(500),
            interest_rate: Dezimal::from_scaled(3, 2),
            loan_amount: dezimal(&case["loan_amount"]),
            creation: text(&case["creation"]).parse().unwrap(),
            maturity: "2040-01-02".parse().unwrap(),
            principal_outstanding: dezimal(&case["principal_outstanding"]),
            principal_paid: Some(Dezimal::ONE),
            interest_type: InterestType::default(),
            installment_frequency: InstallmentFrequency::default(),
            installment_interests: None,
            fixed_interest_rate: None,
            next_payment_date: None,
            euribor_rate: None,
            fixed_years: None,
            name: None,
            unpaid: None,
            hash: text(&case["hash"]).to_owned(),
            manual_data: None,
            source: DataSource::default(),
        };
        match (loan.post_init(), expected(case)) {
            (Ok(mut loan), Ok(result)) => {
                assert_eq!(
                    loan.principal_paid,
                    Some(dezimal(&result["principal_paid"]))
                );
                assert_eq!(
                    loan.compute_hash(text(&case["entity_id"])),
                    text(&result["hash"])
                );
            }
            (Err(error), Err(python)) => assert_eq!(signal(error), python, "{case}"),
            (outcome, python) => panic!("{case}: {outcome:?} where Python gives {python:?}"),
        }
    }
}

/// The investment figures after `post_init`, or the error, as the fixture writes them.
fn security_outcome(
    result: Result<InvestmentFigures, PositionError>,
) -> Result<InvestmentFigures, NamedError> {
    result.map_err(|error| position_error(&error))
}

fn expected_security(case: &Value) -> Result<InvestmentFigures, NamedError> {
    match python_error(case) {
        Some(error) => Err(error),
        None => Ok((
            optional_dezimal(&case["result"]["initial_investment"]),
            optional_dezimal(&case["result"]["average_buy_price"]),
        )),
    }
}

#[test]
fn stock_details() {
    for case in cases("stock_detail").as_array().unwrap() {
        let stock = StockDetail {
            id: None,
            name: "Stock".to_owned(),
            ticker: "STK".to_owned(),
            isin: "XX0000000000".to_owned(),
            shares: dezimal(&case["shares"]),
            market_value: Dezimal::from(100),
            currency: "EUR".to_owned(),
            r#type: EquityType::Stock,
            initial_investment: optional_dezimal(&case["initial_investment"]),
            average_buy_price: optional_dezimal(&case["average_buy_price"]),
            market: String::new(),
            subtype: None,
            info_sheet_url: None,
            manual_data: None,
            issuer: None,
            source: DataSource::default(),
        };
        let result = stock
            .post_init()
            .map(|stock| (stock.initial_investment, stock.average_buy_price));
        assert_eq!(security_outcome(result), expected_security(case), "{case}");
    }
}

#[test]
fn fund_details() {
    for case in cases("fund_detail").as_array().unwrap() {
        let fund = FundDetail {
            id: None,
            name: "Fund".to_owned(),
            isin: "XX0000000000".to_owned(),
            market: None,
            shares: dezimal(&case["shares"]),
            market_value: Dezimal::from(100),
            currency: "EUR".to_owned(),
            r#type: FundType::MutualFund,
            initial_investment: optional_dezimal(&case["initial_investment"]),
            average_buy_price: optional_dezimal(&case["average_buy_price"]),
            asset_type: None,
            portfolio: None,
            info_sheet_url: None,
            manual_data: None,
            issuer: None,
            source: DataSource::default(),
        };
        let result = fund
            .post_init()
            .map(|fund| (fund.initial_investment, fund.average_buy_price));
        assert_eq!(security_outcome(result), expected_security(case), "{case}");
    }
}

fn today() -> Date {
    text(&cases("today")).parse().unwrap()
}

fn optional_datetime(value: &Value) -> Option<DateTime> {
    optional_text(value).map(datetime)
}

#[test]
fn factoring_details() {
    for case in cases("factoring_detail").as_array().unwrap() {
        let detail = FactoringDetail {
            id: None,
            name: "Invoice".to_owned(),
            amount: Dezimal::from(1000),
            currency: "EUR".to_owned(),
            interest_rate: dezimal(&case["interest_rate"]),
            start: datetime(text(&case["start"])),
            maturity: text(&case["maturity"]).parse().unwrap(),
            r#type: "FACTORING".to_owned(),
            state: "ACTIVE".to_owned(),
            last_invest_date: optional_datetime(&case["last_invest_date"]),
            profitability: optional_dezimal(&case["profitability"]),
            late_interest_rate: optional_dezimal(&case["late_interest_rate"]),
            gross_interest_rate: optional_dezimal(&case["gross_interest_rate"]),
            gross_late_interest_rate: optional_dezimal(&case["gross_late_interest_rate"]),
            manual_data: None,
            source: DataSource::default(),
        };
        let result = detail.post_init(today()).map(|detail| {
            (
                detail.gross_interest_rate,
                detail.gross_late_interest_rate,
                detail.last_invest_date,
                detail.profitability,
            )
        });
        let expected = expected(case).map(|result| {
            (
                optional_dezimal(&result["gross_interest_rate"]),
                optional_dezimal(&result["gross_late_interest_rate"]),
                optional_datetime(&result["last_invest_date"]),
                optional_dezimal(&result["profitability"]),
            )
        });
        assert_eq!(result.map_err(signal), expected, "{case}");
    }
}

#[test]
fn real_estate_cf_details() {
    for case in cases("real_estate_cf_detail").as_array().unwrap() {
        let detail = RealEstateCFDetail {
            id: None,
            name: "Project".to_owned(),
            amount: Dezimal::from(1000),
            pending_amount: Dezimal::ZERO,
            currency: "EUR".to_owned(),
            interest_rate: dezimal(&case["interest_rate"]),
            start: datetime(text(&case["start"])),
            maturity: text(&case["maturity"]).parse().unwrap(),
            r#type: "LOAN".to_owned(),
            state: "ACTIVE".to_owned(),
            business_type: String::new(),
            last_invest_date: optional_datetime(&case["last_invest_date"]),
            profitability: optional_dezimal(&case["profitability"]),
            extended_maturity: optional_date(&case["extended_maturity"]),
            extended_interest_rate: optional_dezimal(&case["extended_interest_rate"]),
            manual_data: None,
            source: DataSource::default(),
        };
        let result = detail
            .post_init(today())
            .map(|detail| (detail.last_invest_date, detail.profitability));
        let expected = expected(case).map(|result| {
            (
                optional_datetime(&result["last_invest_date"]),
                optional_dezimal(&result["profitability"]),
            )
        });
        assert_eq!(result.map_err(signal), expected, "{case}");
    }
}

#[test]
fn deposits() {
    for case in cases("deposit").as_array().unwrap() {
        let deposit = Deposit {
            id: None,
            name: "Deposit".to_owned(),
            amount: dezimal(&case["amount"]),
            currency: "EUR".to_owned(),
            interest_rate: dezimal(&case["interest_rate"]),
            creation: datetime(text(&case["creation"])),
            maturity: text(&case["maturity"]).parse().unwrap(),
            expected_interests: optional_dezimal(&case["expected_interests"]),
            source: DataSource::default(),
        };
        let result = deposit
            .post_init(today())
            .map(|deposit| deposit.expected_interests);
        let expected = expected(case).map(optional_dezimal);
        assert_eq!(result.map_err(signal), expected, "{case}");
    }
}

#[test]
fn crypto_currency_positions() {
    for case in cases("crypto_currency_position").as_array().unwrap() {
        let position = CryptoCurrencyPosition {
            id: None,
            symbol: "BTC".to_owned(),
            amount: dezimal(&case["amount"]),
            r#type: CryptoCurrencyType::Native,
            name: None,
            crypto_asset: None,
            market_value: None,
            currency: None,
            contract_address: None,
            initial_investment: optional_dezimal(&case["initial_investment"]),
            average_buy_price: optional_dezimal(&case["average_buy_price"]),
            investment_currency: optional_text(&case["investment_currency"]).map(str::to_owned),
            chain: None,
            protocol: None,
            position_type: CryptoPositionType::default(),
            icon_url: None,
            source: DataSource::default(),
        };
        let result = position
            .post_init()
            .map(|position| (position.initial_investment, position.average_buy_price));
        assert_eq!(security_outcome(result), expected_security(case), "{case}");
    }
}

#[test]
fn credit_availability() {
    for case in cases("credit_available_amount").as_array().unwrap() {
        let credit = CreditDetail {
            id: None,
            currency: "EUR".to_owned(),
            credit_limit: dezimal(&case["credit_limit"]),
            drawn_amount: dezimal(&case["drawn_amount"]),
            interest_rate: Dezimal::from_scaled(5, 2),
            name: None,
            pledged_amount: None,
            creation: None,
            source: DataSource::default(),
        };
        assert_eq!(
            credit.available_amount().map_err(signal),
            expected(case).map(dezimal),
            "{case}"
        );
    }
}
