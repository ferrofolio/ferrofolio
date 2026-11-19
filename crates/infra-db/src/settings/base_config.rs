// SPDX-License-Identifier: AGPL-3.0-only

//! Port of `finanze/infrastructure/config/base_config.py`: the default sections.

use core_domain::settings::{
    AccountAssetConfig, AiConfig, AssetConfig, CryptoAssetConfig, DataConfig, ExportConfig,
    GeneralConfig, ImportConfig, LabelingConfig, SectionConfig, SettingsSection,
};

/// `DEFAULT_STABLECOINS`.
pub const DEFAULT_STABLECOINS: [&str; 27] = [
    "USDT", "USDC", "USDe", "DAI", "USD1", "BUSD", "PYUSD", "PUSD", "FDUSD", "TUSD", "USDD",
    "GUSD", "USDP", "LUSD", "cUSD", "OUSD", "FRAX", "sUSD", "USDX", "USDY", "USYC", "EURC", "EURS",
    "EURT", "PAXG", "XAUT", "JPYC",
];

/// `BASE_CONFIG`: the section a stored one falls back to, and the time the
/// settings report as their last change while no stored section has one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BaseConfig {
    last_update: String,
}

impl BaseConfig {
    /// Python takes `lastUpdate` from the clock when the backend starts
    /// (`datetime.now().astimezone().isoformat()`); the caller passes such a time.
    pub fn new(last_update: String) -> Self {
        BaseConfig { last_update }
    }

    pub fn last_update(&self) -> &str {
        &self.last_update
    }

    /// The default of a section.
    pub fn section(&self, section: SettingsSection) -> SectionConfig {
        match section {
            SettingsSection::General => SectionConfig::General(GeneralConfig::default()),
            SettingsSection::Data => SectionConfig::Data(DataConfig::default()),
            SettingsSection::Export => SectionConfig::Export(ExportConfig::default()),
            SettingsSection::Importing => SectionConfig::Importing(ImportConfig::default()),
            SettingsSection::Assets => SectionConfig::Assets(AssetConfig {
                crypto: CryptoAssetConfig {
                    stablecoins: DEFAULT_STABLECOINS.map(str::to_owned).to_vec(),
                    hide_unknown_tokens: false,
                },
                accounts: AccountAssetConfig::default(),
            }),
            SettingsSection::Labeling => SectionConfig::Labeling(LabelingConfig::default()),
            SettingsSection::Ai => SectionConfig::Ai(AiConfig::default()),
        }
    }
}
