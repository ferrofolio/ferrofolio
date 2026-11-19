// SPDX-License-Identifier: AGPL-3.0-only

//! Domain errors from `finanze/domain/exception/exceptions.py`, ported as needed.

use crate::dezimal::ArithmeticError;

/// `MissingFieldsError`: required input is absent.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("Missing required fields: {}", .missing_fields.join(", "))]
pub struct MissingFieldsError {
    pub missing_fields: Vec<String>,
}

impl MissingFieldsError {
    pub fn new<I, S>(fields: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        MissingFieldsError {
            missing_fields: fields.into_iter().map(Into::into).collect(),
        }
    }
}

/// What the constructors of positions raise in Python's `__post_init__`.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum PositionError {
    #[error(transparent)]
    MissingFields(#[from] MissingFieldsError),
    /// Arithmetic that Python would abort with a `decimal` signal.
    #[error(transparent)]
    Arithmetic(#[from] ArithmeticError),
}

/// `CalculationInputError`: the input cannot produce a result.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{details}")]
pub struct CalculationInputError {
    pub details: String,
}

impl CalculationInputError {
    pub fn new(details: impl Into<String>) -> Self {
        CalculationInputError {
            details: details.into(),
        }
    }
}

/// Errors of the calculators.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum CalculationError {
    #[error(transparent)]
    MissingFields(#[from] MissingFieldsError),
    #[error(transparent)]
    Input(#[from] CalculationInputError),
    /// Arithmetic that Python would abort with a `decimal` signal.
    #[error(transparent)]
    Arithmetic(#[from] ArithmeticError),
    /// A date outside the supported calendar range.
    #[error("date out of range")]
    DateOutOfRange,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_fields_message_matches_python() {
        let error = MissingFieldsError::new(["start", "end"]);
        assert_eq!(error.to_string(), "Missing required fields: start, end");
    }
}
