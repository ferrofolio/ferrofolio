// SPDX-License-Identifier: AGPL-3.0-only

//! Ports of the application layer, the counterpart of `finanze/application/ports`.

use std::fmt::Debug;

use core_domain::dates::Date;
use core_domain::exception::CalculationError;
use core_domain::loan_calculator::{LoanCalculationParams, LoanCalculationResult};

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
