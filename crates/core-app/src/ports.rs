// SPDX-License-Identifier: AGPL-3.0-only

//! Ports of the application layer, the counterpart of `finanze/application/ports`.

use std::collections::BTreeSet;
use std::fmt::Debug;

use core_domain::Uuid;
use core_domain::auto_contributions::{ContributionQueryRequest, EntityContributions};
use core_domain::dates::Date;
use core_domain::earnings_expenses::{PendingFlow, PeriodicFlow};
use core_domain::exception::CalculationError;
use core_domain::labeling::{Label, LabelingRule};
use core_domain::loan_calculator::{LoanCalculationParams, LoanCalculationResult};
use core_domain::settings::{PartialSettings, SettingsSection};

/// The current date. Injected so that calculations are reproducible in tests.
pub trait Clock: Send + Sync + Debug {
    /// Today in the server's local time zone, like Python's `date.today()`.
    fn today(&self) -> Date;
}

/// A clock that always returns the same date.
#[derive(Debug, Clone, Copy)]
pub struct FixedClock(pub Date);

impl Clock for FixedClock {
    fn today(&self) -> Date {
        self.0
    }
}

/// `LoanCalculatorPort`: computes the next installment of a loan.
pub trait LoanCalculatorPort: Send + Sync + Debug {
    fn calculate(
        &self,
        params: &LoanCalculationParams,
    ) -> Result<LoanCalculationResult, CalculationError>;
}

/// A port failed in a way its caller cannot handle. Python raises an exception
/// there, which the API answers with `500 UNEXPECTED_ERROR` and its message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PortError(pub String);

impl std::fmt::Display for PortError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for PortError {}

/// `ConfigPort`: the stored settings.
///
/// Ports that read the user's database are built for one request from the
/// database the login opened, so they borrow it and need not be `Send`.
pub trait ConfigPort: Debug {
    /// `load_sections`: each section asked for as stored, or its default when
    /// nothing valid is stored, with the time of the last change among them.
    fn load_sections(
        &self,
        sections: &BTreeSet<SettingsSection>,
    ) -> Result<PartialSettings, PortError>;
}

/// `LabelPort`: the labels of transactions, the reading half.
pub trait LabelPort: Debug {
    /// `get_all`: every label, those with a key first, then by creation and name.
    fn get_all(&self) -> Result<Vec<Label>, PortError>;

    /// `get_usage`: how many transactions carry each label, by the label's id
    /// (`str(UUID(...))`); labels no transaction carries are missing.
    fn get_usage(&self) -> Result<std::collections::HashMap<String, u64>, PortError>;
}

/// Why a port could not read what it was asked for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReadError {
    /// The read failed as it fails in Python, which answers `500`.
    Failed(PortError),
    /// The stored data has a form Python's writer does not produce and Rust does
    /// not read exactly as Python does, such as a value of the wrong type that
    /// pydantic may coerce. The route passes the request on to Python, which
    /// answers as it always has.
    Unreadable(String),
}

impl std::fmt::Display for ReadError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ReadError::Failed(error) => write!(f, "{error}"),
            ReadError::Unreadable(reason) => write!(f, "unreadable stored data: {reason}"),
        }
    }
}

impl std::error::Error for ReadError {}

impl From<PortError> for ReadError {
    fn from(error: PortError) -> Self {
        ReadError::Failed(error)
    }
}

/// `LabelingRulePort`: the rules that label transactions, the reading half.
pub trait LabelingRulePort: Debug {
    /// `get_all(enabled_only=False)`: every rule, by creation and id.
    fn get_all(&self) -> Result<Vec<LabelingRule>, ReadError>;
}

/// `PeriodicFlowPort`: the recurring earnings and expenses, the reading half.
pub trait PeriodicFlowPort: Debug {
    /// `get_all`: every flow in the order the database returns them, each with
    /// its real estate link and without a next date.
    fn get_all(&self) -> Result<Vec<PeriodicFlow>, ReadError>;
}

/// `PendingFlowPort`: the single expected earnings and expenses, the reading half.
pub trait PendingFlowPort: Debug {
    /// `get_all`: every flow in the order the database returns them.
    fn get_all(&self) -> Result<Vec<PendingFlow>, ReadError>;
}

/// `AutoContributionsPort`: the periodic contributions, the reading half.
pub trait AutoContributionsPort: Debug {
    /// `get_all_grouped_by_entity`: the contributions by entity id, in the order
    /// the database returns the rows.
    fn get_all_grouped_by_entity(
        &self,
        query: &ContributionQueryRequest,
    ) -> Result<EntityContributions, ReadError>;
}

/// `EntityPort`: the reading half of the entities the use cases need.
pub trait EntityPort: Debug {
    /// `get_disabled_entities`: the ids of the entities whose data is excluded.
    fn get_disabled_entities(&self) -> Result<Vec<Uuid>, ReadError>;
}
