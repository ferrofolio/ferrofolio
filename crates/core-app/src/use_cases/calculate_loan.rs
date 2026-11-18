// SPDX-License-Identifier: AGPL-3.0-only

//! Port of `finanze/application/use_cases/calculate_loan.py`.

use std::sync::Arc;

use core_domain::exception::CalculationError;
use core_domain::loan_calculator::{LoanCalculationParams, LoanCalculationResult};

use crate::ports::LoanCalculatorPort;

/// `CalculateLoan`: delegates to the loan calculator.
#[derive(Debug, Clone)]
pub struct CalculateLoan {
    loan_calculator: Arc<dyn LoanCalculatorPort>,
}

impl CalculateLoan {
    pub fn new(loan_calculator: Arc<dyn LoanCalculatorPort>) -> Self {
        CalculateLoan { loan_calculator }
    }

    pub fn execute(
        &self,
        params: &LoanCalculationParams,
    ) -> Result<LoanCalculationResult, CalculationError> {
        self.loan_calculator.calculate(params)
    }
}
