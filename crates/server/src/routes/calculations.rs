// SPDX-License-Identifier: AGPL-3.0-only

//! The calculator routes, ported from
//! `finanze/infrastructure/controller/routes/calculate_loan.py` and
//! `calculate_savings.py`.

use axum::http::header::CONTENT_TYPE;
use axum::response::{IntoResponse, Response};
use core_domain::Dezimal;
use core_domain::calculations::{
    SavingsCalculationRequest, SavingsCalculationResult, SavingsPeriodEntry, SavingsPeriodicity,
    SavingsRetirementPeriodEntry, SavingsRetirementRequest, SavingsRetirementResult,
    SavingsScenarioRequest, SavingsScenarioResult,
};
use core_domain::dates::Date;
use core_domain::dezimal::ArithmeticError;
use core_domain::exception::{CalculationError, MissingFieldsError};
use core_domain::global_position::{InstallmentFrequency, InterestType};
use core_domain::loan_calculator::{LoanCalculationParams, LoanCalculationResult};
use serde::Serialize;
use serde_json::{Map, Value};

use crate::app::AppState;
use crate::error::ApiError;
use crate::pycompat::{self, dezimal, int, optional, required, truthy};

pub const LOAN_PATH: &str = "/api/v1/calculation/loan";
pub const SAVINGS_PATH: &str = "/api/v1/calculations/savings";

/// `POST /api/v1/calculation/loan`.
pub fn calculate_loan(state: &AppState, body: &Map<String, Value>) -> Response {
    let params = match parse_loan(body) {
        Ok(params) => params,
        Err(message) => return ApiError::InvalidRequest(message).into_response(),
    };
    match state.calculate_loan.execute(&params) {
        Ok(result) => json_response(&LoanResultDto::from(&result)),
        // The Python route reports missing fields without an error code.
        Err(CalculationError::MissingFields(error)) => {
            ApiError::BadRequestMessage(error.to_string()).into_response()
        }
        Err(error) => unexpected(&error),
    }
}

/// `POST /api/v1/calculations/savings`.
pub fn calculate_savings(state: &AppState, body: &Map<String, Value>) -> Response {
    let request = match parse_savings(body) {
        Ok(request) => request,
        Err(message) => return ApiError::InvalidRequest(message).into_response(),
    };
    match state.calculate_savings.execute(&request) {
        Ok(result) => json_response(&SavingsResultDto::from(&result)),
        Err(CalculationError::MissingFields(error)) => {
            ApiError::InvalidRequest(error.to_string()).into_response()
        }
        Err(CalculationError::Input(error)) => {
            ApiError::InvalidRequest(error.to_string()).into_response()
        }
        Err(error) => unexpected(&error),
    }
}

/// A 500 response with the message the Python exception would carry.
fn unexpected(error: &CalculationError) -> Response {
    let message = match error {
        CalculationError::Arithmetic(ArithmeticError::DivisionByZero) => {
            "[<class 'decimal.DivisionByZero'>]".to_owned()
        }
        CalculationError::Arithmetic(ArithmeticError::InvalidOperation) => {
            "[<class 'decimal.InvalidOperation'>]".to_owned()
        }
        CalculationError::Arithmetic(ArithmeticError::Overflow) => {
            "[<class 'decimal.Overflow'>]".to_owned()
        }
        CalculationError::DateOutOfRange => "date value out of range".to_owned(),
        CalculationError::MissingFields(error) => error.to_string(),
        CalculationError::Input(error) => error.to_string(),
    };
    tracing::error!(error = %message, "calculation failed");
    ApiError::Unexpected(message).into_response()
}

fn parse_loan(body: &Map<String, Value>) -> Result<LoanCalculationParams, String> {
    let loan_amount = optional(body, "loan_amount").map(dezimal).transpose()?;
    let principal_outstanding = optional(body, "principal_outstanding")
        .map(dezimal)
        .transpose()?;
    let interest_rate = dezimal(required(body, "interest_rate")?)?;
    let raw_interest_type = required(body, "interest_type")?;
    let interest_type = raw_interest_type
        .as_str()
        .and_then(InterestType::from_value)
        .ok_or_else(|| pycompat::invalid_enum(raw_interest_type, InterestType::NAME))?;
    let euribor_rate = optional(body, "euribor_rate").map(dezimal).transpose()?;
    let fixed_years = optional(body, "fixed_years").map(int).transpose()?;
    let fixed_interest_rate = optional(body, "fixed_interest_rate")
        .map(dezimal)
        .transpose()?;
    let installment_frequency = match body.get("installment_frequency") {
        Some(raw) if truthy(raw) => raw
            .as_str()
            .and_then(InstallmentFrequency::from_value)
            .ok_or_else(|| pycompat::invalid_enum(raw, InstallmentFrequency::NAME))?,
        _ => InstallmentFrequency::Monthly,
    };
    let raw_start = required(body, "start")?;
    let raw_end = required(body, "end")?;
    let start = loan_date(raw_start)?;
    let end = loan_date(raw_end)?;

    Ok(LoanCalculationParams {
        loan_amount,
        interest_rate,
        interest_type,
        euribor_rate,
        fixed_years,
        start,
        end,
        principal_outstanding,
        fixed_interest_rate,
        installment_frequency,
        tracking_ref_outstanding: None,
        tracking_ref_date: None,
    })
}

/// A schedule date. `null` is reported later as a missing field, like in Python.
fn loan_date(value: &Value) -> Result<Option<Date>, String> {
    match value {
        Value::Null => Ok(None),
        Value::String(text) => pycompat::iso_date(text).map(Some),
        // Python passes other types on and fails later with an internal error.
        other => Err(format!(
            "Invalid isoformat string: {}",
            pycompat::repr(other)
        )),
    }
}

fn parse_savings(body: &Map<String, Value>) -> Result<SavingsCalculationRequest, String> {
    let base_amount = optional(body, "base_amount").map(dezimal).transpose()?;
    let years = optional(body, "years").map(int).transpose()?;
    let periodicity = required(body, "periodicity")?
        .as_str()
        .and_then(SavingsPeriodicity::from_value)
        .ok_or_else(|| MissingFieldsError::new(["periodicity"]).to_string())?;
    let scenarios = match required(body, "scenarios")? {
        Value::Array(items) => items
            .iter()
            .map(parse_scenario)
            .collect::<Result<Vec<_>, _>>()?,
        Value::Object(fields) if fields.is_empty() => Vec::new(),
        Value::String(text) if text.is_empty() => Vec::new(),
        // Iterating a string or a dict yields strings, which cannot be indexed by key.
        Value::Object(_) | Value::String(_) => {
            return Err("string indices must be integers, not 'str'".to_owned());
        }
        other => {
            return Err(format!(
                "'{}' object is not iterable",
                pycompat::type_name(other)
            ));
        }
    };
    let retirement = match body.get("retirement") {
        None | Some(Value::Null) => None,
        Some(Value::Object(payload)) => Some(parse_retirement(payload)?),
        Some(other) => {
            return Err(format!(
                "'{}' object has no attribute 'get'",
                pycompat::type_name(other)
            ));
        }
    };
    Ok(SavingsCalculationRequest {
        base_amount,
        years,
        periodicity,
        scenarios,
        retirement,
    })
}

fn parse_scenario(raw: &Value) -> Result<SavingsScenarioRequest, String> {
    let fields = match raw {
        Value::Object(fields) => fields,
        Value::String(_) => return Err("string indices must be integers, not 'str'".to_owned()),
        Value::Array(_) => {
            return Err("list indices must be integers or slices, not str".to_owned());
        }
        other => {
            return Err(format!(
                "'{}' object is not subscriptable",
                pycompat::type_name(other)
            ));
        }
    };
    let raw_id = required(fields, "id")?;
    let annual_market_performance = dezimal(required(fields, "annual_market_performance")?)?;
    let periodic_contribution = optional(fields, "periodic_contribution")
        .map(dezimal)
        .transpose()?;
    // `raw.get("target_amount")` is tested for truthiness, so 0 means no target.
    // Python tests the JSON number after reading it as a float, so `truthy` does the
    // same: a value that underflows to 0.0, such as 1e-400, also means no target. This
    // only decides whether a target was given; the amount itself is read exactly
    // (docs/rust-backend.md, amounts in requests).
    let target_amount = match fields.get("target_amount") {
        Some(raw) if truthy(raw) => Some(dezimal(raw)?),
        _ => None,
    };
    let scenario_id = raw_id.as_str().ok_or_else(|| {
        validation_error(
            "SavingsScenarioRequest",
            "scenario_id",
            "Input should be a valid string",
        )
    })?;
    Ok(SavingsScenarioRequest {
        scenario_id: scenario_id.to_owned(),
        annual_market_performance,
        periodic_contribution,
        target_amount,
    })
}

fn parse_retirement(payload: &Map<String, Value>) -> Result<SavingsRetirementRequest, String> {
    let withdrawal_amount = optional(payload, "withdrawal_amount")
        .map(dezimal)
        .transpose()?;
    let withdrawal_years = match optional(payload, "withdrawal_years") {
        None => None,
        Some(raw) => Some(pycompat::lax_int(raw).ok_or_else(|| {
            validation_error(
                "SavingsRetirementRequest",
                "withdrawal_years",
                pycompat::lax_int_error(raw),
            )
        })?),
    };
    Ok(SavingsRetirementRequest {
        withdrawal_amount,
        withdrawal_years,
    })
}

/// The leading lines of a pydantic `ValidationError` message.
fn validation_error(model: &str, field: &str, problem: &str) -> String {
    format!("1 validation error for {model}\n{field}\n  {problem}")
}

/// An amount in a response. The API carries amounts as JSON numbers, which is the
/// only place where they become binary floats.
fn amount(value: Dezimal) -> Amount {
    Amount(value.to_f64_for_api())
}

/// A number in a response. A value beyond the range of a float has no JSON
/// number; serde_json would write `null` and Python writes `Infinity`, which is not
/// JSON, so serialising it fails instead (docs/rust-backend.md, known deviations).
#[derive(Debug, Clone, Copy)]
struct Amount(f64);

impl Serialize for Amount {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        if !self.0.is_finite() {
            return Err(serde::ser::Error::custom(OUT_OF_RANGE));
        }
        serializer.serialize_f64(self.0)
    }
}

const OUT_OF_RANGE: &str = "Result beyond the range of a JSON number";

/// A JSON response, or a `500` when a number in it has no JSON form.
fn json_response<T: Serialize>(value: &T) -> Response {
    match serde_json::to_vec(value) {
        Ok(body) => ([(CONTENT_TYPE, "application/json")], body).into_response(),
        Err(_) => {
            tracing::error!(error = OUT_OF_RANGE, "calculation failed");
            ApiError::Unexpected(OUT_OF_RANGE.to_owned()).into_response()
        }
    }
}

#[derive(Debug, Serialize)]
struct LoanResultDto {
    current_installment_payment: Option<Amount>,
    current_installment_interests: Option<Amount>,
    principal_outstanding: Option<Amount>,
    installment_date: Option<String>,
}

impl From<&LoanCalculationResult> for LoanResultDto {
    fn from(result: &LoanCalculationResult) -> Self {
        LoanResultDto {
            current_installment_payment: result.current_installment_payment.map(amount),
            current_installment_interests: result.current_installment_interests.map(amount),
            principal_outstanding: result.principal_outstanding.map(amount),
            installment_date: result.installment_date.map(|date| date.to_string()),
        }
    }
}

#[derive(Debug, Serialize)]
struct SavingsResultDto {
    scenarios: Vec<SavingsScenarioDto>,
}

#[derive(Debug, Serialize)]
struct SavingsScenarioDto {
    scenario_id: String,
    annual_market_performance: Amount,
    periodic_contribution: Amount,
    accumulation_periods: Vec<SavingsPeriodDto>,
    total_contributions: Amount,
    total_revaluation: Amount,
    final_balance: Amount,
    retirement: Option<SavingsRetirementDto>,
}

#[derive(Debug, Serialize)]
struct SavingsPeriodDto {
    period_index: i64,
    contributed: Amount,
    total_contributed: Amount,
    revaluation: Amount,
    total_revaluation: Amount,
    total_invested: Amount,
    balance: Amount,
}

#[derive(Debug, Serialize)]
struct SavingsRetirementDto {
    withdrawal_amount: Amount,
    duration_periods: i64,
    duration_years: Amount,
    total_withdrawn: Amount,
    periods: Vec<SavingsRetirementPeriodDto>,
}

#[derive(Debug, Serialize)]
struct SavingsRetirementPeriodDto {
    period_index: i64,
    withdrawal: Amount,
    total_withdrawn: Amount,
    revaluation: Amount,
    balance: Amount,
}

impl From<&SavingsCalculationResult> for SavingsResultDto {
    fn from(result: &SavingsCalculationResult) -> Self {
        SavingsResultDto {
            scenarios: result
                .scenarios
                .iter()
                .map(SavingsScenarioDto::from)
                .collect(),
        }
    }
}

impl From<&SavingsScenarioResult> for SavingsScenarioDto {
    fn from(scenario: &SavingsScenarioResult) -> Self {
        SavingsScenarioDto {
            scenario_id: scenario.scenario_id.clone(),
            annual_market_performance: amount(scenario.annual_market_performance),
            periodic_contribution: amount(scenario.periodic_contribution),
            accumulation_periods: scenario
                .accumulation_periods
                .iter()
                .map(SavingsPeriodDto::from)
                .collect(),
            total_contributions: amount(scenario.total_contributions),
            total_revaluation: amount(scenario.total_revaluation),
            final_balance: amount(scenario.final_balance),
            retirement: scenario.retirement.as_ref().map(SavingsRetirementDto::from),
        }
    }
}

impl From<&SavingsPeriodEntry> for SavingsPeriodDto {
    fn from(entry: &SavingsPeriodEntry) -> Self {
        SavingsPeriodDto {
            period_index: entry.period_index,
            contributed: amount(entry.contributed),
            total_contributed: amount(entry.total_contributed),
            revaluation: amount(entry.revaluation),
            total_revaluation: amount(entry.total_revaluation),
            total_invested: amount(entry.total_invested),
            balance: amount(entry.balance),
        }
    }
}

impl From<&SavingsRetirementResult> for SavingsRetirementDto {
    fn from(retirement: &SavingsRetirementResult) -> Self {
        SavingsRetirementDto {
            withdrawal_amount: amount(retirement.withdrawal_amount),
            duration_periods: retirement.duration_periods,
            duration_years: amount(retirement.duration_years),
            total_withdrawn: amount(retirement.total_withdrawn),
            periods: retirement
                .periods
                .iter()
                .map(SavingsRetirementPeriodDto::from)
                .collect(),
        }
    }
}

impl From<&SavingsRetirementPeriodEntry> for SavingsRetirementPeriodDto {
    fn from(entry: &SavingsRetirementPeriodEntry) -> Self {
        SavingsRetirementPeriodDto {
            period_index: entry.period_index,
            withdrawal: amount(entry.withdrawal),
            total_withdrawn: amount(entry.total_withdrawn),
            revaluation: amount(entry.revaluation),
            balance: amount(entry.balance),
        }
    }
}
