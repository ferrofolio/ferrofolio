// SPDX-License-Identifier: AGPL-3.0-only

//! `Dezimal` against Python's `decimal` in the default context.
//!
//! The fixtures are recorded by `tests/parity/generate_decimal_fixtures.py`: seeded
//! random and hand-picked additions, subtractions, multiplications, divisions,
//! integer powers, roundings to cents and comparisons. Every result must have the
//! same value as Python's, and every trapped signal must be the same.

// Test helpers outside #[test] functions may panic on broken fixtures.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use core_domain::dezimal::{ArithmeticError, Dezimal};
use serde_json::Value;

const FIXTURES: &str = include_str!("fixtures/decimal_parity.json");

fn parse(text: &str) -> Dezimal {
    Dezimal::from_python_str(text).unwrap_or_else(|_| panic!("cannot parse {text}"))
}

/// The outcome as Python's `str()` names it: a value, or the signal's name.
fn outcome(case: &Value) -> Result<String, ArithmeticError> {
    let a = parse(case["a"].as_str().unwrap());
    let b = || case["b"].as_str().unwrap();
    let result = match case["op"].as_str().unwrap() {
        "add" => (a + parse(b())).value(),
        "sub" => (a - parse(b())).value(),
        "mul" => (a * parse(b())).value(),
        "div" => (a / parse(b())).value(),
        "pow" => a.powi(b().parse().unwrap()).value(),
        "cents" => a.round_cents(),
        "cmp" => {
            return Ok(match a.cmp(&parse(b())) {
                std::cmp::Ordering::Less => "-1",
                std::cmp::Ordering::Equal => "0",
                std::cmp::Ordering::Greater => "1",
            }
            .to_owned());
        }
        other => panic!("unknown operation {other}"),
    };
    result.map(|value| value.to_string())
}

/// A decimal text as sign, significant digits without trailing zeros and the power
/// of ten of the last digit; zero keeps its sign. Accepts Python's `str()` forms.
fn canonical(text: &str) -> Option<(bool, String, i64)> {
    let (negative, unsigned) = match text.strip_prefix('-') {
        Some(rest) => (true, rest),
        None => (false, text),
    };
    let (mantissa, exponent) = match unsigned.split_once(['e', 'E']) {
        Some((mantissa, exponent)) => (mantissa, exponent.parse::<i64>().ok()?),
        None => (unsigned, 0),
    };
    let (integral, fraction) = mantissa.split_once('.').unwrap_or((mantissa, ""));
    let digits = format!("{integral}{fraction}");
    if digits.is_empty() || !digits.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    let significant = digits.trim_start_matches('0');
    if significant.is_empty() {
        return Some((negative, String::new(), 0));
    }
    let trimmed = significant.trim_end_matches('0');
    let exponent = exponent - i64::try_from(fraction.len()).ok()?
        + i64::try_from(significant.len() - trimmed.len()).ok()?;
    Some((negative, trimmed.to_owned(), exponent))
}

fn matches(case: &Value, actual: &Result<String, ArithmeticError>) -> bool {
    let expected = case["result"].as_str().unwrap();
    match (expected, actual) {
        ("DivisionByZero", Err(ArithmeticError::DivisionByZero))
        | ("InvalidOperation", Err(ArithmeticError::InvalidOperation))
        | ("Overflow", Err(ArithmeticError::Overflow)) => true,
        // Python returns Infinity for 0 ** -n without trapping; there is no
        // infinite amount here (docs/rust-backend.md, known deviations).
        ("Infinity", Err(ArithmeticError::DivisionByZero)) => true,
        (_, Ok(value)) if case["op"] == "cmp" => value == expected,
        // Rounding to cents fixes the scale as well: Python's `30.40`, not `30.4`.
        (_, Ok(value)) if case["op"] == "cents" => value == expected,
        // The same digits, power of ten and sign, also for zero, which the API
        // reports as -0.0; `1.5` and `1.50` are the same result. The texts are
        // compared without `Dezimal`'s own comparison.
        (_, Ok(value)) => canonical(value).is_some() && canonical(value) == canonical(expected),
        _ => false,
    }
}

#[test]
fn dezimal_computes_like_python_decimal() {
    let cases: Vec<Value> = serde_json::from_str(FIXTURES).unwrap();
    assert!(cases.len() > 5000, "the fixtures look truncated");
    let failures: Vec<String> = cases
        .iter()
        .filter_map(|case| {
            let actual = outcome(case);
            (!matches(case, &actual)).then(|| format!("{case}\n  got {actual:?}"))
        })
        .collect();
    assert!(
        failures.is_empty(),
        "{} of {} operations differ from Python:\n{}",
        failures.len(),
        cases.len(),
        failures
            .iter()
            .take(200)
            .cloned()
            .collect::<Vec<_>>()
            .join("\n")
    );
}
