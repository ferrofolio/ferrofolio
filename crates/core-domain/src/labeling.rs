// SPDX-License-Identifier: AGPL-3.0-only

//! Labels of transactions and the rules that assign them, the part of
//! `finanze/domain/labeling.py` the ported routes read.

use crate::Dezimal;
use crate::dates::Date;
use crate::pyrepr::repr_str;
use crate::transactions::TxType;

/// The keys of the labels that cannot be changed or deleted
/// (`PROTECTED_LABEL_KEYS`).
pub const PROTECTED_LABEL_KEYS: [&str; 1] = ["internal_transfer"];

/// `LabelCategory`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LabelCategory {
    Income,
    Expense,
    Excluded,
}

impl LabelCategory {
    pub fn value(self) -> &'static str {
        match self {
            LabelCategory::Income => "INCOME",
            LabelCategory::Expense => "EXPENSE",
            LabelCategory::Excluded => "EXCLUDED",
        }
    }

    /// `LabelCategory(value)`, with Python's message for any other value.
    pub fn parse(value: &str) -> Result<Self, String> {
        match value {
            "INCOME" => Ok(LabelCategory::Income),
            "EXPENSE" => Ok(LabelCategory::Expense),
            "EXCLUDED" => Ok(LabelCategory::Excluded),
            other => Err(format!("{} is not a valid LabelCategory", repr_str(other))),
        }
    }
}

/// `LabelSubcategory`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LabelSubcategory {
    Need,
    Wants,
    Savings,
    None,
}

impl LabelSubcategory {
    pub fn value(self) -> &'static str {
        match self {
            LabelSubcategory::Need => "NEED",
            LabelSubcategory::Wants => "WANTS",
            LabelSubcategory::Savings => "SAVINGS",
            LabelSubcategory::None => "NONE",
        }
    }

    /// `LabelSubcategory(value)`, with Python's message for any other value.
    pub fn parse(value: &str) -> Result<Self, String> {
        match value {
            "NEED" => Ok(LabelSubcategory::Need),
            "WANTS" => Ok(LabelSubcategory::Wants),
            "SAVINGS" => Ok(LabelSubcategory::Savings),
            "NONE" => Ok(LabelSubcategory::None),
            other => Err(format!(
                "{} is not a valid LabelSubcategory",
                repr_str(other)
            )),
        }
    }
}

/// `Label`, as the repository reads it: every field but `usage` comes from the
/// `labels` table.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Label {
    /// `str(UUID(...))` of the stored id.
    pub id: String,
    pub key: Option<String>,
    pub name: Option<String>,
    pub description: Option<String>,
    pub color: Option<String>,
    pub icon: Option<String>,
    pub category: LabelCategory,
    pub subcategory: LabelSubcategory,
    /// How many transactions carry the label.
    pub usage: Option<u64>,
}

impl Label {
    /// `protected`, set in `__post_init__`: the key is one of
    /// `PROTECTED_LABEL_KEYS`.
    pub fn protected(&self) -> bool {
        self.key
            .as_deref()
            .is_some_and(|key| PROTECTED_LABEL_KEYS.contains(&key))
    }
}

/// `LabelingRuleKind`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LabelingRuleKind {
    Match,
    Transfer,
}

impl LabelingRuleKind {
    pub fn value(self) -> &'static str {
        match self {
            LabelingRuleKind::Match => "MATCH",
            LabelingRuleKind::Transfer => "TRANSFER",
        }
    }

    /// `LabelingRuleKind(value)`: the member with exactly this value.
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "MATCH" => Some(LabelingRuleKind::Match),
            "TRANSFER" => Some(LabelingRuleKind::Transfer),
            _ => None,
        }
    }
}

/// `TextMatchOperator`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TextMatchOperator {
    Contains,
    StartsWith,
    Equals,
    Regex,
}

impl TextMatchOperator {
    pub fn value(self) -> &'static str {
        match self {
            TextMatchOperator::Contains => "CONTAINS",
            TextMatchOperator::StartsWith => "STARTS_WITH",
            TextMatchOperator::Equals => "EQUALS",
            TextMatchOperator::Regex => "REGEX",
        }
    }

    /// `TextMatchOperator(value)`: the member with exactly this value.
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "CONTAINS" => Some(TextMatchOperator::Contains),
            "STARTS_WITH" => Some(TextMatchOperator::StartsWith),
            "EQUALS" => Some(TextMatchOperator::Equals),
            "REGEX" => Some(TextMatchOperator::Regex),
            _ => None,
        }
    }
}

/// `TextMatchField`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TextMatchField {
    Any,
    Name,
    Counterparty,
}

impl TextMatchField {
    pub fn value(self) -> &'static str {
        match self {
            TextMatchField::Any => "ANY",
            TextMatchField::Name => "NAME",
            TextMatchField::Counterparty => "COUNTERPARTY",
        }
    }

    /// `TextMatchField(value)`: the member with exactly this value.
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "ANY" => Some(TextMatchField::Any),
            "NAME" => Some(TextMatchField::Name),
            "COUNTERPARTY" => Some(TextMatchField::Counterparty),
            _ => None,
        }
    }
}

/// `TextCondition`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TextCondition {
    pub values: Vec<String>,
    pub operator: TextMatchOperator,
    pub field: TextMatchField,
}

/// `LabelingRuleConditions`: one group of conditions, all of which a transaction
/// must meet.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LabelingRuleConditions {
    pub types: Option<Vec<TxType>>,
    pub min_amount: Option<Dezimal>,
    pub max_amount: Option<Dezimal>,
    pub currency: Option<String>,
    pub from_date: Option<Date>,
    pub to_date: Option<Date>,
    pub day_from: Option<i64>,
    pub day_to: Option<i64>,
    /// `str(UUID(...))` of each entity.
    pub entities: Option<Vec<String>>,
    pub text: Option<TextCondition>,
    pub max_days: Option<i64>,
    pub ibans: Option<Vec<String>>,
}

/// `LabelingRule`, as the repository reads it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LabelingRule {
    /// `str(UUID(...))` of the stored id.
    pub id: String,
    pub conditions: Vec<LabelingRuleConditions>,
    /// `str(UUID(...))` of each label, in the order the query returns them.
    pub label_ids: Vec<String>,
    pub name: Option<String>,
    pub enabled: bool,
    pub kind: LabelingRuleKind,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_the_enums_like_python() {
        assert_eq!(LabelCategory::parse("INCOME"), Ok(LabelCategory::Income));
        assert_eq!(
            LabelCategory::parse("income"),
            Err("'income' is not a valid LabelCategory".to_owned())
        );
        assert_eq!(
            LabelSubcategory::parse("it's"),
            Err("\"it's\" is not a valid LabelSubcategory".to_owned())
        );
        for subcategory in [
            LabelSubcategory::Need,
            LabelSubcategory::Wants,
            LabelSubcategory::Savings,
            LabelSubcategory::None,
        ] {
            assert_eq!(
                LabelSubcategory::parse(subcategory.value()),
                Ok(subcategory)
            );
        }
    }

    #[test]
    fn reads_the_rule_enums_exactly() {
        for kind in [LabelingRuleKind::Match, LabelingRuleKind::Transfer] {
            assert_eq!(LabelingRuleKind::parse(kind.value()), Some(kind));
        }
        for operator in [
            TextMatchOperator::Contains,
            TextMatchOperator::StartsWith,
            TextMatchOperator::Equals,
            TextMatchOperator::Regex,
        ] {
            assert_eq!(TextMatchOperator::parse(operator.value()), Some(operator));
        }
        for field in [
            TextMatchField::Any,
            TextMatchField::Name,
            TextMatchField::Counterparty,
        ] {
            assert_eq!(TextMatchField::parse(field.value()), Some(field));
        }
        assert_eq!(LabelingRuleKind::parse("match"), None);
        assert_eq!(TextMatchOperator::parse("CONTAINS "), None);
        assert_eq!(TextMatchField::parse(""), None);
    }

    #[test]
    fn protects_the_internal_transfer_label() {
        let mut label = Label {
            id: "0d3e7c1a-5b2f-4c8e-9a1b-2c3d4e5f6a7b".to_owned(),
            key: Some("internal_transfer".to_owned()),
            name: None,
            description: None,
            color: None,
            icon: None,
            category: LabelCategory::Excluded,
            subcategory: LabelSubcategory::None,
            usage: None,
        };
        assert!(label.protected());
        label.key = Some("groceries".to_owned());
        assert!(!label.protected());
        label.key = None;
        assert!(!label.protected());
    }
}
