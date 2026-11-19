// SPDX-License-Identifier: AGPL-3.0-only

//! Financial entities, from `finanze/domain/entity.py`.

use std::fmt;
use std::hash::{Hash, Hasher};

use uuid::Uuid;

use crate::str_enum::str_enum;

str_enum! {
    /// `Feature`: what can be fetched from an entity.
    pub enum Feature {
        Position = "POSITION",
        AutoContributions = "AUTO_CONTRIBUTIONS",
        Transactions = "TRANSACTIONS",
        Historic = "HISTORIC",
    }
}

str_enum! {
    /// `TransactionKind`: whether a transaction belongs to an account or an investment.
    pub enum TransactionKind {
        Account = "ACCOUNT",
        Investment = "INVESTMENT",
    }
}

str_enum! {
    /// `EntityType`: the kind of institution or source an entity is.
    pub enum EntityType {
        FinancialInstitution = "FINANCIAL_INSTITUTION",
        CryptoWallet = "CRYPTO_WALLET",
        CryptoExchange = "CRYPTO_EXCHANGE",
        MarketForecastPlatform = "MARKET_FORECAST_PLATFORM",
        Commodity = "COMMODITY",
    }
}

str_enum! {
    /// `EntityOrigin`: how an entity came to exist.
    pub enum EntityOrigin {
        Manual = "MANUAL",
        Native = "NATIVE",
        ExternallyProvided = "EXTERNALLY_PROVIDED",
        Internal = "INTERNAL",
    }
}

/// `Entity`: a bank, broker, exchange, wallet or other source of positions.
///
/// As in Python, two entities are equal when all their fields are, and an entity
/// hashes and prints by its name only.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entity {
    pub id: Option<Uuid>,
    pub name: String,
    pub natural_id: Option<String>,
    pub r#type: EntityType,
    pub origin: EntityOrigin,
    pub icon_url: Option<String>,
}

impl fmt::Display for Entity {
    /// `str(entity)`: the name.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.name)
    }
}

impl Hash for Entity {
    /// `hash(entity)` is `hash(entity.name)`; equal entities have equal names.
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.name.hash(state);
    }
}

/// `ACCOUNT_SCOPED_ENTITY_TYPES`: entities whose positions belong to an account of
/// the entity rather than to the entity itself.
pub const ACCOUNT_SCOPED_ENTITY_TYPES: [EntityType; 2] = [
    EntityType::CryptoExchange,
    EntityType::MarketForecastPlatform,
];

/// `is_account_scoped_entity(entity)`.
pub fn is_account_scoped_entity(entity: &Entity) -> bool {
    ACCOUNT_SCOPED_ENTITY_TYPES.contains(&entity.r#type)
}

#[cfg(test)]
mod tests {
    use std::collections::hash_map::DefaultHasher;

    use super::*;

    fn entity(name: &str, r#type: EntityType) -> Entity {
        Entity {
            id: None,
            name: name.to_owned(),
            natural_id: None,
            r#type,
            origin: EntityOrigin::Native,
            icon_url: None,
        }
    }

    fn hash_of(entity: &Entity) -> u64 {
        let mut hasher = DefaultHasher::new();
        entity.hash(&mut hasher);
        hasher.finish()
    }

    #[test]
    fn prints_and_hashes_by_name() {
        let bank = entity("Bank", EntityType::FinancialInstitution);
        let mut other = bank.clone();
        other.origin = EntityOrigin::Manual;
        assert_eq!(bank.to_string(), "Bank");
        assert_ne!(bank, other);
        assert_eq!(hash_of(&bank), hash_of(&other));
    }

    #[test]
    fn exchanges_and_forecast_platforms_are_account_scoped() {
        for r#type in EntityType::ALL {
            let scoped = matches!(
                r#type,
                EntityType::CryptoExchange | EntityType::MarketForecastPlatform
            );
            assert_eq!(is_account_scoped_entity(&entity("E", *r#type)), scoped);
        }
    }
}
