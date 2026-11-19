// SPDX-License-Identifier: AGPL-3.0-only

//! Periodic earnings and expenses from `finanze/domain/earnings_expenses.py`,
//! ported as they are needed.

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
