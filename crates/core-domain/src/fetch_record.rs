// SPDX-License-Identifier: AGPL-3.0-only

//! Fetch records and data sources, from `finanze/domain/fetch_record.py`.

use uuid::Uuid;

use crate::datetime::DateTime;
use crate::entity::Feature;
use crate::str_enum::str_enum;

/// `FetchRecord`: when a feature of an entity was last fetched.
#[derive(Debug, Clone, PartialEq)]
pub struct FetchRecord {
    pub entity_id: Uuid,
    pub feature: Feature,
    pub date: DateTime,
    pub entity_account_id: Option<Uuid>,
}

str_enum! {
    /// `DataSource`: where a position or record comes from. `REAL`, data fetched from
    /// an entity, is the default of every `source` field in Python.
    #[derive(Default)]
    pub enum DataSource {
        Sheets = "SHEETS",
        Manual = "MANUAL",
        #[default]
        Real = "REAL",
    }
}
