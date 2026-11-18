// SPDX-License-Identifier: AGPL-3.0-only

//! Position types from `finanze/domain/global_position.py`, ported as they are needed.

use std::fmt;

/// `InterestType`: how the interest rate of a loan is defined.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum InterestType {
    Fixed,
    Variable,
    Mixed,
}

impl InterestType {
    pub const NAME: &'static str = "InterestType";

    /// The enum value, which is also its wire representation.
    pub const fn as_str(self) -> &'static str {
        match self {
            InterestType::Fixed => "FIXED",
            InterestType::Variable => "VARIABLE",
            InterestType::Mixed => "MIXED",
        }
    }

    /// `InterestType(value)` in Python: an exact match of the value.
    pub fn from_value(value: &str) -> Option<Self> {
        match value {
            "FIXED" => Some(InterestType::Fixed),
            "VARIABLE" => Some(InterestType::Variable),
            "MIXED" => Some(InterestType::Mixed),
            _ => None,
        }
    }
}

impl fmt::Display for InterestType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// `InstallmentFrequency`: how often a loan installment is due.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum InstallmentFrequency {
    Weekly,
    Biweekly,
    Semimonthly,
    #[default]
    Monthly,
    Bimonthly,
    Quarterly,
    Semiannual,
    Yearly,
}

impl InstallmentFrequency {
    pub const NAME: &'static str = "InstallmentFrequency";

    pub const fn as_str(self) -> &'static str {
        match self {
            InstallmentFrequency::Weekly => "WEEKLY",
            InstallmentFrequency::Biweekly => "BIWEEKLY",
            InstallmentFrequency::Semimonthly => "SEMIMONTHLY",
            InstallmentFrequency::Monthly => "MONTHLY",
            InstallmentFrequency::Bimonthly => "BIMONTHLY",
            InstallmentFrequency::Quarterly => "QUARTERLY",
            InstallmentFrequency::Semiannual => "SEMIANNUAL",
            InstallmentFrequency::Yearly => "YEARLY",
        }
    }

    /// `InstallmentFrequency(value)` in Python: an exact match of the value.
    pub fn from_value(value: &str) -> Option<Self> {
        match value {
            "WEEKLY" => Some(InstallmentFrequency::Weekly),
            "BIWEEKLY" => Some(InstallmentFrequency::Biweekly),
            "SEMIMONTHLY" => Some(InstallmentFrequency::Semimonthly),
            "MONTHLY" => Some(InstallmentFrequency::Monthly),
            "BIMONTHLY" => Some(InstallmentFrequency::Bimonthly),
            "QUARTERLY" => Some(InstallmentFrequency::Quarterly),
            "SEMIANNUAL" => Some(InstallmentFrequency::Semiannual),
            "YEARLY" => Some(InstallmentFrequency::Yearly),
            _ => None,
        }
    }

    pub const fn payments_per_year(self) -> u32 {
        match self {
            InstallmentFrequency::Weekly => 52,
            InstallmentFrequency::Biweekly => 26,
            InstallmentFrequency::Semimonthly => 24,
            InstallmentFrequency::Monthly => 12,
            InstallmentFrequency::Bimonthly => 6,
            InstallmentFrequency::Quarterly => 4,
            InstallmentFrequency::Semiannual => 2,
            InstallmentFrequency::Yearly => 1,
        }
    }
}

impl fmt::Display for InstallmentFrequency {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn values_round_trip() {
        for frequency in [
            InstallmentFrequency::Weekly,
            InstallmentFrequency::Biweekly,
            InstallmentFrequency::Semimonthly,
            InstallmentFrequency::Monthly,
            InstallmentFrequency::Bimonthly,
            InstallmentFrequency::Quarterly,
            InstallmentFrequency::Semiannual,
            InstallmentFrequency::Yearly,
        ] {
            assert_eq!(
                InstallmentFrequency::from_value(frequency.as_str()),
                Some(frequency)
            );
        }
        for interest_type in [
            InterestType::Fixed,
            InterestType::Variable,
            InterestType::Mixed,
        ] {
            assert_eq!(
                InterestType::from_value(interest_type.as_str()),
                Some(interest_type)
            );
        }
        assert_eq!(InterestType::from_value("fixed"), None);
    }
}
