// SPDX-License-Identifier: AGPL-3.0-only

//! Constants from `finanze/domain/constants.py`.

use crate::dezimal::Dezimal;

/// `CAPITAL_GAINS_BASE_TAX`: 19 %, `Dezimal(0.19)`, which Python reads from the
/// float's shortest text and so is exactly 0.19.
pub const CAPITAL_GAINS_BASE_TAX: Dezimal = Dezimal::from_scaled(19, 2);

/// `SUPPORTED_CURRENCIES`.
pub const SUPPORTED_CURRENCIES: [&str; 2] = ["EUR", "USD"];

/// `PUSD_SYMBOL`.
pub const PUSD_SYMBOL: &str = "PUSD";

/// `PUSD_CONTRACT_ADDRESS`.
pub const PUSD_CONTRACT_ADDRESS: &str = "0xc011a7e12a19f7b1f670d46f03b03f3342e82dfb";
