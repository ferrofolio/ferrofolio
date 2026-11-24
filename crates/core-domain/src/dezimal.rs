// SPDX-License-Identifier: AGPL-3.0-only

//! Exact decimal numbers for money, the counterpart of `finanze/domain/dezimal.py`.
//!
//! Python's `Dezimal` wraps `decimal.Decimal` and computes in the default context:
//! every result of `+ - * /` is the exact result rounded to 28 significant digits
//! with `ROUND_HALF_EVEN`, and integer powers follow libmpdec. This type reproduces
//! that arithmetic exactly, which a fixed-scale decimal such as rust_decimal cannot:
//! `150 * (0.01 / 12)` is 0.125 here and in Python, so it rounds to 0.13 cents.
//!
//! Arithmetic never panics: every operator returns a [`Checked`] value that carries
//! the first error through a whole expression, the way Python's traps raise. The
//! result is unwrapped once with [`Checked::value`]:
//!
//! ```
//! use core_domain::dezimal::Dezimal;
//!
//! let rate = Dezimal::from_python_str("0.0025").unwrap();
//! let denominator = (Dezimal::ONE - (Dezimal::ONE + rate).powi(-360)).value().unwrap();
//! assert!(denominator > Dezimal::ZERO);
//! ```

use std::cmp::Ordering;
use std::fmt;
use std::hash::{Hash, Hasher};
use std::ops::{Add, Div, Mul, Neg, Sub};
use std::sync::OnceLock;

use bnum::cast::As;
use bnum::types::U256;

use crate::unicode_digits::to_ascii_digits;

/// Significant digits of Python's default decimal context.
const PRECISION: u32 = 28;
/// Most significant digits a stored value keeps; they fit a `u128`.
const MAX_DIGITS: u32 = 38;
/// Largest adjusted exponent of Python's default context (`Emax`).
const EMAX: i64 = 999_999;
/// Smallest adjusted exponent of Python's default context (`Emin`).
const EMIN: i64 = -999_999;
/// Smallest exponent of a subnormal value in the context (`Etiny`): Python keeps
/// values below 10^Emin with fewer digits down to this exponent.
const ETINY: i64 = EMIN - PRECISION as i64 + 1;

/// A signal of Python's decimal context that is trapped, so that it raises.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum ArithmeticError {
    /// `decimal.Overflow`: a result beyond the largest exponent.
    #[error("decimal overflow")]
    Overflow,
    /// `decimal.DivisionByZero`.
    #[error("division by zero")]
    DivisionByZero,
    /// `decimal.InvalidOperation`, for example rounding to cents a value with
    /// more than 28 digits, or `0 / 0`.
    #[error("invalid decimal operation")]
    InvalidOperation,
}

/// A string that is not a finite decimal number this type can hold.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("invalid decimal value")]
pub struct ParseDezimalError;

/// An exact decimal number: `(-1)^negative × coefficient × 10^exponent`.
#[derive(Clone, Copy, Debug)]
pub struct Dezimal {
    negative: bool,
    coefficient: u128,
    exponent: i32,
}

/// How a result is rounded to its last kept digit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Rounding {
    /// `ROUND_HALF_EVEN`, the context's rounding for arithmetic.
    HalfEven,
    /// `ROUND_HALF_UP`, which the calculators use for cents.
    HalfUp,
}

impl Dezimal {
    pub const ZERO: Dezimal = Dezimal::integer(0);
    pub const ONE: Dezimal = Dezimal::integer(1);
    pub const TWO: Dezimal = Dezimal::integer(2);
    /// 0.01, one cent.
    pub const CENT: Dezimal = Dezimal {
        negative: false,
        coefficient: 1,
        exponent: -2,
    };

    const fn integer(value: u128) -> Self {
        Dezimal {
            negative: false,
            coefficient: value,
            exponent: 0,
        }
    }

    /// `value × 10^-scale`, exactly: `from_scaled(19, 2)` is 0.19. A `const`
    /// constructor for the domain's fixed rates and factors.
    pub const fn from_scaled(value: i64, scale: u16) -> Self {
        Dezimal {
            negative: value < 0,
            coefficient: value.unsigned_abs() as u128,
            exponent: -(scale as i32),
        }
    }

    /// Parses a string the way Python's `Decimal(str)` does for finite numbers:
    /// surrounding whitespace, a sign, underscores, digits of any script and an
    /// exponent are accepted. `NaN` and `Infinity` are rejected, because money is
    /// always finite, and so are numbers with more than 38 significant digits.
    pub fn from_python_str(value: &str) -> Result<Self, ParseDezimalError> {
        parse_python_decimal(value)
    }

    /// The power of ten of the coefficient, needed to bound a plain-text expansion.
    pub const fn exponent(self) -> i32 {
        self.exponent
    }

    pub fn is_zero(self) -> bool {
        self.coefficient == 0
    }

    pub fn is_sign_negative(self) -> bool {
        self.negative && !self.is_zero()
    }

    pub fn abs(self) -> Self {
        Dezimal {
            negative: false,
            ..self
        }
    }

    /// The larger of two values, `max(a, b)` in Python.
    pub fn max(self, other: Self) -> Self {
        if other > self { other } else { self }
    }

    /// Raises to an integer power like Python's `Decimal.__pow__`; negative
    /// exponents divide.
    pub fn powi(self, exponent: i64) -> Checked {
        Checked::from(self).powi(exponent)
    }

    /// Rounds to `places` decimal places with Python's `ROUND_HALF_UP`, which rounds
    /// ties away from zero. A value with fewer places is returned unchanged.
    pub fn round_half_up(self, places: u32) -> Self {
        self.round_places(places, Rounding::HalfUp)
    }

    /// Rounds to `places` decimal places; a value with fewer places is returned
    /// unchanged.
    fn round_places(self, places: u32, rounding: Rounding) -> Self {
        let target = -i64::from(places);
        if i64::from(self.exponent) >= target {
            return self;
        }
        let drop = u32::try_from(target - i64::from(self.exponent)).unwrap_or(u32::MAX);
        let (quotient, _) = round_off(self.coefficient.as_::<U256>(), drop, false, rounding);
        Dezimal {
            negative: self.negative,
            coefficient: quotient.try_into().unwrap_or(0),
            exponent: i32::try_from(target).unwrap_or(i32::MIN),
        }
    }

    /// `value.quantize(Decimal("0.01"), rounding=ROUND_HALF_UP)` in Python.
    ///
    /// Fails like Python does when the result has more than 28 digits.
    pub fn round_cents(self) -> Result<Self, ArithmeticError> {
        self.quantize(2, Rounding::HalfUp)
    }

    /// `round(value, places)` in Python: the value quantized to `places` decimal
    /// places with the context's `ROUND_HALF_EVEN`, which rounds ties to the even
    /// digit, so `round(0.125, 2)` is 0.12. The result has exactly `places` places.
    /// `places` is nonnegative: Python also rounds to tens with a negative number
    /// of places, which this port does not support. A zero is quantized without a
    /// scaling factor, so `round(0, 39)` succeeds although 10^39 does not fit the
    /// coefficient.
    ///
    /// Fails like Python does when a nonzero result has more than 28 digits.
    pub fn round(self, places: u32) -> Result<Self, ArithmeticError> {
        self.quantize(places, Rounding::HalfEven)
    }

    /// `value.quantize(Decimal(10) ** -places, rounding)`.
    fn quantize(self, places: u32, rounding: Rounding) -> Result<Self, ArithmeticError> {
        let rounded = self.round_places(places, rounding);
        let places = i64::from(places);
        // The quantized coefficient counts every digit down to the last place, also
        // those a large value only has as trailing zeros (1E+26 has 29 to the cents).
        if !rounded.is_zero() {
            let length = i64::from(digits(rounded.coefficient.as_::<U256>()));
            if i64::from(rounded.exponent) + places + length > i64::from(PRECISION) {
                return Err(ArithmeticError::InvalidOperation);
            }
        }
        // The result has exactly `places` places, as in Python: 1.5 becomes 1.50
        // in cents. The check above keeps the scaled coefficient within 28 digits.
        // A zero needs no scaling factor: Python allows `0E-39`, although 10^39
        // does not fit the coefficient. Keep the sign of a negative zero.
        let coefficient = if rounded.is_zero() {
            0
        } else {
            let shift = u32::try_from(i64::from(rounded.exponent) + places).unwrap_or(0);
            10_u128
                .checked_pow(shift)
                .and_then(|scale| rounded.coefficient.checked_mul(scale))
                .ok_or(ArithmeticError::InvalidOperation)?
        };
        Ok(Dezimal {
            coefficient,
            exponent: i32::try_from(-places).map_err(|_| ArithmeticError::InvalidOperation)?,
            ..rounded
        })
    }

    /// `f"{value:.{places}f}"` in Python: the value in plain notation with exactly
    /// `places` decimal places, rounded with `ROUND_HALF_EVEN` and, unlike
    /// [`round`](Self::round), without a limit on the digits. The sign of a negative
    /// value that rounds to zero is kept: -0.001 gives `-0.00`.
    pub fn to_fixed(self, places: u32) -> String {
        let rounded = self.round_places(places, Rounding::HalfEven);
        let mut text = rounded.to_string();
        // Rounding left at most `places` places; pad to exactly that many.
        let written = u32::try_from(-i64::from(rounded.exponent)).unwrap_or(0);
        if places > 0 && written == 0 {
            text.push('.');
        }
        for _ in written..places {
            text.push('0');
        }
        text
    }

    /// The nearest binary float, like Python's `float(Dezimal)`.
    ///
    /// Only for serialising API responses, which carry amounts as JSON numbers.
    pub fn to_f64_for_api(self) -> f64 {
        let sign = if self.negative { "-" } else { "" };
        // Parsing the decimal text rounds correctly, as Python does.
        format!("{sign}{}e{}", self.coefficient, self.exponent)
            .parse::<f64>()
            .unwrap_or(f64::NAN)
    }

    fn from_parts(
        negative: bool,
        coefficient: u128,
        exponent: i64,
    ) -> Result<Self, ArithmeticError> {
        if coefficient == 0 {
            return Ok(Dezimal {
                negative,
                coefficient: 0,
                exponent: 0,
            });
        }
        let adjusted = exponent + i64::from(digits(coefficient.as_::<U256>())) - 1;
        if adjusted > EMAX {
            return Err(ArithmeticError::Overflow);
        }
        let exponent = i32::try_from(exponent).map_err(|_| ArithmeticError::Overflow)?;
        Ok(Dezimal {
            negative,
            coefficient,
            exponent,
        })
    }
}

impl From<i64> for Dezimal {
    fn from(value: i64) -> Self {
        Dezimal {
            negative: value < 0,
            coefficient: u128::from(value.unsigned_abs()),
            exponent: 0,
        }
    }
}

impl Default for Dezimal {
    fn default() -> Self {
        Dezimal::ZERO
    }
}

impl fmt::Display for Dezimal {
    /// Plain notation like Python's `f"{value:f}"`: the same value and sign, also
    /// for zero. Parsed values do not keep trailing zeros (`1.50` prints `1.5`), so
    /// the number of places can differ; results of `round_cents` have two.
    ///
    /// The output has as many digits as the exponent demands, up to about two
    /// billion for a parsed `1e2147483647`, as in Python. Nothing in the server
    /// formats request values with it; a caller that would has to bound them first.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let sign = if self.negative { "-" } else { "" };
        let digits = self.coefficient.to_string();
        let exponent = i64::from(self.exponent);
        if exponent >= 0 {
            write!(f, "{sign}{digits}")?;
            return write_zeros(f, exponent.unsigned_abs());
        }
        let places = exponent.unsigned_abs();
        let length = u64::try_from(digits.len()).unwrap_or(u64::MAX);
        if length > places {
            let split = usize::try_from(length - places).unwrap_or(0);
            let (integral, fraction) = digits.split_at(split);
            write!(f, "{sign}{integral}.{fraction}")
        } else {
            write!(f, "{sign}0.")?;
            write_zeros(f, places - length)?;
            f.write_str(&digits)
        }
    }
}

/// Writes zeros in chunks, so that a destination that fails or stops accepting
/// output ends the write early. A `String` destination still grows to the full size.
fn write_zeros(f: &mut fmt::Formatter<'_>, count: u64) -> fmt::Result {
    const ZEROS: &str = "0000000000000000000000000000000000000000000000000000000000000000";
    let mut remaining = count;
    while remaining > 0 {
        let chunk = remaining.min(64);
        f.write_str(
            ZEROS
                .get(..usize::try_from(chunk).unwrap_or(64))
                .unwrap_or(ZEROS),
        )?;
        remaining -= chunk;
    }
    Ok(())
}

impl PartialEq for Dezimal {
    fn eq(&self, other: &Self) -> bool {
        self.cmp(other) == Ordering::Equal
    }
}

impl Eq for Dezimal {}

impl PartialOrd for Dezimal {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Dezimal {
    /// Compares values, like Python's `compare`: 1.50 equals 1.5 and -0 equals 0.
    fn cmp(&self, other: &Self) -> Ordering {
        match (self.is_zero(), other.is_zero()) {
            (true, true) => return Ordering::Equal,
            (true, false) => {
                return if other.negative {
                    Ordering::Greater
                } else {
                    Ordering::Less
                };
            }
            (false, true) => {
                return if self.negative {
                    Ordering::Less
                } else {
                    Ordering::Greater
                };
            }
            (false, false) => {}
        }
        match (self.negative, other.negative) {
            (false, true) => Ordering::Greater,
            (true, false) => Ordering::Less,
            (false, false) => compare_magnitudes(*self, *other),
            (true, true) => compare_magnitudes(*other, *self),
        }
    }
}

impl Hash for Dezimal {
    /// Hashes the value, consistent with equality: trailing zeros are removed.
    fn hash<H: Hasher>(&self, state: &mut H) {
        if self.is_zero() {
            return 0u8.hash(state);
        }
        let mut coefficient = self.coefficient;
        let mut exponent = self.exponent;
        while coefficient.is_multiple_of(10) {
            coefficient /= 10;
            exponent = exponent.saturating_add(1);
        }
        (self.negative, coefficient, exponent).hash(state);
    }
}

impl Neg for Dezimal {
    type Output = Checked;

    /// `-value` in Python's context, as `Decimal.__neg__` computes it: rounded to 28
    /// significant digits, and the negation of a zero is a positive zero.
    fn neg(self) -> Checked {
        Checked(add(Dezimal::ZERO, flip_sign(self), PRECISION))
    }
}

/// The same digits with the other sign, without rounding.
fn flip_sign(value: Dezimal) -> Dezimal {
    Dezimal {
        negative: !value.negative,
        ..value
    }
}

impl PartialEq<i64> for Dezimal {
    fn eq(&self, other: &i64) -> bool {
        *self == Dezimal::from(*other)
    }
}

impl PartialOrd<i64> for Dezimal {
    fn partial_cmp(&self, other: &i64) -> Option<Ordering> {
        Some(self.cmp(&Dezimal::from(*other)))
    }
}

/// The result of decimal arithmetic that may have raised a decimal signal.
///
/// Operators on [`Checked`] keep the first error, so a whole formula is written
/// with plain operators and checked once with [`Checked::value`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[must_use = "a checked result has to be unwrapped with `value()`"]
pub struct Checked(Result<Dezimal, ArithmeticError>);

impl Checked {
    /// The computed value, or the first error that occurred.
    pub fn value(self) -> Result<Dezimal, ArithmeticError> {
        self.0
    }

    /// Raises to an integer power; negative exponents divide.
    pub fn powi(self, exponent: i64) -> Checked {
        Checked(self.0.and_then(|base| power(base, exponent)))
    }

    fn combine(
        self,
        rhs: Checked,
        operation: impl FnOnce(Dezimal, Dezimal) -> Result<Dezimal, ArithmeticError>,
    ) -> Checked {
        Checked(
            self.0
                .and_then(|lhs| rhs.0.and_then(|rhs| operation(lhs, rhs))),
        )
    }
}

impl From<Dezimal> for Checked {
    fn from(value: Dezimal) -> Self {
        Checked(Ok(value))
    }
}

fn context_add(lhs: Dezimal, rhs: Dezimal) -> Result<Dezimal, ArithmeticError> {
    add(lhs, rhs, PRECISION)
}

fn context_sub(lhs: Dezimal, rhs: Dezimal) -> Result<Dezimal, ArithmeticError> {
    // The sign flips exactly; only the sum is rounded, once.
    add(lhs, flip_sign(rhs), PRECISION)
}

fn context_mul(lhs: Dezimal, rhs: Dezimal) -> Result<Dezimal, ArithmeticError> {
    multiply(lhs, rhs, PRECISION)
}

fn context_div(lhs: Dezimal, rhs: Dezimal) -> Result<Dezimal, ArithmeticError> {
    divide(lhs, rhs, PRECISION)
}

macro_rules! checked_operator {
    ($trait:ident, $method:ident, $operation:path) => {
        impl $trait<Checked> for Checked {
            type Output = Checked;

            fn $method(self, rhs: Checked) -> Checked {
                self.combine(rhs, $operation)
            }
        }

        impl $trait<Dezimal> for Checked {
            type Output = Checked;

            fn $method(self, rhs: Dezimal) -> Checked {
                self.combine(Checked::from(rhs), $operation)
            }
        }

        impl $trait<Checked> for Dezimal {
            type Output = Checked;

            fn $method(self, rhs: Checked) -> Checked {
                Checked::from(self).combine(rhs, $operation)
            }
        }

        impl $trait<Dezimal> for Dezimal {
            type Output = Checked;

            fn $method(self, rhs: Dezimal) -> Checked {
                Checked::from(self).combine(Checked::from(rhs), $operation)
            }
        }
    };
}

checked_operator!(Add, add, context_add);
checked_operator!(Sub, sub, context_sub);
checked_operator!(Mul, mul, context_mul);
checked_operator!(Div, div, context_div);

fn u256(value: u128) -> U256 {
    value.as_::<U256>()
}

/// 10^exponent as a `u128`, if it fits (up to 10^38).
fn ten_to_small(exponent: u32) -> Option<u128> {
    10u128.checked_pow(exponent)
}

/// 10^0 to 10^77, every power of ten a `U256` holds.
fn powers_u256() -> &'static [U256; 78] {
    static POWERS: OnceLock<[U256; 78]> = OnceLock::new();
    POWERS.get_or_init(|| {
        let mut powers = [u256(1); 78];
        for index in 1..78 {
            if let (Some(previous), Some(slot)) =
                (powers.get(index - 1).copied(), powers.get_mut(index))
            {
                *slot = previous * u256(10);
            }
        }
        powers
    })
}

/// 10^exponent; callers keep it below 10^78, the range of `U256`.
fn ten_to(exponent: u32) -> U256 {
    usize::try_from(exponent)
        .ok()
        .and_then(|index| powers_u256().get(index).copied())
        .unwrap_or(U256::MAX)
}

/// Number of decimal digits; 0 has none.
fn digits(value: U256) -> u32 {
    match u128::try_from(value) {
        Ok(small) => digits_u128(small),
        Err(_) => {
            let powers = powers_u256();
            u32::try_from(powers.partition_point(|power| *power <= value)).unwrap_or(78)
        }
    }
}

fn digits_u128(value: u128) -> u32 {
    value.checked_ilog10().map_or(0, |log| log + 1)
}

/// Divides by 10^drop and rounds. `sticky` says that the exact value lies a little
/// above `value`, below its last digit. Returns the quotient and whether it gained
/// a digit by rounding up to a power of ten.
fn round_off(value: U256, drop: u32, sticky: bool, rounding: Rounding) -> (U256, bool) {
    if drop == 0 {
        return (value, false);
    }
    if drop > 77 {
        // Everything is dropped and the remainder is below half of the unit.
        return (U256::MIN, false);
    }
    let divisor = ten_to(drop);
    let quotient = value / divisor;
    let remainder = value % divisor;
    let half = divisor / u256(2);
    let round_up = match remainder.cmp(&half) {
        Ordering::Greater => true,
        Ordering::Less => false,
        Ordering::Equal => match rounding {
            Rounding::HalfUp => true,
            Rounding::HalfEven => sticky || quotient % u256(2) == u256(1),
        },
    };
    if round_up {
        let rounded = quotient + u256(1);
        let gained = digits(rounded) > digits(quotient) && quotient != U256::MIN;
        (rounded, gained)
    } else {
        (quotient, false)
    }
}

/// Rounds the exact value `coefficient × 10^exponent` (plus less than one unit of
/// its last digit when `sticky`) to `precision` significant digits.
fn round_to_precision(
    negative: bool,
    coefficient: U256,
    exponent: i64,
    sticky: bool,
    precision: u32,
) -> Result<Dezimal, ArithmeticError> {
    if let Ok(small) = u128::try_from(coefficient) {
        return round_small(negative, small, exponent, sticky, precision);
    }
    round_wide(negative, coefficient, exponent, sticky, precision)
}

/// `round_to_precision` in 256 bits.
fn round_wide(
    negative: bool,
    coefficient: U256,
    exponent: i64,
    sticky: bool,
    precision: u32,
) -> Result<Dezimal, ArithmeticError> {
    let length = digits(coefficient);
    let drop = digits_to_drop(length, exponent, precision);
    let (mut rounded, gained) = round_off(coefficient, drop, sticky, Rounding::HalfEven);
    let mut exponent = exponent + i64::from(drop);
    if gained && digits(rounded) > precision {
        // 9.99… rounded up to 10.0…: one digit too many.
        rounded /= u256(10);
        exponent += 1;
    }
    let coefficient = u128::try_from(rounded).map_err(|_| ArithmeticError::Overflow)?;
    Dezimal::from_parts(negative, coefficient, exponent)
}

/// Digits to drop from a coefficient of `length` digits: those beyond the precision,
/// or more for a subnormal value, whose exponent may not go below `Etiny`.
fn digits_to_drop(length: u32, exponent: i64, precision: u32) -> u32 {
    let for_precision = i64::from(length) - i64::from(precision);
    // Etiny depends on the precision: the working precision of a power reaches
    // further down than the context's 28 digits.
    let etiny = EMIN - i64::from(precision) + 1;
    let for_subnormal = etiny - exponent;
    u32::try_from(for_precision.max(for_subnormal).max(0)).unwrap_or(u32::MAX)
}

/// `round_to_precision` for a coefficient that fits a `u128`, the common case.
fn round_small(
    negative: bool,
    coefficient: u128,
    exponent: i64,
    sticky: bool,
    precision: u32,
) -> Result<Dezimal, ArithmeticError> {
    let length = digits_u128(coefficient);
    if length <= precision && !sticky && exponent >= ETINY {
        return Dezimal::from_parts(negative, coefficient, exponent);
    }
    let drop = digits_to_drop(length, exponent, precision);
    let Some(divisor) = ten_to_small(drop) else {
        return round_wide(negative, u256(coefficient), exponent, sticky, precision);
    };
    let mut quotient = coefficient / divisor;
    let remainder = coefficient % divisor;
    let half = divisor / 2;
    let round_up = drop > 0
        && match remainder.cmp(&half) {
            Ordering::Greater => true,
            Ordering::Less => false,
            Ordering::Equal => sticky || quotient % 2 == 1,
        };
    let mut exponent = exponent + i64::from(drop);
    if round_up {
        quotient += 1;
        if digits_u128(quotient) > precision {
            quotient /= 10;
            exponent += 1;
        }
    }
    Dezimal::from_parts(negative, quotient, exponent)
}

/// `lhs + rhs` rounded to `precision` digits.
fn add(lhs: Dezimal, rhs: Dezimal, precision: u32) -> Result<Dezimal, ArithmeticError> {
    if rhs.is_zero() || lhs.is_zero() {
        let (value, other) = if rhs.is_zero() {
            (lhs, rhs)
        } else {
            (rhs, lhs)
        };
        let negative = if value.is_zero() {
            // The sum of two zeros is negative only if both are.
            value.negative && other.negative
        } else {
            value.negative
        };
        return round_to_precision(
            negative,
            value.coefficient.as_::<U256>(),
            i64::from(value.exponent),
            false,
            precision,
        );
    }
    let (high, low) = if lhs.exponent >= rhs.exponent {
        (lhs, rhs)
    } else {
        (rhs, lhs)
    };
    // Up to 2^32 for exponents at both ends of the i32 range, so it is a u64.
    let width = (i64::from(high.exponent) - i64::from(low.exponent)).unsigned_abs();
    let high_digits = digits(high.coefficient.as_::<U256>());
    let aligned_digits = u64::from(high_digits) + width;
    // Below 76 digits the shift is at most 75.
    let shift = u32::try_from(width).unwrap_or(u32::MAX);

    if aligned_digits <= 38 {
        // Exact in 128 bits: both coefficients on the scale of the smaller exponent.
        let scale = ten_to_small(shift);
        if let Some(high_scaled) = scale.and_then(|scale| high.coefficient.checked_mul(scale)) {
            let low_scaled = low.coefficient;
            let exponent = i64::from(low.exponent);
            if high.negative == low.negative
                && let Some(sum) = high_scaled.checked_add(low_scaled)
            {
                return round_small(high.negative, sum, exponent, false, precision);
            }
            if high.negative != low.negative {
                return match high_scaled.cmp(&low_scaled) {
                    Ordering::Equal => Ok(Dezimal::ZERO),
                    Ordering::Greater => round_small(
                        high.negative,
                        high_scaled - low_scaled,
                        exponent,
                        false,
                        precision,
                    ),
                    Ordering::Less => round_small(
                        low.negative,
                        low_scaled - high_scaled,
                        exponent,
                        false,
                        precision,
                    ),
                };
            }
        }
    }
    if aligned_digits <= 76 {
        // Exact: both coefficients on the scale of the smaller exponent.
        let high_scaled = high.coefficient.as_::<U256>() * ten_to(shift);
        let low_scaled = low.coefficient.as_::<U256>();
        let exponent = i64::from(low.exponent);
        if high.negative == low.negative {
            return round_to_precision(
                high.negative,
                high_scaled + low_scaled,
                exponent,
                false,
                precision,
            );
        }
        return match high_scaled.cmp(&low_scaled) {
            Ordering::Equal => Ok(Dezimal::ZERO),
            Ordering::Greater => round_to_precision(
                high.negative,
                high_scaled - low_scaled,
                exponent,
                false,
                precision,
            ),
            Ordering::Less => round_to_precision(
                low.negative,
                low_scaled - high_scaled,
                exponent,
                false,
                precision,
            ),
        };
    }

    // The smaller operand lies below the last of 39 digits of the larger one, so
    // only its sign matters for the rounding: it is carried as a sticky part.
    let scale = 39 - high_digits;
    let high_scaled = high.coefficient.as_::<U256>() * ten_to(scale);
    let exponent = i64::from(high.exponent) - i64::from(scale);
    if high.negative == low.negative {
        round_to_precision(high.negative, high_scaled, exponent, true, precision)
    } else {
        round_to_precision(
            high.negative,
            high_scaled - u256(1),
            exponent,
            true,
            precision,
        )
    }
}

/// `lhs × rhs` rounded to `precision` digits.
fn multiply(lhs: Dezimal, rhs: Dezimal, precision: u32) -> Result<Dezimal, ArithmeticError> {
    let negative = lhs.negative != rhs.negative;
    let exponent = i64::from(lhs.exponent) + i64::from(rhs.exponent);
    if let Some(product) = lhs.coefficient.checked_mul(rhs.coefficient) {
        return round_small(negative, product, exponent, false, precision);
    }
    let product = u256(lhs.coefficient) * u256(rhs.coefficient);
    round_to_precision(negative, product, exponent, false, precision)
}

/// `lhs ÷ rhs` correctly rounded to `precision` digits.
fn divide(lhs: Dezimal, rhs: Dezimal, precision: u32) -> Result<Dezimal, ArithmeticError> {
    let negative = lhs.negative != rhs.negative;
    if rhs.is_zero() {
        return Err(if lhs.is_zero() {
            ArithmeticError::InvalidOperation
        } else {
            ArithmeticError::DivisionByZero
        });
    }
    if lhs.is_zero() {
        return Ok(Dezimal {
            negative,
            coefficient: 0,
            exponent: 0,
        });
    }
    // Scale the dividend so that the quotient has at least two digits more than
    // the precision; the remainder then decides the rounding exactly.
    let shift =
        (precision + 2 + digits_u128(rhs.coefficient)).saturating_sub(digits_u128(lhs.coefficient));
    let exponent = i64::from(lhs.exponent) - i64::from(rhs.exponent) - i64::from(shift);
    let scaled_small = ten_to_small(shift).and_then(|scale| lhs.coefficient.checked_mul(scale));
    if let Some(scaled) = scaled_small {
        let quotient = scaled / rhs.coefficient;
        let sticky = scaled % rhs.coefficient != 0;
        return round_small(negative, quotient, exponent, sticky, precision);
    }
    let dividend = lhs.coefficient.as_::<U256>();
    let divisor = rhs.coefficient.as_::<U256>();
    let scaled = dividend * ten_to(shift);
    let quotient = scaled / divisor;
    let sticky = scaled % divisor != U256::MIN;
    round_to_precision(negative, quotient, exponent, sticky, precision)
}

/// `base ** exponent` for an integer exponent, computed the way libmpdec does
/// (`_mpd_qpow_int`): left-to-right binary exponentiation in a working precision of
/// 28 + digits(exponent) + 2 digits, one more and the reciprocal of the base for a
/// negative exponent, then rounded to 28 digits.
fn power(base: Dezimal, exponent: i64) -> Result<Dezimal, ArithmeticError> {
    let magnitude = exponent.unsigned_abs();
    // An odd exponent keeps the sign of the base, also of a zero: (-0) ** 3 is -0.
    let negative = base.negative && magnitude % 2 == 1;
    if base.is_zero() {
        return match exponent.cmp(&0) {
            Ordering::Less => Err(ArithmeticError::DivisionByZero),
            Ordering::Equal => Err(ArithmeticError::InvalidOperation),
            Ordering::Greater => Ok(Dezimal {
                negative,
                ..Dezimal::ZERO
            }),
        };
    }
    if exponent == 0 {
        return Ok(Dezimal::ONE);
    }
    if base.abs() == Dezimal::ONE {
        // Exact for every exponent, as in Python.
        return Ok(Dezimal {
            negative,
            ..Dezimal::ONE
        });
    }
    // At most 28 + 19 + 3 = 50 digits, which the 512-bit intermediates hold.
    let exponent_digits = magnitude.checked_ilog10().map_or(1, |log| log + 1);
    let working = PRECISION + exponent_digits + 2 + u32::from(exponent < 0);
    let factor = if exponent < 0 {
        wide::reciprocal(base, working)?
    } else {
        wide::Wide::of(base)
    };

    let mut result = factor;
    let highest_bit = 63 - magnitude.leading_zeros();
    for bit in (0..highest_bit).rev() {
        // A result that rounded to zero stays zero; squaring it further would only
        // keep doubling its exponent.
        if result.coefficient == bnum::types::U512::MIN {
            break;
        }
        result = wide::multiply(result, result, working)?;
        if magnitude & (1 << bit) != 0 {
            result = wide::multiply(result, factor, working)?;
        }
    }
    let rounded = wide::round(result.coefficient, result.exponent, false, PRECISION)?;
    let coefficient = u128::try_from(rounded.coefficient).map_err(|_| ArithmeticError::Overflow)?;
    Dezimal::from_parts(negative, coefficient, rounded.exponent)
}

/// Intermediate results of powers, which keep more digits than a stored value.
mod wide {
    use std::cmp::Ordering;
    use std::sync::OnceLock;

    use bnum::cast::As;
    use bnum::types::U512;

    use super::{ArithmeticError, Dezimal, EMAX, digits_to_drop};

    /// `coefficient × 10^exponent`, non-negative.
    #[derive(Debug, Clone, Copy)]
    pub(super) struct Wide {
        pub(super) coefficient: U512,
        pub(super) exponent: i64,
    }

    impl Wide {
        /// The magnitude of a stored value, unrounded.
        pub(super) fn of(value: Dezimal) -> Self {
            Wide {
                coefficient: value.coefficient.as_::<U512>(),
                exponent: i64::from(value.exponent),
            }
        }
    }

    fn u512(value: u128) -> U512 {
        value.as_::<U512>()
    }

    /// 10^0 to 10^154, every power of ten a `U512` holds.
    fn powers() -> &'static [U512; 155] {
        static POWERS: OnceLock<[U512; 155]> = OnceLock::new();
        POWERS.get_or_init(|| {
            let mut powers = [u512(1); 155];
            for index in 1..155 {
                if let (Some(previous), Some(slot)) =
                    (powers.get(index - 1).copied(), powers.get_mut(index))
                {
                    *slot = previous * u512(10);
                }
            }
            powers
        })
    }

    fn ten_to(exponent: u32) -> Option<U512> {
        usize::try_from(exponent)
            .ok()
            .and_then(|index| powers().get(index).copied())
    }

    fn digits(value: U512) -> u32 {
        u32::try_from(powers().partition_point(|power| *power <= value)).unwrap_or(155)
    }

    /// Rounds `coefficient × 10^exponent`, plus less than one unit of its last
    /// digit when `sticky`, to `precision` digits with `ROUND_HALF_EVEN`.
    pub(super) fn round(
        coefficient: U512,
        exponent: i64,
        sticky: bool,
        precision: u32,
    ) -> Result<Wide, ArithmeticError> {
        let zero = U512::MIN;
        if coefficient == zero {
            return Ok(Wide {
                coefficient,
                exponent,
            });
        }
        let length = digits(coefficient);
        let drop = digits_to_drop(length, exponent, precision);
        let (mut quotient, mut exponent) = match ten_to(drop) {
            _ if drop == 0 => (coefficient, exponent),
            // Everything goes, and the remainder is below half of the unit.
            None => (zero, exponent + i64::from(drop)),
            Some(divisor) => {
                let quotient = coefficient / divisor;
                let remainder = coefficient % divisor;
                let half = divisor / u512(2);
                let round_up = match remainder.cmp(&half) {
                    Ordering::Greater => true,
                    Ordering::Less => false,
                    Ordering::Equal => sticky || quotient % u512(2) == u512(1),
                };
                let quotient = if round_up {
                    quotient + u512(1)
                } else {
                    quotient
                };
                (quotient, exponent + i64::from(drop))
            }
        };
        if digits(quotient) > precision {
            // 9.99… rounded up to 10.0…: one digit too many.
            quotient /= u512(10);
            exponent += 1;
        }
        if quotient != zero && exponent + i64::from(digits(quotient)) - 1 > EMAX {
            return Err(ArithmeticError::Overflow);
        }
        Ok(Wide {
            coefficient: quotient,
            exponent,
        })
    }

    /// `lhs × rhs` rounded to `precision` digits; both have at most 50 digits.
    pub(super) fn multiply(lhs: Wide, rhs: Wide, precision: u32) -> Result<Wide, ArithmeticError> {
        round(
            lhs.coefficient * rhs.coefficient,
            lhs.exponent.saturating_add(rhs.exponent),
            false,
            precision,
        )
    }

    /// `1 ÷ |base|` correctly rounded to `precision` digits.
    pub(super) fn reciprocal(base: Dezimal, precision: u32) -> Result<Wide, ArithmeticError> {
        let divisor = u512(base.coefficient);
        if divisor == U512::MIN {
            return Err(ArithmeticError::DivisionByZero);
        }
        // A quotient with two digits more than the precision decides the rounding.
        let shift = precision + 2 + digits(divisor);
        let dividend = ten_to(shift).ok_or(ArithmeticError::Overflow)?;
        let quotient = dividend / divisor;
        let sticky = dividend % divisor != U512::MIN;
        round(
            quotient,
            -i64::from(base.exponent) - i64::from(shift),
            sticky,
            precision,
        )
    }
}

/// Compares the magnitudes of two non-zero values.
fn compare_magnitudes(lhs: Dezimal, rhs: Dezimal) -> Ordering {
    let lhs_coefficient = lhs.coefficient.as_::<U256>();
    let rhs_coefficient = rhs.coefficient.as_::<U256>();
    let lhs_adjusted = i64::from(lhs.exponent) + i64::from(digits(lhs_coefficient));
    let rhs_adjusted = i64::from(rhs.exponent) + i64::from(digits(rhs_coefficient));
    if lhs_adjusted != rhs_adjusted {
        return lhs_adjusted.cmp(&rhs_adjusted);
    }
    // Same leading position: the exponents differ by fewer than 38 digits.
    let shift = (i64::from(lhs.exponent) - i64::from(rhs.exponent)).unsigned_abs();
    let shift = u32::try_from(shift).unwrap_or(u32::MAX).min(76);
    if lhs.exponent >= rhs.exponent {
        (lhs_coefficient * ten_to(shift)).cmp(&rhs_coefficient)
    } else {
        lhs_coefficient.cmp(&(rhs_coefficient * ten_to(shift)))
    }
}

/// Parses the finite subset of Python's decimal string syntax.
///
/// Like Python, it ignores surrounding whitespace, removes every underscore and
/// reads decimal digits of any script.
fn parse_python_decimal(raw: &str) -> Result<Dezimal, ParseDezimalError> {
    // Python also strips the separators U+001C to U+001F, which Rust does not
    // count as whitespace.
    let trimmed = raw.trim_matches(|character: char| {
        character.is_whitespace() || ('\u{1C}'..='\u{1F}').contains(&character)
    });
    let normalized: String = to_ascii_digits(trimmed)
        .chars()
        .filter(|character| *character != '_')
        .collect();
    let (negative, unsigned) = match normalized.as_bytes().first() {
        Some(b'-') => (true, normalized.get(1..).unwrap_or_default()),
        Some(b'+') => (false, normalized.get(1..).unwrap_or_default()),
        _ => (false, normalized.as_str()),
    };

    let (mantissa, exponent) = match unsigned.find(['e', 'E']) {
        Some(position) => (
            unsigned.get(..position).unwrap_or_default(),
            Some(unsigned.get(position + 1..).unwrap_or_default()),
        ),
        None => (unsigned, None),
    };

    let (integral, fractional) = match mantissa.split_once('.') {
        Some((integral, fractional)) => (integral, fractional),
        None => (mantissa, ""),
    };
    if !is_digits(integral)
        || !is_digits(fractional)
        || (integral.is_empty() && fractional.is_empty())
    {
        return Err(ParseDezimalError);
    }

    let exponent: i64 = match exponent {
        Some(raw_exponent) => {
            let (exponent_negative, digits) = match raw_exponent.as_bytes().first() {
                Some(b'-') => (true, raw_exponent.get(1..).unwrap_or_default()),
                Some(b'+') => (false, raw_exponent.get(1..).unwrap_or_default()),
                _ => (false, raw_exponent),
            };
            if digits.is_empty() || !is_digits(digits) {
                return Err(ParseDezimalError);
            }
            let magnitude: i64 = digits.parse().map_err(|_| ParseDezimalError)?;
            if exponent_negative {
                -magnitude
            } else {
                magnitude
            }
        }
        None => 0,
    };

    // value = digits × 10^(exponent - fraction length), without zeros that do not
    // change the value.
    let all_digits = format!("{integral}{fractional}");
    let significant = all_digits.trim_start_matches('0');
    if significant.is_empty() {
        // A zero keeps the limit on the exponent that every other value has:
        // Python rejects `0e1000000000000000000`, and an exponent beyond 32 bits
        // is rejected here as for any other value (a recorded deviation).
        let exponent = i64::try_from(fractional.len())
            .ok()
            .and_then(|length| exponent.checked_sub(length));
        if exponent
            .and_then(|value| i32::try_from(value).ok())
            .is_none()
        {
            return Err(ParseDezimalError);
        }
        return Ok(Dezimal {
            negative,
            coefficient: 0,
            exponent: 0,
        });
    }
    let trimmed_digits = significant.trim_end_matches('0');
    let trailing_zeros = significant.len() - trimmed_digits.len();
    if trimmed_digits.len() > MAX_DIGITS as usize {
        return Err(ParseDezimalError);
    }
    let coefficient: u128 = trimmed_digits.parse().map_err(|_| ParseDezimalError)?;
    let exponent = i64::try_from(trailing_zeros)
        .ok()
        .and_then(|zeros| exponent.checked_add(zeros))
        .and_then(|value| value.checked_sub(i64::try_from(fractional.len()).ok()?))
        .and_then(|value| i32::try_from(value).ok())
        .ok_or(ParseDezimalError)?;
    Ok(Dezimal {
        negative,
        coefficient,
        exponent,
    })
}

fn is_digits(part: &str) -> bool {
    part.bytes().all(|byte| byte.is_ascii_digit())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d(value: &str) -> Dezimal {
        Dezimal::from_python_str(value).unwrap()
    }

    #[test]
    fn limits_the_exponent_of_zero_like_that_of_any_value() {
        // Python rejects the first; the second exceeds 32 bits like `1e-2147483649`.
        for value in [
            "0e1000000000000000000",
            "0e999999999999999999",
            "0e-2147483649",
        ] {
            assert_eq!(
                Dezimal::from_python_str(value),
                Err(ParseDezimalError),
                "{value}"
            );
        }
        for value in ["0e0", "0e-500", "-0.000", "0e2147483647"] {
            assert!(d(value).is_zero(), "{value}");
        }
    }

    #[test]
    fn parses_python_decimal_strings() {
        assert_eq!(d("1.50"), d("1.5"));
        assert_eq!(d("  -2.5 "), (-d("2.5")).value().unwrap());
        assert_eq!(d("+3"), Dezimal::from(3));
        assert_eq!(d("1_000.25"), d("1000.25"));
        assert_eq!(d("1e3"), Dezimal::from(1000));
        assert_eq!(d("1.5E-2"), d("0.015"));
        assert_eq!(d(".5"), d("0.5"));
        assert_eq!(d("5."), Dezimal::from(5));
        assert_eq!(d("1e-05"), d("0.00001"));
        // Python removes every underscore and reads digits of any script.
        assert_eq!(d("_1"), Dezimal::ONE);
        assert_eq!(d("1__0"), Dezimal::from(10));
        assert_eq!(d("1_._5"), d("1.5"));
        assert_eq!(d("1e1_0"), Dezimal::from(10_000_000_000));
        assert_eq!(d("\u{0661}\u{0662}"), Dezimal::from(12));
        assert_eq!(d("\u{FF11}\u{FF12}.\u{FF15}\u{FF10}"), d("12.5"));
        assert_eq!(d("1.000000000000000000000000000000"), Dezimal::ONE);
        assert_eq!(d("000123.4500"), d("123.45"));
        assert_eq!(d("0e-500"), Dezimal::ZERO);
        assert_eq!(d("\u{1C}150\u{1F}"), Dezimal::from(150));
        assert!(Dezimal::from_python_str("1\u{1C}50").is_err());
        // Up to 38 significant digits are kept exactly, at any scale.
        assert_eq!(
            d("0.1234567890123456789012345678901").to_string(),
            "0.1234567890123456789012345678901"
        );
        assert!(d("1e-200") > Dezimal::ZERO);
    }

    #[test]
    fn rejects_non_finite_malformed_and_overlong_strings() {
        for value in [
            "",
            " ",
            "_",
            "abc",
            "NaN",
            "Infinity",
            "-Inf",
            "1..2",
            "1e",
            "e5",
            "1,5",
            "--1",
            "\u{00B2}",
            "123456789012345678901234567890123456789",
            "1e9223372036854775807",
            "1e-9223372036854775807",
        ] {
            assert_eq!(
                Dezimal::from_python_str(value),
                Err(ParseDezimalError),
                "{value:?}"
            );
        }
    }

    #[test]
    fn computes_in_python_s_context() {
        // 150 × (0.01 / 12): Python rounds the product to exactly 0.125.
        let rate = (d("0.01") / Dezimal::from(12)).value().unwrap();
        assert_eq!(rate.to_string(), "0.0008333333333333333333333333333");
        // Parsed values drop trailing zeros, so the places can differ from Python's.
        assert_eq!(d("1.50").to_string(), "1.5");
        assert_eq!(d("-2.505").round_cents().unwrap().to_string(), "-2.51");
        assert_eq!(d("-0.001").round_cents().unwrap().to_string(), "-0.00");
        assert_eq!(d("1.5").round_cents().unwrap().to_string(), "1.50");
        assert_eq!(d("100").round_cents().unwrap().to_string(), "100.00");
        assert_eq!(d("-0").round_cents().unwrap().to_string(), "-0.00");
        assert_eq!(
            d("9999999999999999999999999E+1")
                .round_cents()
                .unwrap()
                .to_string(),
            "99999999999999999999999990.00"
        );
        assert_eq!(
            d("1E+26").round_cents(),
            Err(ArithmeticError::InvalidOperation)
        );
        let interest = (Dezimal::from(150) * rate).value().unwrap();
        assert_eq!(interest, d("0.125"));
        assert_eq!(interest.round_cents().unwrap(), d("0.13"));
        assert_eq!(
            (d("0.035") / Dezimal::from(12))
                .value()
                .unwrap()
                .to_string(),
            "0.002916666666666666666666666667"
        );
        assert_eq!((d("1E+30") + d("1E-30")).value().unwrap(), d("1E+30"));
    }

    #[test]
    fn scaled_constants_are_exact() {
        assert_eq!(Dezimal::from_scaled(19, 2), d("0.19"));
        assert_eq!(Dezimal::from_scaled(-5, 0), d("-5"));
        assert_eq!(
            Dezimal::from_scaled(311_034_768, 7).to_string(),
            "31.1034768"
        );
    }

    #[test]
    fn rounds_half_even_like_python_round() {
        assert_eq!(d("0.125").round(2).unwrap().to_string(), "0.12");
        assert_eq!(d("0.135").round(2).unwrap().to_string(), "0.14");
        assert_eq!(d("1.5").round(2).unwrap().to_string(), "1.50");
        assert_eq!(d("-0.00005").round(4).unwrap().to_string(), "-0.0000");
        assert_eq!(d("2.5").round(0).unwrap().to_string(), "2");
        assert_eq!(
            d("1e24").round(4),
            Err(ArithmeticError::InvalidOperation),
            "29 digits"
        );
        assert!(d("1e23").round(4).is_ok());
        // Python accepts `round(Decimal(0), 39)` although 10^39 does not fit a
        // 128-bit coefficient; the scaled coefficient of a zero stays zero.
        let zero = d("0").round(39).unwrap();
        assert!(zero.is_zero());
        assert_eq!(zero.exponent(), -39);
        assert_eq!(zero.to_string().len(), 41);
        assert_eq!(d("-0").round(39).unwrap().exponent(), -39);
    }

    #[test]
    fn formats_fixed_places_like_python() {
        assert_eq!(d("0.125").to_fixed(2), "0.12");
        assert_eq!(d("0.135").to_fixed(2), "0.14");
        assert_eq!(d("100").to_fixed(2), "100.00");
        assert_eq!(d("1.5").to_fixed(2), "1.50");
        assert_eq!(d("-0.001").to_fixed(2), "-0.00");
        assert_eq!(d("1e30").to_fixed(2), "1000000000000000000000000000000.00");
        assert_eq!(d("12.5").to_fixed(0), "12");
        assert_eq!(d("0").to_fixed(3), "0.000");
    }

    #[test]
    fn rounds_half_up_away_from_zero() {
        assert_eq!(d("2.505").round_cents().unwrap(), d("2.51"));
        assert_eq!(d("-2.505").round_cents().unwrap(), d("-2.51"));
        assert_eq!(d("2.504999").round_cents().unwrap(), d("2.50"));
        assert_eq!(d("0.125").round_half_up(2), d("0.13"));
        assert_eq!(d("1e-50").round_cents().unwrap(), Dezimal::ZERO);
    }

    #[test]
    fn rounding_to_cents_fails_beyond_python_precision() {
        assert!(d("99999999999999999999999999.99").round_cents().is_ok());
        assert_eq!(
            d("99999999999999999999999999.995").round_cents(),
            Err(ArithmeticError::InvalidOperation)
        );
        assert_eq!(
            d("1E+26").round_cents(),
            Err(ArithmeticError::InvalidOperation)
        );
        assert!(d("9.9E+25").round_cents().is_ok());
    }

    #[test]
    fn arithmetic_propagates_errors() {
        let one = Dezimal::ONE;
        assert_eq!(
            (one / Dezimal::ZERO).value(),
            Err(ArithmeticError::DivisionByZero)
        );
        assert_eq!(
            ((one / Dezimal::ZERO) + one).value(),
            Err(ArithmeticError::DivisionByZero)
        );
        assert_eq!(
            (Dezimal::ZERO / Dezimal::ZERO).value(),
            Err(ArithmeticError::InvalidOperation)
        );
        assert_eq!(
            Dezimal::ZERO.powi(-1).value(),
            Err(ArithmeticError::DivisionByZero)
        );
        assert_eq!(
            d("1e999999").powi(2).value(),
            Err(ArithmeticError::Overflow)
        );
        // Exponents at both ends of the range neither panic nor overflow wrongly.
        let sum = (d("1e2000") + d("-1e-2147483648")).value().unwrap();
        assert_eq!(sum, d("1e2000"));
        assert!((d("1e2147483647") + d("-1e-2147483648")).value().is_err());
    }

    #[test]
    fn keeps_subnormal_values_like_python() {
        let tiny = (d("1e-999999") * d("0.1")).value().unwrap();
        assert_eq!(tiny, d("1E-1000000"));
        assert_eq!((tiny * d("1e999999")).value().unwrap(), d("0.1"));
        assert_eq!((d("1e-1000026") * d("0.1")).value().unwrap(), Dezimal::ZERO);
    }

    #[test]
    fn integer_powers() {
        assert_eq!(Dezimal::ONE.powi(1_000_000).value().unwrap(), Dezimal::ONE);
        assert_eq!(
            (-Dezimal::ONE).powi(1_000_001).value().unwrap(),
            (-Dezimal::ONE).value().unwrap()
        );
        assert_eq!(d("1.1").powi(2).value().unwrap(), d("1.21"));
        assert_eq!(d("2").powi(-2).value().unwrap(), d("0.25"));
        assert_eq!(d("5").powi(0).value().unwrap(), Dezimal::ONE);
        assert_eq!(d("-1.5").powi(7).value().unwrap(), d("-17.0859375"));
        assert_eq!(
            d("0.5").powi(-95).value().unwrap(),
            d("3.961408125713216879677197517E+28")
        );
    }

    #[test]
    fn converts_to_float_for_the_api() {
        assert_eq!(d("0.1").to_f64_for_api().to_bits(), 0.1f64.to_bits());
        assert_eq!(
            d("1234.56").to_f64_for_api().to_bits(),
            1234.56f64.to_bits()
        );
        assert!(d("-0.00").to_f64_for_api().is_sign_negative());
    }

    #[test]
    fn compares_and_hashes_by_value() {
        use std::collections::hash_map::DefaultHasher;
        let hash = |value: Dezimal| {
            let mut hasher = DefaultHasher::new();
            value.hash(&mut hasher);
            hasher.finish()
        };
        assert!(d("0.5") > 0);
        assert!(d("-0.5") < 0);
        assert_eq!(Dezimal::ZERO, 0);
        assert_eq!(d("-0"), Dezimal::ZERO);
        assert_eq!(hash(d("1.50")), hash(d("1.5")));
        assert_eq!(hash(d("-0")), hash(Dezimal::ZERO));
        assert!(d("1e-40") < d("1e-39"));
        assert!(d("-1e30") < d("-1e29"));
    }
}
