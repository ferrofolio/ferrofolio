// SPDX-License-Identifier: AGPL-3.0-only

//! Ports of the application layer, the counterpart of `finanze/application/ports`.

use std::collections::BTreeSet;
use std::fmt::Debug;

use core_domain::dates::Date;
use core_domain::exception::CalculationError;
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
