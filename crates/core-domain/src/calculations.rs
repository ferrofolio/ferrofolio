// SPDX-License-Identifier: AGPL-3.0-only

//! Savings calculation types from `finanze/domain/calculations.py`.

use crate::dezimal::Dezimal;

/// How often contributions are made and returns are applied.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SavingsPeriodicity {
    Monthly,
    Quarterly,
    Yearly,
}

impl SavingsPeriodicity {
    pub const fn as_str(self) -> &'static str {
        match self {
            SavingsPeriodicity::Monthly => "MONTHLY",
            SavingsPeriodicity::Quarterly => "QUARTERLY",
            SavingsPeriodicity::Yearly => "YEARLY",
        }
    }

    /// `SavingsPeriodicity(value)` in Python: an exact match of the value.
    pub fn from_value(value: &str) -> Option<Self> {
        match value {
            "MONTHLY" => Some(SavingsPeriodicity::Monthly),
            "QUARTERLY" => Some(SavingsPeriodicity::Quarterly),
            "YEARLY" => Some(SavingsPeriodicity::Yearly),
            _ => None,
        }
    }

    pub const fn periods_per_year(self) -> i64 {
        match self {
            SavingsPeriodicity::Monthly => 12,
            SavingsPeriodicity::Quarterly => 4,
            SavingsPeriodicity::Yearly => 1,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct SavingsScenarioRequest {
    pub scenario_id: String,
    pub annual_market_performance: Dezimal,
    pub periodic_contribution: Option<Dezimal>,
    pub target_amount: Option<Dezimal>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SavingsRetirementRequest {
    pub withdrawal_amount: Option<Dezimal>,
    pub withdrawal_years: Option<i64>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SavingsCalculationRequest {
    pub base_amount: Option<Dezimal>,
    pub years: Option<i64>,
    pub periodicity: SavingsPeriodicity,
    pub scenarios: Vec<SavingsScenarioRequest>,
    pub retirement: Option<SavingsRetirementRequest>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SavingsPeriodEntry {
    pub period_index: i64,
    pub contributed: Dezimal,
    pub total_contributed: Dezimal,
    pub revaluation: Dezimal,
    pub total_revaluation: Dezimal,
    pub total_invested: Dezimal,
    pub balance: Dezimal,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SavingsRetirementPeriodEntry {
    pub period_index: i64,
    pub withdrawal: Dezimal,
    pub total_withdrawn: Dezimal,
    pub revaluation: Dezimal,
    pub balance: Dezimal,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SavingsRetirementResult {
    pub withdrawal_amount: Dezimal,
    pub duration_periods: i64,
    pub duration_years: Dezimal,
    pub total_withdrawn: Dezimal,
    pub periods: Vec<SavingsRetirementPeriodEntry>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SavingsScenarioResult {
    pub scenario_id: String,
    pub annual_market_performance: Dezimal,
    pub periodic_contribution: Dezimal,
    pub accumulation_periods: Vec<SavingsPeriodEntry>,
    pub total_contributions: Dezimal,
    pub total_revaluation: Dezimal,
    pub final_balance: Dezimal,
    pub retirement: Option<SavingsRetirementResult>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SavingsCalculationResult {
    pub scenarios: Vec<SavingsScenarioResult>,
}
