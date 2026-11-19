// SPDX-License-Identifier: AGPL-3.0-only

//! The user's settings, ported from `finanze/domain/settings.py`.
//!
//! The types serialize to the shape of Python's `dataclasses.asdict`, which is both
//! how the backend stores a section and how the API returns it. The validators of
//! the pydantic dataclasses are functions here; reading a stored section with
//! pydantic's coercion is the job of the repository.

use std::collections::HashSet;

use serde::{Serialize, Serializer};
use serde_json::{Map, Value};

use crate::text;

/// `CURRENT_VERSION`: the version of the settings schema.
pub const CURRENT_VERSION: u32 = 8;
/// `MAX_AI_PROVIDERS`: the most providers a list of AI providers may hold.
pub const MAX_AI_PROVIDERS: usize = 5;
/// `MAX_IBAN_LENGTH` of `finanze/domain/transactions.py`.
pub const MAX_IBAN_LENGTH: usize = 34;
/// Length limits of a provider and a model name, in characters.
const PROVIDER_NAME_LENGTH: std::ops::RangeInclusive<usize> = 1..=200;

/// A settings value that breaks a rule of the domain. The message names the rule,
/// never the value, which may be an IBAN.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{0}")]
pub struct InvalidSettings(pub String);

/// A Python `str` enum: each variant has the string value Python stores.
macro_rules! value_enum {
    ($(#[$meta:meta])* $name:ident { $($variant:ident = $value:literal),+ $(,)? }) => {
        $(#[$meta])*
        #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
        pub enum $name {
            $($variant),+
        }

        impl $name {
            /// Every variant, in the order of the Python enum.
            pub const ALL: &'static [$name] = &[$($name::$variant),+];

            /// The value Python stores.
            pub fn value(self) -> &'static str {
                match self {
                    $($name::$variant => $value),+
                }
            }

            /// `Enum(value)`: the variant with exactly this value.
            pub fn from_value(value: &str) -> Option<Self> {
                Self::ALL.iter().copied().find(|variant| variant.value() == value)
            }
        }

        impl Serialize for $name {
            fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
                serializer.serialize_str(self.value())
            }
        }
    };
}

value_enum! {
    /// `SettingsSection`: the sections, each stored on its own.
    SettingsSection {
        General = "general",
        Data = "data",
        Export = "export",
        Importing = "importing",
        Assets = "assets",
        Labeling = "labeling",
        Ai = "ai",
    }
}

value_enum! {
    /// `EditMode`.
    EditMode { Draft = "DRAFT", Quick = "QUICK" }
}

value_enum! {
    /// `AutoRefreshMode`.
    AutoRefreshMode { Off = "OFF", No2fa = "NO_2FA" }
}

value_enum! {
    /// `AutoRefreshMaxOutdatedTime`.
    AutoRefreshMaxOutdatedTime {
        ThreeHours = "THREE_HOURS",
        SixHours = "SIX_HOURS",
        TwelveHours = "TWELVE_HOURS",
        Day = "DAY",
        TwoDays = "TWO_DAYS",
        Week = "WEEK",
    }
}

value_enum! {
    /// `LabelOrigin` of `finanze/domain/transactions.py`.
    LabelOrigin { Manual = "MANUAL", Rule = "RULE", External = "EXTERNAL" }
}

/// `WeightUnit.GRAM` of `finanze/domain/commodity.py`, the default weight unit.
pub const GRAM: &str = "GRAM";

/// `GeneralConfig`.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GeneralConfig {
    pub default_currency: String,
    /// Any string; Python does not check it against `WeightUnit`.
    pub default_commodity_weight_unit: String,
    pub edit_mode: EditMode,
}

impl Default for GeneralConfig {
    fn default() -> Self {
        GeneralConfig {
            default_currency: "EUR".to_owned(),
            default_commodity_weight_unit: GRAM.to_owned(),
            edit_mode: EditMode::Quick,
        }
    }
}

/// `AutoRefreshEntityEntry`.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct AutoRefreshEntityEntry {
    pub id: String,
}

/// `AutoRefresh`.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct AutoRefresh {
    pub mode: AutoRefreshMode,
    pub max_outdated: AutoRefreshMaxOutdatedTime,
    pub entities: Vec<AutoRefreshEntityEntry>,
}

impl Default for AutoRefresh {
    fn default() -> Self {
        AutoRefresh {
            mode: AutoRefreshMode::No2fa,
            max_outdated: AutoRefreshMaxOutdatedTime::TwelveHours,
            entities: Vec::new(),
        }
    }
}

/// `DataConfig`.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DataConfig {
    pub auto_refresh: AutoRefresh,
}

/// `SheetsGlobalConfig`.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SheetsGlobalConfig {
    pub spreadsheet_id: String,
    pub datetime_format: Option<String>,
    pub date_format: Option<String>,
}

/// `FilterValues`: one value or several.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(untagged)]
pub enum FilterValues {
    One(String),
    Many(Vec<String>),
}

/// `FilterConfig`.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct FilterConfig {
    pub field: String,
    pub values: FilterValues,
}

/// `TemplateConfig`: a template and its parameters, which can be any JSON object.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct TemplateConfig {
    pub id: String,
    pub params: Option<Map<String, Value>>,
}

/// `ExportSheetConfig` and its subclasses for positions, contributions,
/// transactions and history, which add nothing to it.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportSheetConfig {
    pub range: String,
    pub spreadsheet_id: Option<String>,
    pub datetime_format: Option<String>,
    pub date_format: Option<String>,
    pub last_update: Option<bool>,
    pub data: Vec<String>,
    pub filters: Option<Vec<FilterConfig>>,
    pub template: Option<TemplateConfig>,
}

/// `SheetsExportConfig`.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct SheetsExportConfig {
    pub globals: Option<SheetsGlobalConfig>,
    pub position: Vec<ExportSheetConfig>,
    pub contributions: Vec<ExportSheetConfig>,
    pub transactions: Vec<ExportSheetConfig>,
    pub historic: Vec<ExportSheetConfig>,
}

/// `ExportConfig`.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct ExportConfig {
    pub sheets: Option<SheetsExportConfig>,
}

/// `ImportSheetConfig` and its subclasses for positions and transactions.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportSheetConfig {
    pub range: String,
    pub spreadsheet_id: Option<String>,
    pub datetime_format: Option<String>,
    pub date_format: Option<String>,
    pub last_update: Option<bool>,
    pub template: Option<TemplateConfig>,
    pub data: String,
}

/// `SheetsImportConfig`.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct SheetsImportConfig {
    pub globals: Option<SheetsGlobalConfig>,
    pub position: Option<Vec<ImportSheetConfig>>,
    pub transactions: Option<Vec<ImportSheetConfig>>,
}

/// `AIProviderEntry`. Build it with [`AiProviderEntry::new`], which applies the
/// validators of the Python class.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AiProviderEntry {
    pub provider: String,
    pub model: String,
    pub upstream_provider: Option<String>,
}

impl AiProviderEntry {
    /// Strips every value, as `trim_values` does, and requires a provider and a
    /// model of 1 to 200 characters.
    pub fn new(
        provider: &str,
        model: &str,
        upstream_provider: Option<&str>,
    ) -> Result<Self, InvalidSettings> {
        let name = |value: &str, field: &str| {
            let value = text::strip(value);
            if PROVIDER_NAME_LENGTH.contains(&value.chars().count()) {
                Ok(value.to_owned())
            } else {
                Err(InvalidSettings(format!(
                    "{field} must have 1 to 200 characters"
                )))
            }
        };
        Ok(AiProviderEntry {
            provider: name(provider, "provider")?,
            model: name(model, "model")?,
            upstream_provider: upstream_provider.map(|value| text::strip(value).to_owned()),
        })
    }
}

/// The rules of every list of AI providers: at most `MAX_AI_PROVIDERS`, and no two
/// with the same provider, model and upstream provider, where an empty upstream
/// provider counts as none (`_unique_entries`).
pub fn validate_ai_providers(providers: &[AiProviderEntry]) -> Result<(), InvalidSettings> {
    if providers.len() > MAX_AI_PROVIDERS {
        return Err(InvalidSettings(format!(
            "at most {MAX_AI_PROVIDERS} AI providers are allowed"
        )));
    }
    let mut seen = HashSet::new();
    for entry in providers {
        let upstream = entry
            .upstream_provider
            .as_deref()
            .filter(|upstream| !upstream.is_empty());
        if !seen.insert((entry.provider.as_str(), entry.model.as_str(), upstream)) {
            return Err(InvalidSettings("Duplicate AI providers".to_owned()));
        }
    }
    Ok(())
}

/// `AIConfig`.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct AiConfig {
    pub providers: Vec<AiProviderEntry>,
}

/// `AIImportConfig`.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct AiImportConfig {
    pub providers: Vec<AiProviderEntry>,
    pub instructions: Option<String>,
}

/// `ImportConfig`.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct ImportConfig {
    pub sheets: Option<SheetsImportConfig>,
    pub ai: AiImportConfig,
}

/// `CryptoAssetConfig`.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CryptoAssetConfig {
    pub stablecoins: Vec<String>,
    pub hide_unknown_tokens: bool,
}

/// `AccountExclusion`: an account excluded by its IBAN, the only type there is.
#[derive(Clone, PartialEq, Eq)]
pub struct AccountExclusion {
    iban: String,
}

impl std::fmt::Debug for AccountExclusion {
    // IBANs stay out of logs.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AccountExclusion").finish_non_exhaustive()
    }
}

impl AccountExclusion {
    /// `validate_iban`: the IBAN without whitespace and in upper case, which must
    /// be ASCII letters and digits, at most `MAX_IBAN_LENGTH` of them.
    pub fn iban(value: &str) -> Result<Self, InvalidSettings> {
        let normalized = text::remove_whitespace(value).to_uppercase();
        let valid = !normalized.is_empty()
            && normalized.len() <= MAX_IBAN_LENGTH
            && normalized.bytes().all(|byte| byte.is_ascii_alphanumeric());
        if valid {
            Ok(AccountExclusion { iban: normalized })
        } else {
            Err(InvalidSettings("Invalid IBAN".to_owned()))
        }
    }

    pub fn value(&self) -> &str {
        &self.iban
    }
}

impl Serialize for AccountExclusion {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeStruct;
        let mut fields = serializer.serialize_struct("AccountExclusion", 2)?;
        fields.serialize_field("type", "IBAN")?;
        fields.serialize_field("value", &self.iban)?;
        fields.end()
    }
}

/// `AccountAssetConfig`.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct AccountAssetConfig {
    pub exclusions: Vec<AccountExclusion>,
}

/// `unique_exclusions`: no IBAN twice.
pub fn validate_exclusions(exclusions: &[AccountExclusion]) -> Result<(), InvalidSettings> {
    let mut seen = HashSet::new();
    if exclusions
        .iter()
        .all(|exclusion| seen.insert(exclusion.value()))
    {
        Ok(())
    } else {
        Err(InvalidSettings("Duplicate account exclusions".to_owned()))
    }
}

/// `AssetConfig`. It has no default in Python; the base configuration supplies
/// one with the default stablecoins.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct AssetConfig {
    pub crypto: CryptoAssetConfig,
    pub accounts: AccountAssetConfig,
}

/// `ExternalLabelingExamplesConfig`.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ExternalLabelingExamplesConfig {
    pub enabled: bool,
    pub count: i64,
    pub origins: Vec<LabelOrigin>,
}

impl Default for ExternalLabelingExamplesConfig {
    fn default() -> Self {
        ExternalLabelingExamplesConfig {
            enabled: false,
            count: 20,
            origins: vec![LabelOrigin::Manual, LabelOrigin::Rule],
        }
    }
}

/// `ExternalLabelingConfig`.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExternalLabelingConfig {
    pub enabled: bool,
    pub providers: Vec<AiProviderEntry>,
    pub auto_run: bool,
    pub min_confidence: i64,
    pub max_per_run: i64,
    pub examples: ExternalLabelingExamplesConfig,
    pub instructions: Option<String>,
}

impl Default for ExternalLabelingConfig {
    fn default() -> Self {
        ExternalLabelingConfig {
            enabled: false,
            providers: Vec::new(),
            auto_run: true,
            min_confidence: 70,
            max_per_run: 400,
            examples: ExternalLabelingExamplesConfig::default(),
            instructions: None,
        }
    }
}

/// `LabelingConfig`.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct LabelingConfig {
    pub external: ExternalLabelingConfig,
}

/// One section of the settings.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(untagged)]
pub enum SectionConfig {
    General(GeneralConfig),
    Data(DataConfig),
    Export(ExportConfig),
    Importing(ImportConfig),
    Assets(AssetConfig),
    Labeling(LabelingConfig),
    Ai(AiConfig),
}

/// `PartialSettings`: the sections asked for, the time of their last change and
/// the schema version.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PartialSettings {
    pub last_update: String,
    pub version: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub general: Option<GeneralConfig>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<DataConfig>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub export: Option<ExportConfig>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub importing: Option<ImportConfig>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub assets: Option<AssetConfig>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub labeling: Option<LabelingConfig>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ai: Option<AiConfig>,
}

impl PartialSettings {
    /// Settings without sections, at the current version.
    pub fn new(last_update: String) -> Self {
        PartialSettings {
            last_update,
            version: CURRENT_VERSION,
            general: None,
            data: None,
            export: None,
            importing: None,
            assets: None,
            labeling: None,
            ai: None,
        }
    }

    /// Puts a section in its place.
    pub fn set(&mut self, section: SectionConfig) {
        match section {
            SectionConfig::General(value) => self.general = Some(value),
            SectionConfig::Data(value) => self.data = Some(value),
            SectionConfig::Export(value) => self.export = Some(value),
            SectionConfig::Importing(value) => self.importing = Some(value),
            SectionConfig::Assets(value) => self.assets = Some(value),
            SectionConfig::Labeling(value) => self.labeling = Some(value),
            SectionConfig::Ai(value) => self.ai = Some(value),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn enums_read_only_their_values() {
        assert_eq!(SettingsSection::from_value("ai"), Some(SettingsSection::Ai));
        assert_eq!(SettingsSection::from_value("AI"), None);
        assert_eq!(AutoRefreshMode::No2fa.value(), "NO_2FA");
        assert!(SettingsSection::General < SettingsSection::Ai);
    }

    #[test]
    fn providers_are_stripped_and_unique() {
        let entry = AiProviderEntry::new(" a\u{3000}", "\u{1C}m", Some(" u ")).unwrap();
        assert_eq!((entry.provider.as_str(), entry.model.as_str()), ("a", "m"));
        assert_eq!(entry.upstream_provider.as_deref(), Some("u"));
        assert!(AiProviderEntry::new(" ", "m", None).is_err());
        assert!(AiProviderEntry::new(&"😀".repeat(200), "m", None).is_ok());
        assert!(AiProviderEntry::new(&"a".repeat(201), "m", None).is_err());

        let plain = AiProviderEntry::new("a", "m", None).unwrap();
        let empty_upstream = AiProviderEntry::new("a", "m", Some("")).unwrap();
        assert!(validate_ai_providers(&[plain.clone(), empty_upstream]).is_err());
        assert!(validate_ai_providers(std::slice::from_ref(&plain)).is_ok());
    }

    #[test]
    fn ibans_are_normalized() {
        let iban = |value: &str| AccountExclusion::iban(value).map(|iban| iban.value().to_owned());
        assert_eq!(iban("de89 3704\u{A0}0044").unwrap(), "DE8937040044");
        assert_eq!(iban("\u{131}ban").unwrap(), "IBAN");
        assert_eq!(iban("\u{DF}x").unwrap(), "SSX");
        assert!(iban("").is_err());
        assert!(iban("DE-89").is_err());
        assert!(iban(&"\u{DF}".repeat(18)).is_err());
        assert!(iban("\u{FF24}E89").is_err());
    }
}
