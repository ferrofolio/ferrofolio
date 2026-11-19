// SPDX-License-Identifier: AGPL-3.0-only

//! Precious metals, from `finanze/domain/commodity.py`.

use crate::dates::Date;
use crate::dezimal::{ArithmeticError, Dezimal};
use crate::str_enum::str_enum;

/// `COMMODITY_HISTORIC_CUTOFF`: 12 June 2026.
pub const COMMODITY_HISTORIC_CUTOFF: Date = jiff::civil::date(2026, 6, 12);

str_enum! {
    /// `CommodityType`: a precious metal.
    pub enum CommodityType {
        Gold = "GOLD",
        Silver = "SILVER",
        Platinum = "PLATINUM",
        Palladium = "PALLADIUM",
    }
}

impl CommodityType {
    /// The metal's ISO 4217 code, `COMMODITY_SYMBOLS[type]` in Python.
    pub const fn symbol(self) -> &'static str {
        match self {
            CommodityType::Gold => "XAU",
            CommodityType::Silver => "XAG",
            CommodityType::Platinum => "XPT",
            CommodityType::Palladium => "XPD",
        }
    }
}

str_enum! {
    /// `WeightUnit`: the unit a commodity amount is given in.
    pub enum WeightUnit {
        Gram = "GRAM",
        TroyOunce = "TROY_OUNCE",
    }
}

/// `WEIGHT_CONVERSIONS[from][to]`: what one `from` is in `to`. Python's table has no
/// entry from a unit to itself, so that gives `None`, where Python raises `KeyError`.
pub const fn weight_conversion(from: WeightUnit, to: WeightUnit) -> Option<Dezimal> {
    match (from, to) {
        (WeightUnit::Gram, WeightUnit::TroyOunce) => {
            Some(Dezimal::from_scaled(32_150_746_568_628, 15))
        }
        (WeightUnit::TroyOunce, WeightUnit::Gram) => Some(Dezimal::from_scaled(311_034_768, 7)),
        (WeightUnit::Gram, WeightUnit::Gram) | (WeightUnit::TroyOunce, WeightUnit::TroyOunce) => {
            None
        }
    }
}

/// `to_troy_ounces(amount, unit)`: grams are divided by the grams in a troy ounce.
pub fn to_troy_ounces(amount: Dezimal, unit: WeightUnit) -> Result<Dezimal, ArithmeticError> {
    match unit {
        WeightUnit::TroyOunce => Ok(amount),
        WeightUnit::Gram => (amount / Dezimal::from_scaled(311_034_768, 7)).value(),
    }
}

/// `CommodityRegister`: a holding of a metal as the user enters it.
#[derive(Debug, Clone, PartialEq)]
pub struct CommodityRegister {
    pub name: String,
    pub r#type: CommodityType,
    pub amount: Dezimal,
    pub unit: WeightUnit,
    pub market_value: Option<Dezimal>,
    pub initial_investment: Option<Dezimal>,
    pub average_buy_price: Option<Dezimal>,
    pub currency: Option<String>,
}

/// `UpdateCommodityPosition`: the registers that replace the user's commodities.
#[derive(Debug, Clone, PartialEq)]
pub struct UpdateCommodityPosition {
    pub registers: Vec<CommodityRegister>,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d(value: &str) -> Dezimal {
        Dezimal::from_python_str(value).unwrap()
    }

    #[test]
    fn converts_grams_with_the_troy_ounce_factor() {
        assert_eq!(
            to_troy_ounces(d("31.1034768"), WeightUnit::Gram),
            Ok(Dezimal::ONE)
        );
        assert_eq!(
            to_troy_ounces(d("2.5"), WeightUnit::TroyOunce),
            Ok(d("2.5"))
        );
        assert_eq!(
            weight_conversion(WeightUnit::Gram, WeightUnit::TroyOunce),
            Some(d("0.032150746568628"))
        );
        assert_eq!(weight_conversion(WeightUnit::Gram, WeightUnit::Gram), None);
    }

    #[test]
    fn symbols_and_cutoff() {
        assert_eq!(CommodityType::Platinum.symbol(), "XPT");
        assert_eq!(COMMODITY_HISTORIC_CUTOFF.to_string(), "2026-06-12");
    }
}
