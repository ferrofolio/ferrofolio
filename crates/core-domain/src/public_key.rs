// SPDX-License-Identifier: AGPL-3.0-only

//! Addresses derived from extended public keys, from `finanze/domain/public_key.py`.

use crate::entity::Entity;
use crate::str_enum::str_enum;

str_enum! {
    /// `CoinType`: the chain an extended public key belongs to.
    pub enum CoinType {
        Bitcoin = "BITCOIN",
        Litecoin = "LITECOIN",
    }
}

str_enum! {
    /// `ScriptType`: how addresses are derived from a public key.
    pub enum ScriptType {
        P2pkh = "p2pkh",
        P2shP2wpkh = "p2sh-p2wpkh",
        P2wpkh = "p2wpkh",
        P2tr = "p2tr",
    }
}

/// `DerivedAddress`: one address derived from an extended public key.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct DerivedAddress {
    pub index: i64,
    pub path: String,
    pub address: String,
    pub pubkey: String,
    pub change: i64,
}

/// `DerivedAddressesResult`: the receiving and change addresses of a key.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DerivedAddressesResult {
    pub key_type: String,
    pub script_type: ScriptType,
    pub coin: CoinType,
    pub receiving: Vec<DerivedAddress>,
    pub change: Vec<DerivedAddress>,
    /// Empty by default.
    pub base_path: String,
}

/// `AddressDerivationRequest`: which addresses to derive from a key.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct AddressDerivationRequest {
    pub xpub: String,
    pub coin: CoinType,
    /// Start and end of the receiving indices, `(0, 30)` by default.
    pub receiving_range: (i64, i64),
    /// Start and end of the change indices, `(0, 30)` by default.
    pub change_range: (i64, i64),
    pub script_type: Option<ScriptType>,
}

impl AddressDerivationRequest {
    /// The default range of indices, `(0, 30)`.
    pub const DEFAULT_RANGE: (i64, i64) = (0, 30);

    /// `AddressDerivationRequest(xpub, coin)`: the other fields take their defaults.
    pub fn new(xpub: impl Into<String>, coin: CoinType) -> Self {
        AddressDerivationRequest {
            xpub: xpub.into(),
            coin,
            receiving_range: Self::DEFAULT_RANGE,
            change_range: Self::DEFAULT_RANGE,
            script_type: None,
        }
    }
}

/// `AddressDerivationPreviewRequest`: a few addresses of a key, to show before
/// connecting it.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct AddressDerivationPreviewRequest {
    pub xpub: String,
    pub entity: Entity,
    /// Number of addresses, 5 by default.
    pub range: i64,
    pub script_type: Option<ScriptType>,
}

impl AddressDerivationPreviewRequest {
    /// The default number of addresses.
    pub const DEFAULT_RANGE: i64 = 5;

    /// `AddressDerivationPreviewRequest(xpub, entity)`: the other fields take their
    /// defaults.
    pub fn new(xpub: impl Into<String>, entity: Entity) -> Self {
        AddressDerivationPreviewRequest {
            xpub: xpub.into(),
            entity,
            range: Self::DEFAULT_RANGE,
            script_type: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::entity::{EntityOrigin, EntityType};

    #[test]
    fn requests_take_python_defaults() {
        let request = AddressDerivationRequest::new("xpub", CoinType::Bitcoin);
        assert_eq!(request.receiving_range, (0, 30));
        assert_eq!(request.change_range, (0, 30));
        assert_eq!(request.script_type, None);

        let entity = Entity {
            id: None,
            name: "Bitcoin".to_owned(),
            natural_id: None,
            r#type: EntityType::CryptoWallet,
            origin: EntityOrigin::Native,
            icon_url: None,
        };
        let preview = AddressDerivationPreviewRequest::new("xpub", entity);
        assert_eq!(preview.range, 5);
        assert_eq!(preview.script_type, None);
    }
}
