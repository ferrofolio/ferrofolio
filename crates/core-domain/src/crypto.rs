// SPDX-License-Identifier: AGPL-3.0-only

//! Crypto wallets, assets and fetch results, from `finanze/domain/crypto.py`.

use std::ops::Add;

use indexmap::IndexMap;
use uuid::Uuid;

use crate::dezimal::{ArithmeticError, Dezimal};
use crate::external_integration::{EnabledExternalIntegrations, ExternalIntegrationId};
use crate::public_key::{CoinType, ScriptType};
use crate::str_enum::str_enum;

str_enum! {
    /// `CryptoCurrencyType`: a chain's native coin or a token on it.
    pub enum CryptoCurrencyType {
        Native = "NATIVE",
        Token = "TOKEN",
    }
}

str_enum! {
    /// `CryptoPositionType`: how an amount is held; `HOLDING` by default.
    #[derive(Default)]
    pub enum CryptoPositionType {
        #[default]
        Holding = "HOLDING",
        Supplied = "SUPPLIED",
        Borrowed = "BORROWED",
        Staked = "STAKED",
        Lp = "LP",
        Reward = "REWARD",
    }
}

/// `crypto_rate_key(asset_type, symbol, contract_address)`: the key of an asset's
/// exchange rate, the contract address in lower case for a token that has one, else
/// the symbol in upper case. Empty strings count as missing, as in Python.
///
/// Case follows Unicode as the standard library knows it, version 17 for Rust 1.99,
/// while Python 3.14 knows version 16: the 56 letters added in between change case
/// here and stay as they are in Python. No ticker symbol or hex address has them.
pub fn crypto_rate_key(
    asset_type: CryptoCurrencyType,
    symbol: Option<&str>,
    contract_address: Option<&str>,
) -> Option<String> {
    match (asset_type, contract_address) {
        (CryptoCurrencyType::Token, Some(address)) if !address.is_empty() => {
            Some(address.to_lowercase())
        }
        _ => symbol
            .filter(|symbol| !symbol.is_empty())
            .map(str::to_uppercase),
    }
}

/// `CryptoAsset`: an asset as a market data provider knows it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CryptoAsset {
    pub name: String,
    pub symbol: Option<String>,
    pub icon_urls: Option<Vec<String>>,
    pub external_ids: IndexMap<String, String>,
    pub id: Option<Uuid>,
}

/// `HDAddress`: an address of a hierarchical deterministic wallet.
#[derive(Debug, Clone, PartialEq)]
pub struct HDAddress {
    pub address: String,
    pub index: i64,
    pub change: i64,
    pub path: String,
    pub pubkey: String,
    /// Zero by default.
    pub balance: Dezimal,
}

/// `HDWallet`: a wallet given by its extended public key.
#[derive(Debug, Clone, PartialEq)]
pub struct HDWallet {
    pub xpub: String,
    pub addresses: Vec<HDAddress>,
    pub script_type: ScriptType,
    pub coin_type: CoinType,
}

str_enum! {
    /// `AddressSource`: whether a wallet's addresses were derived or entered.
    pub enum AddressSource {
        Derived = "DERIVED",
        Manual = "MANUAL",
    }
}

/// `CryptoWallet`: a connected wallet.
#[derive(Debug, Clone, PartialEq)]
pub struct CryptoWallet {
    pub id: Uuid,
    pub entity_id: Uuid,
    pub addresses: Vec<String>,
    pub name: String,
    pub address_source: AddressSource,
    pub hd_wallet: Option<HDWallet>,
    pub include_wallet_tokens: bool,
}

/// `CryptoFetchRequest`: the addresses to fetch, with the integrations to use.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CryptoFetchRequest {
    pub integrations: EnabledExternalIntegrations,
    pub addresses: Vec<String>,
    pub txs: bool,
    pub include_wallet_tokens: bool,
}

/// `CryptoFetchedPosition`: an amount found at an address.
#[derive(Debug, Clone, PartialEq)]
pub struct CryptoFetchedPosition {
    pub id: Option<Uuid>,
    pub symbol: String,
    pub balance: Dezimal,
    pub r#type: CryptoCurrencyType,
    pub name: Option<String>,
    pub contract_address: Option<String>,
    pub chain: Option<String>,
    pub protocol: Option<String>,
    pub position_type: CryptoPositionType,
    pub market_value: Option<Dezimal>,
    pub currency: Option<String>,
    pub icon_url: Option<String>,
}

/// `CryptoFetchResult`: what was found at one address.
#[derive(Debug, Clone, PartialEq)]
pub struct CryptoFetchResult {
    pub address: String,
    pub assets: Vec<CryptoFetchedPosition>,
    pub has_txs: Option<bool>,
}

/// `CryptoFetchResults`: the results by address, `None` for an address that could
/// not be fetched, in the order they were added.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct CryptoFetchResults {
    pub results: IndexMap<String, Option<CryptoFetchResult>>,
}

/// What `_asset_key` builds: the type, the contract address of a token that has one
/// or else the symbol, the position type, the chain and the protocol.
type AssetKey = (
    CryptoCurrencyType,
    String,
    CryptoPositionType,
    Option<String>,
    Option<String>,
);

fn asset_key(asset: &CryptoFetchedPosition) -> AssetKey {
    let identifier = match &asset.contract_address {
        Some(address) if asset.r#type == CryptoCurrencyType::Token && !address.is_empty() => {
            address.clone()
        }
        _ => asset.symbol.clone(),
    };
    (
        asset.r#type,
        identifier,
        asset.position_type,
        asset.chain.clone(),
        asset.protocol.clone(),
    )
}

/// `_merge_results`: adds the balances and market values of the incoming assets to
/// those with the same key and appends the others.
///
/// As in Python, assets of `existing` that share a key collapse into the last of
/// them, at the place of the first.
fn merge_results(
    existing: &mut CryptoFetchResult,
    incoming: CryptoFetchResult,
) -> Result<(), ArithmeticError> {
    let mut assets: IndexMap<AssetKey, CryptoFetchedPosition> = IndexMap::new();
    for asset in std::mem::take(&mut existing.assets) {
        assets.insert(asset_key(&asset), asset);
    }
    for asset in incoming.assets {
        let key = asset_key(&asset);
        match assets.get_mut(&key) {
            Some(present) => {
                present.balance = (present.balance + asset.balance).value()?;
                if let Some(market_value) = asset.market_value {
                    present.market_value = Some(match present.market_value {
                        None => market_value,
                        Some(present_value) => (present_value + market_value).value()?,
                    });
                }
            }
            None => {
                assets.insert(key, asset);
            }
        }
    }
    existing.assets = assets.into_values().collect();
    if let Some(incoming_has_txs) = incoming.has_txs {
        // `existing.has_txs or incoming.has_txs`
        existing.has_txs = Some(existing.has_txs == Some(true) || incoming_has_txs);
    }
    Ok(())
}

impl Add for CryptoFetchResults {
    type Output = Result<CryptoFetchResults, ArithmeticError>;

    /// `self + other`: the results of both; an address in both gets the merge of
    /// its two results, or the other's where this one has none. Fails, as Python
    /// raises, when adding two amounts leaves the decimal range.
    fn add(self, other: CryptoFetchResults) -> Self::Output {
        let mut combined = self.results;
        for (address, result) in other.results {
            match combined.get_mut(&address) {
                Some(Some(existing)) => {
                    if let Some(incoming) = result {
                        merge_results(existing, incoming)?;
                    }
                }
                _ => {
                    combined.insert(address, result);
                }
            }
        }
        Ok(CryptoFetchResults { results: combined })
    }
}

/// `ConnectCryptoWallet`: a request to connect a wallet.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConnectCryptoWallet {
    pub entity_id: Uuid,
    pub addresses: Vec<String>,
    pub name: String,
    pub address_source: AddressSource,
    pub xpub: Option<String>,
    pub script_type: Option<ScriptType>,
    pub include_wallet_tokens: bool,
}

str_enum! {
    /// `CryptoWalletConnectionFailureCode`: why an address could not be connected.
    pub enum CryptoWalletConnectionFailureCode {
        AddressAlreadyExists = "ADDRESS_ALREADY_EXISTS",
        AddressNotFound = "ADDRESS_NOT_FOUND",
        XpubAlreadyExists = "XPUB_ALREADY_EXISTS",
        TooManyRequests = "TOO_MANY_REQUESTS",
        UnexpectedError = "UNEXPECTED_ERROR",
    }
}

/// `CryptoWalletConnectionResult`: the addresses that failed, with the reason.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CryptoWalletConnectionResult {
    pub failed: IndexMap<String, CryptoWalletConnectionFailureCode>,
}

/// `UpdateCryptoWalletConnection`: renames a wallet.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UpdateCryptoWalletConnection {
    pub id: Uuid,
    pub name: String,
}

/// `CryptoPlatform`: a chain as a market data provider names it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CryptoPlatform {
    pub provider_id: String,
    pub name: String,
    pub icon_url: Option<String>,
}

/// `CryptoAssetPlatform`: a chain an asset exists on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CryptoAssetPlatform {
    pub provider_id: String,
    pub name: String,
    pub contract_address: Option<String>,
    pub icon_url: Option<String>,
    pub related_entity_id: Option<Uuid>,
}

/// `AvailableCryptoAsset`: an asset the user can add.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AvailableCryptoAsset {
    pub name: String,
    pub symbol: String,
    pub platforms: Vec<CryptoAssetPlatform>,
    pub provider: ExternalIntegrationId,
    pub provider_id: String,
}

/// `AvailableCryptoAssets`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AvailableCryptoAssets {
    pub assets: Vec<AvailableCryptoAsset>,
}

/// `AvailableCryptoAssetsRequest`: a search for assets, one page at a time.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AvailableCryptoAssetsRequest {
    pub symbol: Option<String>,
    pub name: Option<String>,
    /// 1 by default.
    pub page: i64,
    /// 50 by default.
    pub limit: i64,
}

impl AvailableCryptoAssetsRequest {
    pub const DEFAULT_PAGE: i64 = 1;
    pub const DEFAULT_LIMIT: i64 = 50;

    /// `AvailableCryptoAssetsRequest(symbol, name)`: the first page of 50.
    pub fn new(symbol: Option<String>, name: Option<String>) -> Self {
        AvailableCryptoAssetsRequest {
            symbol,
            name,
            page: Self::DEFAULT_PAGE,
            limit: Self::DEFAULT_LIMIT,
        }
    }
}

/// `AvailableCryptoAssetsResult`: one page of a search.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AvailableCryptoAssetsResult {
    pub provider: ExternalIntegrationId,
    pub assets: Vec<AvailableCryptoAsset>,
    pub page: i64,
    pub limit: i64,
    pub total: i64,
}

/// `CryptoAssetDetails`: an asset with its prices by currency.
#[derive(Debug, Clone, PartialEq)]
pub struct CryptoAssetDetails {
    pub name: String,
    pub symbol: String,
    pub platforms: Vec<CryptoAssetPlatform>,
    pub provider: ExternalIntegrationId,
    pub provider_id: String,
    pub price: IndexMap<String, Dezimal>,
    pub r#type: CryptoCurrencyType,
    pub icon_url: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d(value: &str) -> Dezimal {
        Dezimal::from_python_str(value).unwrap()
    }

    fn position(symbol: &str, balance: &str, market_value: Option<&str>) -> CryptoFetchedPosition {
        CryptoFetchedPosition {
            id: None,
            symbol: symbol.to_owned(),
            balance: d(balance),
            r#type: CryptoCurrencyType::Native,
            name: None,
            contract_address: None,
            chain: None,
            protocol: None,
            position_type: CryptoPositionType::default(),
            market_value: market_value.map(d),
            currency: None,
            icon_url: None,
        }
    }

    fn results(entries: Vec<(&str, Option<CryptoFetchResult>)>) -> CryptoFetchResults {
        CryptoFetchResults {
            results: entries
                .into_iter()
                .map(|(address, result)| (address.to_owned(), result))
                .collect(),
        }
    }

    fn result(
        address: &str,
        assets: Vec<CryptoFetchedPosition>,
        has_txs: Option<bool>,
    ) -> CryptoFetchResult {
        CryptoFetchResult {
            address: address.to_owned(),
            assets,
            has_txs,
        }
    }

    #[test]
    fn rate_keys_prefer_token_contracts() {
        let token = CryptoCurrencyType::Token;
        assert_eq!(
            crypto_rate_key(token, Some("usdc"), Some("0xAbC")),
            Some("0xabc".to_owned())
        );
        assert_eq!(
            crypto_rate_key(token, Some("usdc"), Some("")),
            Some("USDC".to_owned())
        );
        assert_eq!(
            crypto_rate_key(CryptoCurrencyType::Native, Some("eth"), Some("0xAbC")),
            Some("ETH".to_owned())
        );
        assert_eq!(crypto_rate_key(token, Some(""), None), None);
    }

    #[test]
    fn adding_results_merges_assets_of_the_same_address() {
        let left = results(vec![
            (
                "a",
                Some(result("a", vec![position("ETH", "1.5", None)], None)),
            ),
            ("b", None),
        ]);
        let right = results(vec![
            (
                "a",
                Some(result(
                    "a",
                    vec![
                        position("ETH", "0.5", Some("10")),
                        position("BTC", "1", None),
                    ],
                    Some(false),
                )),
            ),
            ("b", Some(result("b", vec![], Some(true)))),
            ("c", None),
        ]);
        let sum = (left + right).unwrap();
        let addresses: Vec<&str> = sum.results.keys().map(String::as_str).collect();
        assert_eq!(addresses, ["a", "b", "c"]);
        let a = sum.results["a"].as_ref().unwrap();
        assert_eq!(a.assets.len(), 2);
        assert_eq!(a.assets[0].balance, d("2.0"));
        assert_eq!(a.assets[0].market_value, Some(d("10")));
        assert_eq!(a.has_txs, Some(false));
        assert_eq!(sum.results["b"].as_ref().unwrap().has_txs, Some(true));
        assert_eq!(sum.results["c"], None);
    }

    #[test]
    fn adding_results_fails_on_overflow() {
        let huge = "9.999999999999999999999999999E+999999";
        let left = results(vec![(
            "a",
            Some(result("a", vec![position("ETH", huge, None)], None)),
        )]);
        let right = results(vec![(
            "a",
            Some(result("a", vec![position("ETH", huge, None)], None)),
        )]);
        assert_eq!(left + right, Err(ArithmeticError::Overflow));
    }
}
