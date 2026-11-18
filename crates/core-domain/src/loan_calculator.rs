// SPDX-License-Identifier: AGPL-3.0-only

//! Loan calculation types from `finanze/domain/loan_calculator.py`.

use crate::dates::Date;
use crate::dezimal::Dezimal;
use crate::global_position::{InstallmentFrequency, InterestType};

/// Input of a loan calculation. Rates are annual fractions (0.03 is 3 %).
///
/// `start` and `end` are optional only because the API accepts `null` for them; the
/// calculator reports them as missing fields, as the Python implementation does.
#[derive(Debug, Clone, PartialEq)]
pub struct LoanCalculationParams {
    pub loan_amount: Option<Dezimal>,
    pub interest_rate: Dezimal,
    pub interest_type: InterestType,
    pub euribor_rate: Option<Dezimal>,
    pub fixed_years: Option<i64>,
    pub start: Option<Date>,
    pub end: Option<Date>,
    pub principal_outstanding: Option<Dezimal>,
    pub fixed_interest_rate: Option<Dezimal>,
    pub installment_frequency: InstallmentFrequency,
    pub tracking_ref_outstanding: Option<Dezimal>,
    pub tracking_ref_date: Option<Date>,
}

/// Result of a loan calculation for the next installment.
#[derive(Debug, Clone, PartialEq)]
pub struct LoanCalculationResult {
    pub current_installment_payment: Option<Dezimal>,
    pub current_installment_interests: Option<Dezimal>,
    pub principal_outstanding: Option<Dezimal>,
    pub installment_date: Option<Date>,
}
