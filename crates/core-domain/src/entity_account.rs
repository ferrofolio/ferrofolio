// SPDX-License-Identifier: AGPL-3.0-only

//! Accounts within an entity, from `finanze/domain/entity_account.py`.

use uuid::Uuid;

use crate::datetime::DateTime;

/// `EntityAccount`: one account of a user at an entity, such as one exchange login.
#[derive(Debug, Clone, PartialEq)]
pub struct EntityAccount {
    pub id: Uuid,
    pub entity_id: Uuid,
    pub created_at: DateTime,
    pub name: Option<String>,
    pub deleted_at: Option<DateTime>,
}
