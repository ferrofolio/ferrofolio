// SPDX-License-Identifier: AGPL-3.0-only

//! The settings repository, the reading half of
//! `finanze/infrastructure/repository/settings`: one row per section in the
//! `settings` table, its value the section as JSON, with the time it was written.

pub mod base_config;
pub mod mapper;

use std::collections::BTreeSet;

use core_app::ports::{ConfigPort, PortError};
use core_domain::datetime::{DateTime, DateTimeError};
use core_domain::settings::{PartialSettings, SectionConfig, SettingsSection};
use core_domain::text;
use jiff::tz::TimeZone;
use rusqlite::types::ValueRef;
use serde_json::Value;

use crate::Database;
use base_config::BaseConfig;

const SECTION_COUNT: usize = 7;

/// `SettingsQueries.get_by_keys` for every section at once.
const GET_BY_KEYS: &str =
    r#"SELECT "key", value, updated_at FROM settings WHERE "key" IN (?, ?, ?, ?, ?, ?, ?)"#;

/// A stored row: its value and its `updated_at`, each as SQLite holds it.
#[derive(Debug, Clone, PartialEq)]
struct Row {
    key: String,
    value: Option<Vec<u8>>,
    updated_at: Option<String>,
}

/// `SettingsRepository`, for one request on the database a login opened.
#[derive(Debug)]
pub struct SettingsRepository<'a> {
    database: &'a Database,
    base: &'a BaseConfig,
    time_zone: &'a TimeZone,
}

impl<'a> SettingsRepository<'a> {
    /// `time_zone` is the server's local time zone, in which Python reads a stored
    /// time without an offset.
    pub fn new(database: &'a Database, base: &'a BaseConfig, time_zone: &'a TimeZone) -> Self {
        SettingsRepository {
            database,
            base,
            time_zone,
        }
    }

    /// `_get_rows`, in the order SQLite returns them, which decides between equal
    /// times as in Python.
    fn rows(&self, keys: &[SettingsSection]) -> Result<Vec<Row>, rusqlite::Error> {
        let mut statement = self.database.connection.prepare(GET_BY_KEYS)?;
        // One parameter per section; those not asked for are NULL, which matches no row.
        let mut parameters: [Option<&str>; SECTION_COUNT] = [None; SECTION_COUNT];
        for (parameter, key) in parameters.iter_mut().zip(keys) {
            *parameter = Some(key.value());
        }
        let rows = statement.query_map(rusqlite::params_from_iter(parameters), |row| {
            // `json.loads` reads text and bytes; other values fail and the
            // section is skipped. Only text is a timestamp.
            let value = match row.get_ref(1)? {
                ValueRef::Text(bytes) | ValueRef::Blob(bytes) => Some(bytes.to_vec()),
                _ => None,
            };
            let updated_at = match row.get_ref(2)? {
                ValueRef::Text(bytes) => Some(String::from_utf8_lossy(bytes).into_owned()),
                _ => None,
            };
            Ok(Row {
                key: row.get(0)?,
                value,
                updated_at,
            })
        })?;
        rows.collect()
    }
}

impl ConfigPort for SettingsRepository<'_> {
    fn load_sections(
        &self,
        sections: &BTreeSet<SettingsSection>,
    ) -> Result<PartialSettings, PortError> {
        // In the order of the enum, as Python asks for them.
        let keys: Vec<SettingsSection> = SettingsSection::ALL
            .iter()
            .copied()
            .filter(|section| sections.contains(section))
            .collect();
        let rows = if keys.is_empty() {
            Vec::new()
        } else {
            self.rows(&keys)
                .map_err(|error| PortError(error.to_string()))?
        };
        let latest = last_update(
            rows.iter().map(|row| row.updated_at.as_deref()),
            self.time_zone,
        )
        .map_err(|error| PortError(error.to_string()))?;
        let mut settings = PartialSettings::new(latest.map_or_else(
            || self.base.last_update().to_owned(),
            |time| time.isoformat(),
        ));
        for key in keys {
            let stored = rows
                .iter()
                .find(|row| row.key == key.value())
                .and_then(|row| stored_section(key, row.value.as_deref()));
            settings.set(resolve_section(key, stored.as_ref(), self.base));
        }
        Ok(settings)
    }
}

/// `deserialize_section` of a stored value; one that is not JSON is skipped, as
/// Python skips it with a warning.
fn stored_section(key: SettingsSection, value: Option<&[u8]>) -> Option<Value> {
    let parsed = value.and_then(mapper::deserialize_section);
    if parsed.is_none() {
        tracing::warn!(section = key.value(), "Invalid stored settings section");
    }
    parsed
}

/// `_resolve_sections` for one section: the stored value if it is valid, else the
/// default. A stored `null` counts as nothing stored.
pub fn resolve_section(
    section: SettingsSection,
    stored: Option<&Value>,
    base: &BaseConfig,
) -> SectionConfig {
    match stored.filter(|value| !value.is_null()) {
        Some(value) => mapper::parse_section(section, value).unwrap_or_else(|error| {
            tracing::warn!(
                section = section.value(),
                %error,
                "Invalid stored settings section, using defaults"
            );
            base.section(section)
        }),
        None => base.section(section),
    }
}

/// `parse_timestamp` of a stored `updated_at`: Python's `fromisoformat` of the
/// stripped text, a time without an offset taken as local time. Text it cannot
/// read counts as no time; making a local time aware can fail like Python's
/// `astimezone`, and the error then fails the request.
pub fn parse_timestamp(
    text: &str,
    time_zone: &TimeZone,
) -> Result<Option<DateTime>, DateTimeError> {
    match DateTime::from_isoformat(text::strip(text)) {
        Ok(parsed) => parsed.localize(time_zone).map(Some),
        Err(_) => Ok(None),
    }
}

/// `_last_update`: the latest of the times stored with the rows, the first of
/// equal instants; `None` without any.
pub fn last_update<'t>(
    stored: impl IntoIterator<Item = Option<&'t str>>,
    time_zone: &TimeZone,
) -> Result<Option<DateTime>, DateTimeError> {
    let mut latest: Option<(i128, DateTime)> = None;
    for text in stored.into_iter().flatten() {
        let Some(time) = parse_timestamp(text, time_zone)? else {
            continue;
        };
        let Some(instant) = time.timestamp_micros() else {
            continue;
        };
        if latest.as_ref().is_none_or(|(best, _)| instant > *best) {
            latest = Some((instant, time));
        }
    }
    Ok(latest.map(|(_, time)| time))
}
