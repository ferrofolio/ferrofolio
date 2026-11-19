// SPDX-License-Identifier: AGPL-3.0-only

//! Port of the reading half of
//! `finanze/infrastructure/repository/settings/settings_mapper.py`: a stored
//! section becomes the domain type as `SECTION_TYPES[key](**value)` makes it, with
//! the coercion of pydantic in lax mode, or the section's default when it is not
//! valid.
//!
//! pydantic reads a dataclass from a dict and ignores unknown keys; `str` takes
//! only strings, `bool` and `int` also take some numbers and strings (see
//! `boolean` and `integer`), enums and literals take their exact values, lists
//! only lists, and a field that is missing takes its default or fails when it has
//! none. Errors name the field and the rule, never the value, which may be an IBAN.

use core_domain::settings::{
    AccountAssetConfig, AccountExclusion, AiConfig, AiImportConfig, AiProviderEntry, AssetConfig,
    AutoRefresh, AutoRefreshEntityEntry, AutoRefreshMaxOutdatedTime, AutoRefreshMode,
    CryptoAssetConfig, DataConfig, EditMode, ExportConfig, ExportSheetConfig,
    ExternalLabelingConfig, ExternalLabelingExamplesConfig, FilterConfig, FilterValues,
    GeneralConfig, ImportConfig, ImportSheetConfig, InvalidSettings, LabelOrigin, LabelingConfig,
    SectionConfig, SettingsSection, SheetsExportConfig, SheetsGlobalConfig, SheetsImportConfig,
    TemplateConfig, validate_ai_providers, validate_exclusions,
};
use serde_json::{Map, Value};

/// Why a stored section is not valid.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{0}")]
pub struct InvalidSection(String);

type Result<T> = std::result::Result<T, InvalidSection>;

/// `deserialize_section`: the stored text as JSON, or `None` when it is not JSON.
pub fn deserialize_section(text: &[u8]) -> Option<Value> {
    serde_json::from_slice(text).ok()
}

/// `parse_section`: the section a stored value describes.
pub fn parse_section(section: SettingsSection, value: &Value) -> Result<SectionConfig> {
    if !value.is_object() {
        return Err(InvalidSection(format!(
            "Settings section '{}' must be an object",
            section.value()
        )));
    }
    let path = section.value();
    Ok(match section {
        SettingsSection::General => SectionConfig::General(general(value, path)?),
        SettingsSection::Data => SectionConfig::Data(data(value, path)?),
        SettingsSection::Export => SectionConfig::Export(export(value, path)?),
        SettingsSection::Importing => SectionConfig::Importing(importing(value, path)?),
        SettingsSection::Assets => SectionConfig::Assets(assets(value, path)?),
        SettingsSection::Labeling => {
            SectionConfig::Labeling(labeling(&with_provider_list(value), path)?)
        }
        SettingsSection::Ai => SectionConfig::Ai(AiConfig {
            providers: Object::new(value, path)?.get("providers", Vec::new(), ai_providers)?,
        }),
    })
}

/// A dataclass read from a dict.
struct Object<'a> {
    fields: &'a Map<String, Value>,
    path: &'a str,
}

impl<'a> Object<'a> {
    fn new(value: &'a Value, path: &'a str) -> Result<Self> {
        match value {
            Value::Object(fields) => Ok(Object { fields, path }),
            _ => Err(invalid(path, "should be a dictionary")),
        }
    }

    /// A field with a default.
    fn get<T>(
        &self,
        name: &str,
        default: T,
        parse: impl FnOnce(&Value, &str) -> Result<T>,
    ) -> Result<T> {
        match self.fields.get(name) {
            Some(value) => parse(value, &format!("{}.{name}", self.path)),
            None => Ok(default),
        }
    }

    /// A field without a default.
    fn require<T>(&self, name: &str, parse: impl FnOnce(&Value, &str) -> Result<T>) -> Result<T> {
        let path = format!("{}.{name}", self.path);
        match self.fields.get(name) {
            Some(value) => parse(value, &path),
            None => Err(invalid(&path, "field required")),
        }
    }
}

fn invalid(path: &str, rule: &str) -> InvalidSection {
    InvalidSection(format!("{path}: {rule}"))
}

fn domain(path: &str) -> impl FnOnce(InvalidSettings) -> InvalidSection + '_ {
    move |error| invalid(path, &error.0)
}

fn string(value: &Value, path: &str) -> Result<String> {
    match value {
        Value::String(text) => Ok(text.clone()),
        _ => Err(invalid(path, "should be a valid string")),
    }
}

/// `Optional[...]`: `null` is `None`.
fn optional<T>(
    parse: impl FnOnce(&Value, &str) -> Result<T>,
) -> impl FnOnce(&Value, &str) -> Result<Option<T>> {
    move |value, path| match value {
        Value::Null => Ok(None),
        _ => parse(value, path).map(Some),
    }
}

fn list<T>(
    item: impl Fn(&Value, &str) -> Result<T>,
) -> impl FnOnce(&Value, &str) -> Result<Vec<T>> {
    move |value, path| match value {
        Value::Array(items) => items
            .iter()
            .enumerate()
            .map(|(index, value)| item(value, &format!("{path}.{index}")))
            .collect(),
        _ => Err(invalid(path, "should be a valid list")),
    }
}

/// A `str` enum takes exactly one of its values.
fn enumeration<T>(from_value: fn(&str) -> Option<T>) -> impl Fn(&Value, &str) -> Result<T> {
    move |value, path| {
        value
            .as_str()
            .and_then(from_value)
            .ok_or_else(|| invalid(path, "should be a valid enumeration member"))
    }
}

/// Whether a JSON number is written without a fraction or an exponent, which
/// Python's `json` module reads as an `int`.
fn is_integer_token(number: &serde_json::Number) -> bool {
    !number.to_string().contains(['.', 'e', 'E'])
}

/// `bool` in lax mode: booleans, the numbers 0 and 1, and the strings
/// `0 off f false n no` and `1 on t true y yes` in any case, without surrounding
/// whitespace.
fn boolean(value: &Value, path: &str) -> Result<bool> {
    let parsed = match value {
        Value::Bool(flag) => Some(*flag),
        Value::Number(number) => match number.as_f64() {
            Some(1.0) => Some(true),
            Some(0.0) => Some(false),
            _ => None,
        },
        Value::String(text) => match text.to_ascii_lowercase().as_str() {
            "1" | "on" | "t" | "true" | "y" | "yes" => Some(true),
            "0" | "off" | "f" | "false" | "n" | "no" => Some(false),
            _ => None,
        },
        Value::Null | Value::Array(_) | Value::Object(_) => None,
    };
    parsed.ok_or_else(|| invalid(path, "should be a valid boolean"))
}

/// `int` in lax mode: integers, booleans, floats without a fraction within 64 bits,
/// and integer strings with whitespace around, underscores between digits and an
/// optional fraction of zeros. Integers beyond 64 bits, which pydantic accepts, are
/// rejected (docs/rust-backend.md, known deviations).
fn integer(value: &Value, path: &str) -> Result<i64> {
    let parsed = match value {
        Value::Bool(flag) => Some(i64::from(*flag)),
        Value::Number(number) if is_integer_token(number) => number.to_string().parse().ok(),
        Value::Number(number) => number.as_f64().and_then(|float| {
            let fits = (-9_223_372_036_854_775_808.0..9_223_372_036_854_775_808.0).contains(&float);
            (float.fract() == 0.0 && fits).then_some(float as i64)
        }),
        Value::String(text) => match text.trim().split_once('.') {
            Some((integral, fraction))
                if !fraction.is_empty()
                    && fraction.bytes().all(|byte| byte == b'0')
                    && integral.trim() == integral =>
            {
                integer_literal(integral)
            }
            Some(_) => None,
            None => integer_literal(text),
        },
        Value::Null | Value::Array(_) | Value::Object(_) => None,
    };
    parsed.ok_or_else(|| invalid(path, "should be a valid integer"))
}

/// An integer literal with whitespace around, a sign and underscores between
/// ASCII digits.
fn integer_literal(text: &str) -> Option<i64> {
    let trimmed = text.trim();
    let (sign, digits) = match trimmed.as_bytes().first() {
        Some(b'-') => ("-", trimmed.get(1..)?),
        Some(b'+') => ("", trimmed.get(1..)?),
        _ => ("", trimmed),
    };
    let well_formed = !digits.is_empty()
        && !digits.starts_with('_')
        && !digits.ends_with('_')
        && !digits.contains("__")
        && digits
            .bytes()
            .all(|byte| byte.is_ascii_digit() || byte == b'_');
    if !well_formed {
        return None;
    }
    format!("{sign}{}", digits.replace('_', "")).parse().ok()
}

/// Python's truth value of a JSON value.
fn truthy(value: &Value) -> bool {
    match value {
        Value::Null => false,
        Value::Bool(flag) => *flag,
        // Beyond the float range Python reads infinity, which is true.
        Value::Number(number) => number.as_f64().is_none_or(|float| float != 0.0),
        Value::String(text) => !text.is_empty(),
        Value::Array(items) => !items.is_empty(),
        Value::Object(fields) => !fields.is_empty(),
    }
}

fn general(value: &Value, path: &str) -> Result<GeneralConfig> {
    let object = Object::new(value, path)?;
    let defaults = GeneralConfig::default();
    Ok(GeneralConfig {
        default_currency: object.get("defaultCurrency", defaults.default_currency, string)?,
        default_commodity_weight_unit: object.get(
            "defaultCommodityWeightUnit",
            defaults.default_commodity_weight_unit,
            string,
        )?,
        edit_mode: object.get(
            "editMode",
            defaults.edit_mode,
            enumeration(EditMode::from_value),
        )?,
    })
}

fn data(value: &Value, path: &str) -> Result<DataConfig> {
    let auto_refresh = |value: &Value, path: &str| {
        let object = Object::new(value, path)?;
        let defaults = AutoRefresh::default();
        let entity = |value: &Value, path: &str| {
            Ok(AutoRefreshEntityEntry {
                id: Object::new(value, path)?.require("id", string)?,
            })
        };
        Ok(AutoRefresh {
            mode: object.get(
                "mode",
                defaults.mode,
                enumeration(AutoRefreshMode::from_value),
            )?,
            max_outdated: object.get(
                "max_outdated",
                defaults.max_outdated,
                enumeration(AutoRefreshMaxOutdatedTime::from_value),
            )?,
            entities: object.get("entities", defaults.entities, list(entity))?,
        })
    };
    Ok(DataConfig {
        auto_refresh: Object::new(value, path)?.get(
            "autoRefresh",
            AutoRefresh::default(),
            auto_refresh,
        )?,
    })
}

fn sheets_globals(value: &Value, path: &str) -> Result<SheetsGlobalConfig> {
    let object = Object::new(value, path)?;
    Ok(SheetsGlobalConfig {
        spreadsheet_id: object.require("spreadsheetId", string)?,
        datetime_format: object.get("datetimeFormat", None, optional(string))?,
        date_format: object.get("dateFormat", None, optional(string))?,
    })
}

fn template(value: &Value, path: &str) -> Result<TemplateConfig> {
    let object = Object::new(value, path)?;
    let params = |value: &Value, path: &str| match value {
        Value::Object(fields) => Ok(fields.clone()),
        _ => Err(invalid(path, "should be a valid dictionary")),
    };
    Ok(TemplateConfig {
        id: object.require("id", string)?,
        params: object.get("params", None, optional(params))?,
    })
}

fn filter(value: &Value, path: &str) -> Result<FilterConfig> {
    let object = Object::new(value, path)?;
    // `str | list[str]`.
    let values = |value: &Value, path: &str| match value {
        Value::String(text) => Ok(FilterValues::One(text.clone())),
        _ => list(string)(value, path).map(FilterValues::Many),
    };
    Ok(FilterConfig {
        field: object.require("field", string)?,
        values: object.require("values", values)?,
    })
}

fn export_sheet(value: &Value, path: &str) -> Result<ExportSheetConfig> {
    let object = Object::new(value, path)?;
    Ok(ExportSheetConfig {
        range: object.require("range", string)?,
        spreadsheet_id: object.get("spreadsheetId", None, optional(string))?,
        datetime_format: object.get("datetimeFormat", None, optional(string))?,
        date_format: object.get("dateFormat", None, optional(string))?,
        last_update: object.get("lastUpdate", None, optional(boolean))?,
        data: object.get("data", Vec::new(), list(string))?,
        filters: object.get("filters", None, optional(list(filter)))?,
        template: object.get("template", None, optional(template))?,
    })
}

fn export(value: &Value, path: &str) -> Result<ExportConfig> {
    let sheets = |value: &Value, path: &str| {
        let object = Object::new(value, path)?;
        Ok(SheetsExportConfig {
            globals: object.get("globals", None, optional(sheets_globals))?,
            position: object.get("position", Vec::new(), list(export_sheet))?,
            contributions: object.get("contributions", Vec::new(), list(export_sheet))?,
            transactions: object.get("transactions", Vec::new(), list(export_sheet))?,
            historic: object.get("historic", Vec::new(), list(export_sheet))?,
        })
    };
    Ok(ExportConfig {
        sheets: Object::new(value, path)?.get("sheets", None, optional(sheets))?,
    })
}

fn import_sheet(value: &Value, path: &str) -> Result<ImportSheetConfig> {
    let object = Object::new(value, path)?;
    Ok(ImportSheetConfig {
        range: object.require("range", string)?,
        spreadsheet_id: object.get("spreadsheetId", None, optional(string))?,
        datetime_format: object.get("datetimeFormat", None, optional(string))?,
        date_format: object.get("dateFormat", None, optional(string))?,
        last_update: object.get("lastUpdate", None, optional(boolean))?,
        template: object.get("template", None, optional(template))?,
        data: object.get("data", String::new(), string)?,
    })
}

/// `AIProviderEntry`, with its validators.
fn ai_provider(value: &Value, path: &str) -> Result<AiProviderEntry> {
    let object = Object::new(value, path)?;
    let provider = object.require("provider", string)?;
    let model = object.require("model", string)?;
    let upstream = object.get("upstreamProvider", None, optional(string))?;
    AiProviderEntry::new(&provider, &model, upstream.as_deref()).map_err(domain(path))
}

/// A list of AI providers: at most five, none twice.
fn ai_providers(value: &Value, path: &str) -> Result<Vec<AiProviderEntry>> {
    let providers = list(ai_provider)(value, path)?;
    validate_ai_providers(&providers).map_err(domain(path))?;
    Ok(providers)
}

fn importing(value: &Value, path: &str) -> Result<ImportConfig> {
    let object = Object::new(value, path)?;
    let sheets = |value: &Value, path: &str| {
        let object = Object::new(value, path)?;
        Ok(SheetsImportConfig {
            globals: object.get("globals", None, optional(sheets_globals))?,
            position: object.get("position", None, optional(list(import_sheet)))?,
            transactions: object.get("transactions", None, optional(list(import_sheet)))?,
        })
    };
    let ai = |value: &Value, path: &str| {
        let object = Object::new(value, path)?;
        Ok(AiImportConfig {
            providers: object.get("providers", Vec::new(), ai_providers)?,
            instructions: object.get("instructions", None, optional(string))?,
        })
    };
    Ok(ImportConfig {
        sheets: object.get("sheets", None, optional(sheets))?,
        ai: object.get("ai", AiImportConfig::default(), ai)?,
    })
}

fn assets(value: &Value, path: &str) -> Result<AssetConfig> {
    let object = Object::new(value, path)?;
    let crypto = |value: &Value, path: &str| {
        let object = Object::new(value, path)?;
        Ok(CryptoAssetConfig {
            stablecoins: object.get("stablecoins", Vec::new(), list(string))?,
            hide_unknown_tokens: object.get("hideUnknownTokens", false, boolean)?,
        })
    };
    let exclusion = |value: &Value, path: &str| {
        let object = Object::new(value, path)?;
        // `type: Literal["IBAN"]`, then `value`, whose validator turns any value
        // that is not a string into an invalid IBAN.
        object.require("type", |value, path| match value.as_str() {
            Some("IBAN") => Ok(()),
            _ => Err(invalid(path, "should be 'IBAN'")),
        })?;
        object.require("value", |value, path| {
            let text = value
                .as_str()
                .ok_or_else(|| invalid(path, "Invalid IBAN"))?;
            AccountExclusion::iban(text).map_err(domain(path))
        })
    };
    let accounts = |value: &Value, path: &str| {
        let exclusions =
            Object::new(value, path)?.get("exclusions", Vec::new(), list(exclusion))?;
        validate_exclusions(&exclusions).map_err(domain(path))?;
        Ok(AccountAssetConfig { exclusions })
    };
    Ok(AssetConfig {
        crypto: object.require("crypto", crypto)?,
        accounts: object.get("accounts", AccountAssetConfig::default(), accounts)?,
    })
}

/// Settings of older versions name one provider of the external labeling with
/// `provider`, `model` and `upstreamProvider`; `parse_section` turns them into
/// the list `providers`, unless the section has one already. The three fields
/// are dropped either way.
fn with_provider_list(value: &Value) -> Value {
    let mut value = value.clone();
    let Some(Value::Object(external)) = value.get_mut("external") else {
        return value;
    };
    let provider = external.remove("provider");
    let model = external.remove("model");
    let upstream = external.remove("upstreamProvider");
    if !external.contains_key("providers") {
        let providers = match (provider, model) {
            (Some(provider), Some(model)) if truthy(&provider) && truthy(&model) => {
                let mut entry = Map::new();
                entry.insert("provider".to_owned(), provider);
                entry.insert("model".to_owned(), model);
                entry.insert(
                    "upstreamProvider".to_owned(),
                    upstream.unwrap_or(Value::Null),
                );
                vec![Value::Object(entry)]
            }
            _ => Vec::new(),
        };
        external.insert("providers".to_owned(), Value::Array(providers));
    }
    value
}

fn labeling(value: &Value, path: &str) -> Result<LabelingConfig> {
    let examples = |value: &Value, path: &str| {
        let object = Object::new(value, path)?;
        let defaults = ExternalLabelingExamplesConfig::default();
        Ok(ExternalLabelingExamplesConfig {
            enabled: object.get("enabled", defaults.enabled, boolean)?,
            count: object.get("count", defaults.count, integer)?,
            origins: object.get(
                "origins",
                defaults.origins,
                list(enumeration(LabelOrigin::from_value)),
            )?,
        })
    };
    let external = |value: &Value, path: &str| {
        let object = Object::new(value, path)?;
        let defaults = ExternalLabelingConfig::default();
        Ok(ExternalLabelingConfig {
            enabled: object.get("enabled", defaults.enabled, boolean)?,
            providers: object.get("providers", defaults.providers, ai_providers)?,
            auto_run: object.get("autoRun", defaults.auto_run, boolean)?,
            min_confidence: object.get("minConfidence", defaults.min_confidence, integer)?,
            max_per_run: object.get("maxPerRun", defaults.max_per_run, integer)?,
            examples: object.get("examples", defaults.examples, examples)?,
            instructions: object.get("instructions", defaults.instructions, optional(string))?,
        })
    };
    Ok(LabelingConfig {
        external: Object::new(value, path)?.get(
            "external",
            ExternalLabelingConfig::default(),
            external,
        )?,
    })
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn coerces_like_pydantic_in_lax_mode() {
        for (value, expected) in [
            (json!(true), Some(true)),
            (json!(0), Some(false)),
            (json!(1.0), Some(true)),
            (json!("Off"), Some(false)),
            (json!(2), None),
            (json!(" true"), None),
            (json!(null), None),
        ] {
            assert_eq!(boolean(&value, "x").ok(), expected, "{value}");
        }
        for (value, expected) in [
            (json!(true), Some(1)),
            (json!(5.0), Some(5)),
            (json!(" 5 "), Some(5)),
            (json!("5.0"), Some(5)),
            (json!("1_000"), Some(1000)),
            (json!("+5"), Some(5)),
            (json!(5.5), None),
            (json!("5.5"), None),
            (json!("0x10"), None),
            (json!(1e20), None),
            (json!(null), None),
        ] {
            assert_eq!(integer(&value, "x").ok(), expected, "{value}");
        }
    }

    #[test]
    fn errors_never_show_the_value() {
        let stored = json!({"crypto": {}, "accounts": {"exclusions": [{"type": "IBAN", "value": "DE-SECRET"}]}});
        let error = parse_section(SettingsSection::Assets, &stored).unwrap_err();
        assert_eq!(
            error.to_string(),
            "assets.accounts.exclusions.0.value: Invalid IBAN"
        );
    }

    #[test]
    fn older_labeling_sections_get_a_provider_list() {
        let stored = json!({"external": {"provider": " openai ", "model": "gpt"}});
        let Ok(SectionConfig::Labeling(labeling)) =
            parse_section(SettingsSection::Labeling, &stored)
        else {
            panic!("not read");
        };
        assert_eq!(labeling.external.providers.len(), 1);
        assert_eq!(labeling.external.providers[0].provider, "openai");
    }
}
