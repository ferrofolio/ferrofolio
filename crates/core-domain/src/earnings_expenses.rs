// SPDX-License-Identifier: AGPL-3.0-only

//! Earnings and expenses, the part of `finanze/domain/earnings_expenses.py` the
//! ported routes read.

use crate::Dezimal;
use crate::dates::Date;
use crate::datetime::DateTime;
use crate::str_enum::str_enum;

str_enum! {
    /// `FlowFrequency`: how often a periodic flow recurs.
    pub enum FlowFrequency {
        Daily = "DAILY",
        Weekly = "WEEKLY",
        Biweekly = "BIWEEKLY",
        Semimonthly = "SEMIMONTHLY",
        Monthly = "MONTHLY",
        EveryTwoMonths = "EVERY_TWO_MONTHS",
        Quarterly = "QUARTERLY",
        EveryFourMonths = "EVERY_FOUR_MONTHS",
        Semiannually = "SEMIANNUALLY",
        Yearly = "YEARLY",
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

/// `FlowStatus`: whether a pending flow is still expected.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FlowStatus {
    Active,
    Disabled,
    Completed,
}

impl FlowStatus {
    /// Every member, in the order Python declares them.
    pub const ALL: [FlowStatus; 3] = [
        FlowStatus::Active,
        FlowStatus::Disabled,
        FlowStatus::Completed,
    ];

    /// The member's value.
    pub fn value(self) -> &'static str {
        match self {
            FlowStatus::Active => "ACTIVE",
            FlowStatus::Disabled => "DISABLED",
            FlowStatus::Completed => "COMPLETED",
        }
    }

    /// `FlowStatus(value)`: the member with exactly this value.
    pub fn parse(value: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|status| status.value() == value)
    }
}

/// `PendingFlow`: a single expected earning or expense. Ids are in the form
/// `str(UUID(...))` gives.
#[derive(Debug, Clone, PartialEq)]
pub struct PendingFlow {
    pub id: String,
    pub name: String,
    pub amount: Dezimal,
    pub currency: String,
    pub flow_type: FlowType,
    pub category: Option<String>,
    pub status: FlowStatus,
    pub date: Option<Date>,
    pub icon: Option<String>,
    pub status_changed_at: Option<DateTime>,
}

/// `FlowSortField`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum FlowSortField {
    #[default]
    Amount,
    Date,
}

impl FlowSortField {
    /// `FlowSortField(value)`: the member with exactly this value.
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "amount" => Some(FlowSortField::Amount),
            "date" => Some(FlowSortField::Date),
            _ => None,
        }
    }
}

/// `SortOrder`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SortOrder {
    Asc,
    #[default]
    Desc,
}

impl SortOrder {
    /// `SortOrder(value)`: the member with exactly this value.
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "asc" => Some(SortOrder::Asc),
            "desc" => Some(SortOrder::Desc),
            _ => None,
        }
    }
}

/// `PendingFlowQuery`. `statuses` and `categories` are `None` rather than empty,
/// as the route passes them.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct PendingFlowQuery {
    pub statuses: Option<Vec<FlowStatus>>,
    pub flow_type: Option<FlowType>,
    pub categories: Option<Vec<String>>,
    pub sort_by: FlowSortField,
    pub order: SortOrder,
    pub page: Option<i64>,
    pub limit: Option<i64>,
    pub include_stats: bool,
    pub include_categories: bool,
}

/// `CategoryStat`: the amounts of one category, or of one flow by its name.
/// The amounts keep the order in which their currencies first appeared.
#[derive(Debug, Clone, PartialEq)]
pub struct CategoryStat {
    pub category: Option<String>,
    pub amounts: Vec<(String, Dezimal)>,
    pub name: Option<String>,
}

/// `FlowTypeStat`: the totals by currency, in the order the currencies first
/// appeared, the amounts by category and the number of flows.
#[derive(Debug, Clone, PartialEq)]
pub struct FlowTypeStat {
    pub totals: Vec<(String, Dezimal)>,
    pub by_category: Vec<CategoryStat>,
    pub count: usize,
}

/// `PendingFlowStats`.
#[derive(Debug, Clone, PartialEq)]
pub struct PendingFlowStats {
    pub earning: FlowTypeStat,
    pub expense: FlowTypeStat,
}

/// `PendingFlowPage`.
#[derive(Debug, Clone, PartialEq)]
pub struct PendingFlowPage {
    pub entries: Vec<PendingFlow>,
    pub total: usize,
    pub page: Option<i64>,
    pub limit: Option<i64>,
    pub has_more: bool,
    pub stats: Option<PendingFlowStats>,
    pub categories: Option<Vec<String>>,
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
        for status in FlowStatus::ALL {
            assert_eq!(FlowStatus::parse(status.value()), Some(status));
        }
        assert_eq!(FlowStatus::parse("active"), None);
        assert_eq!(FlowSortField::parse("date"), Some(FlowSortField::Date));
        assert_eq!(FlowSortField::parse("DATE"), None);
        assert_eq!(SortOrder::parse("asc"), Some(SortOrder::Asc));
        assert_eq!(SortOrder::parse(" asc"), None);
    }
}
