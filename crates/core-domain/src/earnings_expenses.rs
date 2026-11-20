// SPDX-License-Identifier: AGPL-3.0-only

//! Earnings and expenses, the part of `finanze/domain/earnings_expenses.py` the
//! ported routes read.

use crate::Dezimal;
use crate::dates::Date;

/// `FlowFrequency`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FlowFrequency {
    Daily,
    Weekly,
    Biweekly,
    Semimonthly,
    Monthly,
    EveryTwoMonths,
    Quarterly,
    EveryFourMonths,
    Semiannually,
    Yearly,
}

impl FlowFrequency {
    /// Every member, in the order Python declares them.
    pub const ALL: [FlowFrequency; 10] = [
        FlowFrequency::Daily,
        FlowFrequency::Weekly,
        FlowFrequency::Biweekly,
        FlowFrequency::Semimonthly,
        FlowFrequency::Monthly,
        FlowFrequency::EveryTwoMonths,
        FlowFrequency::Quarterly,
        FlowFrequency::EveryFourMonths,
        FlowFrequency::Semiannually,
        FlowFrequency::Yearly,
    ];

    /// The member's value.
    pub fn value(self) -> &'static str {
        match self {
            FlowFrequency::Daily => "DAILY",
            FlowFrequency::Weekly => "WEEKLY",
            FlowFrequency::Biweekly => "BIWEEKLY",
            FlowFrequency::Semimonthly => "SEMIMONTHLY",
            FlowFrequency::Monthly => "MONTHLY",
            FlowFrequency::EveryTwoMonths => "EVERY_TWO_MONTHS",
            FlowFrequency::Quarterly => "QUARTERLY",
            FlowFrequency::EveryFourMonths => "EVERY_FOUR_MONTHS",
            FlowFrequency::Semiannually => "SEMIANNUALLY",
            FlowFrequency::Yearly => "YEARLY",
        }
    }

    /// `FlowFrequency(value)`: the member with exactly this value.
    pub fn parse(value: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|frequency| frequency.value() == value)
    }
}

/// `FlowType`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FlowType {
    Earning,
    Expense,
}

impl FlowType {
    /// The member's value.
    pub fn value(self) -> &'static str {
        match self {
            FlowType::Earning => "EARNING",
            FlowType::Expense => "EXPENSE",
        }
    }

    /// `FlowType(value)`: the member with exactly this value.
    pub fn parse(value: &str) -> Option<Self> {
        [FlowType::Earning, FlowType::Expense]
            .into_iter()
            .find(|flow_type| flow_type.value() == value)
    }
}

/// `RealEstateFlowInfo`: how a periodic flow belongs to a real estate.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RealEstateFlowInfo {
    pub flow_subtype: String,
    pub linked_loan_hash: Option<String>,
}

/// `PeriodicFlow`. Ids are in the form `str(UUID(...))` gives.
#[derive(Debug, Clone, PartialEq)]
pub struct PeriodicFlow {
    pub id: String,
    pub name: String,
    pub amount: Dezimal,
    pub currency: String,
    pub flow_type: FlowType,
    pub frequency: FlowFrequency,
    pub category: Option<String>,
    pub enabled: bool,
    pub since: Date,
    pub until: Option<Date>,
    pub icon: Option<String>,
    pub linked: Option<bool>,
    pub real_estate_flow: Option<RealEstateFlowInfo>,
    pub next_date: Option<Date>,
    pub max_amount: Option<Dezimal>,
    pub label_id: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_exactly_the_values() {
        for frequency in FlowFrequency::ALL {
            assert_eq!(FlowFrequency::parse(frequency.value()), Some(frequency));
        }
        assert_eq!(FlowFrequency::parse("monthly"), None);
        assert_eq!(FlowType::parse("EXPENSE"), Some(FlowType::Expense));
        assert_eq!(FlowType::parse("Earning"), None);
    }
}
