// SPDX-License-Identifier: AGPL-3.0-only

//! Request parsing with the semantics of the Python routes.
//!
//! The Python controllers read the JSON body into Python objects and convert fields
//! with `Dezimal(...)`, `int(...)`, enum constructors and `date.fromisoformat`.
//! The helpers here accept the same inputs and produce the same error messages for
//! the common cases, so that ported routes answer like the Python ones.

use core_domain::Dezimal;
use core_domain::dates::Date;
use core_domain::unicode_digits::to_ascii_digits;

pub use core_domain::pyrepr::{repr_str, repr_str_limited};
use jiff::civil::{ISOWeekDate, Weekday};
use serde_json::{Map, Value};

/// Python's truthiness of a JSON value.
pub fn truthy(value: &Value) -> bool {
    match value {
        Value::Null => false,
        Value::Bool(flag) => *flag,
        // A number beyond the float range is infinite in Python, and so truthy.
        Value::Number(number) => number
            .as_i64()
            .map(|integer| integer != 0)
            .or_else(|| number.as_u64().map(|integer| integer != 0))
            .unwrap_or_else(|| number.as_f64().is_none_or(|float| float != 0.0)),
        Value::String(text) => !text.is_empty(),
        Value::Array(items) => !items.is_empty(),
        Value::Object(fields) => !fields.is_empty(),
    }
}

/// `str.strip()`: Python also strips the separators U+001C to U+001F, which Rust
/// does not count as whitespace.
pub fn strip(text: &str) -> &str {
    core_domain::text::strip(text)
}

/// The name of the Python type a JSON value is parsed into.
pub fn type_name(value: &Value) -> &'static str {
    match value {
        Value::Null => "NoneType",
        Value::Bool(_) => "bool",
        Value::Number(number) if is_integer_token(number) => "int",
        Value::Number(_) => "float",
        Value::String(_) => "str",
        Value::Array(_) => "list",
        Value::Object(_) => "dict",
    }
}

/// `repr()` of the Python object a JSON value is parsed into.
pub fn repr(value: &Value) -> String {
    match value {
        Value::Null => "None".to_owned(),
        Value::Bool(true) => "True".to_owned(),
        Value::Bool(false) => "False".to_owned(),
        Value::Number(number) if is_integer_token(number) => integer_text(number),
        // Beyond the float range Python's json module reads infinity.
        Value::Number(number) => match number.as_f64() {
            Some(float) => repr_float(float),
            None if number.to_string().starts_with('-') => "-inf".to_owned(),
            None => "inf".to_owned(),
        },
        Value::String(text) => repr_str(text),
        Value::Array(items) => {
            let items: Vec<String> = items.iter().map(repr).collect();
            format!("[{}]", items.join(", "))
        }
        Value::Object(fields) => {
            let fields: Vec<String> = fields
                .iter()
                .map(|(key, value)| format!("{}: {}", repr_str(key), repr(value)))
                .collect();
            format!("{{{}}}", fields.join(", "))
        }
    }
}

/// `repr()` of a Python float: the shortest round-trip digits, with an exponent of
/// at least two digits and an explicit sign (`1e+16`, `1e-05`).
fn repr_float(float: f64) -> String {
    let debug = format!("{float:?}");
    match debug.split_once('e') {
        Some((mantissa, exponent)) => {
            let (sign, digits) = match exponent.strip_prefix('-') {
                Some(digits) => ('-', digits),
                None => ('+', exponent),
            };
            format!("{mantissa}e{sign}{digits:0>2}")
        }
        None => debug,
    }
}

/// Whether a JSON number is written without a fraction or an exponent, which
/// Python's `json` module reads as an exact `int`.
fn is_integer_token(number: &serde_json::Number) -> bool {
    !number.to_string().contains(['.', 'e', 'E'])
}

/// The value of an integer token as Python's `int` has it: `-0` is plain `0`, as
/// integers have no signed zero.
fn integer_text(number: &serde_json::Number) -> String {
    let text = number.to_string();
    if text == "-0" { "0".to_owned() } else { text }
}

/// The message of a `KeyError` for a missing field.
pub fn key_error(key: &str) -> String {
    repr_str(key)
}

/// `body[key]`: the field, or the `KeyError` message.
pub fn required<'a>(body: &'a Map<String, Value>, key: &str) -> Result<&'a Value, String> {
    body.get(key).ok_or_else(|| key_error(key))
}

/// `body.get(key)` that is not `None`.
pub fn optional<'a>(body: &'a Map<String, Value>, key: &str) -> Option<&'a Value> {
    body.get(key).filter(|value| !value.is_null())
}

/// `Dezimal(value)`.
pub fn dezimal(value: &Value) -> Result<Dezimal, String> {
    match value {
        Value::Bool(flag) => Ok(Dezimal::from(i64::from(*flag))),
        // Python reads an integer exactly, whatever its size.
        Value::Number(number) if is_integer_token(number) => {
            Dezimal::from_python_str(&integer_text(number)).map_err(|_| {
                "Expecting int, float, Decimal or a str.Found <class 'int'>.".to_owned()
            })
        }
        // Python reads a number with a fraction or an exponent as a float first.
        // Money never goes through a float here: the number is read exactly from its
        // JSON text. The results only differ for numbers with more digits than a
        // float holds (docs/rust-backend.md, known deviations).
        Value::Number(number) => Dezimal::from_python_str(&number.to_string()).map_err(|_| {
            "Expecting int, float, Decimal or a str.Found <class 'float'>.".to_owned()
        }),
        Value::String(text) => Dezimal::from_python_str(text)
            .map_err(|_| "Expecting int, float, Decimal or a str.Found <class 'str'>.".to_owned()),
        Value::Null | Value::Array(_) | Value::Object(_) => {
            Err(format!("Invalid type <class '{}'>", type_name(value)))
        }
    }
}

/// `int(value)`: truncates floats and parses integer strings.
pub fn int(value: &Value) -> Result<i64, String> {
    match value {
        Value::Bool(flag) => Ok(i64::from(*flag)),
        // An integer keeps its exact value; Python's are unbounded, these are 64 bits.
        Value::Number(number) if is_integer_token(number) => number
            .to_string()
            .parse()
            .map_err(|_| "integer out of range".to_owned()),
        Value::Number(number) => {
            // Beyond the float range Python's json module reads infinity.
            let Some(float) = number.as_f64() else {
                return Err("cannot convert float infinity to integer".to_owned());
            };
            let truncated = float.trunc();
            // `as` saturates; the range check rejects values that do not fit.
            if fits_i64(truncated) {
                return Ok(truncated as i64);
            }
            Err("cannot convert float to integer".to_owned())
        }
        // `int()` reads digits of any script; pydantic does not, see `lax_int`.
        Value::String(text) => parse_int_str(&to_ascii_digits(text)).ok_or_else(|| {
            format!(
                "invalid literal for int() with base 10: {}",
                repr_str_limited(text, 200)
            )
        }),
        Value::Null | Value::Array(_) | Value::Object(_) => Err(format!(
            "int() argument must be a string, a bytes-like object or a real number, not '{}'",
            type_name(value)
        )),
    }
}

/// Validation of a pydantic `bool` field in lax mode: booleans, the numbers 0 and 1,
/// and the strings `0 off f false n no` and `1 on t true y yes` in any case, without
/// surrounding whitespace. `None` for every other value, which pydantic rejects.
pub fn lax_bool(value: &Value) -> Option<bool> {
    match value {
        Value::Bool(flag) => Some(*flag),
        Value::Number(number) => match number.as_f64() {
            Some(1.0) => Some(true),
            Some(0.0) => Some(false),
            _ => None,
        },
        Value::String(text) => match text.to_ascii_lowercase().as_str() {
            "1" | "on" | "t" | "true" | "y" | "yes" => Some(true),
            "0" | "off" | "f" | "false" | "n" | "no" => Some(false),
            _ => None,
        },
        Value::Null | Value::Array(_) | Value::Object(_) => None,
    }
}

/// The first line of pydantic's error for a value `lax_int` rejects.
pub fn lax_int_error(value: &Value) -> &'static str {
    match value {
        Value::Number(number) if !is_integer_token(number) => match number.as_f64() {
            None => "Input should be a finite number",
            Some(float) if float.fract() != 0.0 => {
                "Input should be a valid integer, got a number with a fractional part"
            }
            Some(_) => "Unable to parse input string as an integer, exceeded maximum size",
        },
        Value::String(_) => "Input should be a valid integer, unable to parse string as an integer",
        // Integers beyond 64 bits, which pydantic accepts (a recorded deviation),
        // and values of other types.
        _ => "Input should be a valid integer",
    }
}

/// Integer validation of a pydantic field in lax mode: integers, booleans, floats
/// without a fractional part and integer strings.
pub fn lax_int(value: &Value) -> Option<i64> {
    match value {
        Value::Bool(flag) => Some(i64::from(*flag)),
        Value::Number(number) if is_integer_token(number) => number.to_string().parse().ok(),
        Value::Number(number) => {
            let float = number.as_f64()?;
            (float.fract() == 0.0 && fits_i64(float)).then_some(float as i64)
        }
        Value::String(text) => {
            // pydantic trims the whole string and also accepts a fractional part
            // that is all zeros ("10.0"), but no space before the point.
            match text.trim().split_once('.') {
                Some((integral, fraction))
                    if !fraction.is_empty()
                        && fraction.bytes().all(|byte| byte == b'0')
                        && integral.trim() == integral =>
                {
                    parse_int_str(integral)
                }
                Some(_) => None,
                None => parse_int_str(text),
            }
        }
        Value::Null | Value::Array(_) | Value::Object(_) => None,
    }
}

/// Whether an integral float lies in the range of `i64`: from -2^63 inclusive to
/// 2^63 exclusive, both exactly representable as floats.
fn fits_i64(float: f64) -> bool {
    (-9_223_372_036_854_775_808.0..9_223_372_036_854_775_808.0).contains(&float)
}

/// An integer literal as `int(str)` accepts it: whitespace around, a sign and
/// underscores between digits.
fn parse_int_str(text: &str) -> Option<i64> {
    let trimmed = text.trim();
    let (negative, digits) = match trimmed.as_bytes().first() {
        Some(b'-') => (true, trimmed.get(1..)?),
        Some(b'+') => (false, trimmed.get(1..)?),
        _ => (false, trimmed),
    };
    if digits.is_empty()
        || digits.starts_with('_')
        || digits.ends_with('_')
        || digits.contains("__")
        || !digits
            .bytes()
            .all(|byte| byte.is_ascii_digit() || byte == b'_')
    {
        return None;
    }
    // Parsing with the sign keeps i64::MIN, whose magnitude does not fit.
    let sign = if negative { "-" } else { "" };
    format!("{sign}{}", digits.replace('_', "")).parse().ok()
}

/// `date.fromisoformat(text)` for the formats Python accepts for dates:
/// `YYYY-MM-DD`, `YYYYMMDD`, `YYYY-Www`, `YYYYWww`, `YYYY-Www-D` and `YYYYWwwD`.
///
/// A well-formed calendar date that does not exist fails with the message of
/// Python's `date` constructor, everything else as an invalid isoformat string.
pub fn iso_date(text: &str) -> Result<Date, String> {
    let invalid = || format!("Invalid isoformat string: {}", repr_str(text));
    let bytes = text.as_bytes();
    let number = |range: std::ops::Range<usize>| -> Option<u32> {
        let part = text.get(range)?;
        part.bytes()
            .all(|byte| byte.is_ascii_digit())
            .then(|| part.parse().ok())?
    };
    let dash = |index: usize| bytes.get(index) == Some(&b'-');
    let week_marker = |index: usize| bytes.get(index) == Some(&b'W');
    match bytes.len() {
        10 if dash(4) && dash(7) => {
            calendar_date(number(0..4), number(5..7), number(8..10)).ok_or_else(invalid)?
        }
        // CPython 3.14 reads a ten-character compact date from its first eight
        // characters and ignores the rest ("20240101xx" is 1 January 2024).
        8 | 10 if !dash(4) && !week_marker(4) => {
            calendar_date(number(0..4), number(4..6), number(6..8)).ok_or_else(invalid)?
        }
        10 if dash(4) && week_marker(5) && dash(8) => {
            week_date(number(0..4), number(6..8), number(9..10)).ok_or_else(invalid)?
        }
        8 if dash(4) && week_marker(5) => {
            week_date(number(0..4), number(6..8), Some(1)).ok_or_else(invalid)?
        }
        // The same quirk for compact week dates: "2025W011xx" is 30 December 2024.
        // CPython counts the UTF-8 bytes, so "2025W011é" has ten as well.
        8 | 10 if week_marker(4) => {
            week_date(number(0..4), number(5..7), number(7..8)).ok_or_else(invalid)?
        }
        7 if week_marker(4) => {
            week_date(number(0..4), number(5..7), Some(1)).ok_or_else(invalid)?
        }
        _ => Err(invalid()),
    }
}

/// A calendar date with the range checks of Python's `date(year, month, day)`.
/// `None` when a component is not a number.
fn calendar_date(
    year: Option<u32>,
    month: Option<u32>,
    day: Option<u32>,
) -> Option<Result<Date, String>> {
    let (year, month, day) = (year?, month?, day?);
    if !(1..=9999).contains(&year) {
        return Some(Err(format!("year must be in 1..9999, not {year}")));
    }
    if !(1..=12).contains(&month) {
        return Some(Err(format!("month must be in 1..12, not {month}")));
    }
    let year_number = i16::try_from(year).ok()?;
    let month_number = i8::try_from(month).ok()?;
    let days_in_month = Date::new(year_number, month_number, 1)
        .ok()?
        .days_in_month();
    let date = i8::try_from(day)
        .ok()
        .and_then(|day| Date::new(year_number, month_number, day).ok());
    Some(date.ok_or_else(|| {
        format!("day {day} must be in range 1..{days_in_month} for month {month} in year {year}")
    }))
}

/// An ISO week date; `None` when the week or weekday does not exist or a part is not
/// a number. A date after 9999-12-31 fails like Python's `date` constructor.
fn week_date(
    year: Option<u32>,
    week: Option<u32>,
    weekday: Option<u32>,
) -> Option<Result<Date, String>> {
    let weekday = match weekday? {
        1 => Weekday::Monday,
        2 => Weekday::Tuesday,
        3 => Weekday::Wednesday,
        4 => Weekday::Thursday,
        5 => Weekday::Friday,
        6 => Weekday::Saturday,
        7 => Weekday::Sunday,
        _ => return None,
    };
    let year = i16::try_from(year?).ok()?;
    let week = i8::try_from(week?).ok()?;
    match ISOWeekDate::new(year, week, weekday) {
        Ok(date) => {
            Some(Ok(date.date())).filter(|date| date.as_ref().is_ok_and(|date| date.year() >= 1))
        }
        // The week exists, but this day of it falls into year 10000.
        Err(_) if year == 9999 && ISOWeekDate::new(year, week, Weekday::Monday).is_ok() => {
            Some(Err("year must be in 1..9999, not 10000".to_owned()))
        }
        Err(_) => None,
    }
}

/// The values of `name` in a query string as Quart's `request.args.getlist(name)`
/// reads them: `parse_qsl(query, keep_blank_values=True,
/// errors="werkzeug.url_quote")` splits at `&` only, skips empty fields, takes a
/// field without `=` as an empty value, reads `+` as a space and decodes
/// percent-escapes as UTF-8, writing bytes that are not UTF-8 as upper-case escapes.
pub fn query_values(query: &str, name: &str) -> Vec<String> {
    query
        .split('&')
        .filter(|field| !field.is_empty())
        .filter_map(|field| {
            let (key, value) = field.split_once('=').unwrap_or((field, ""));
            (unquote_plus(key) == name).then(|| unquote_plus(value))
        })
        .collect()
}

/// `unquote(text.replace("+", " "), errors="werkzeug.url_quote")`.
fn unquote_plus(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while let Some(&byte) = bytes.get(index) {
        // `from_str_radix` would also take a sign, which is no escape.
        let escaped = (byte == b'%')
            .then(|| bytes.get(index + 1..index + 3))
            .flatten()
            .filter(|digits| digits.iter().all(u8::is_ascii_hexdigit))
            .and_then(|digits| std::str::from_utf8(digits).ok())
            .and_then(|digits| u8::from_str_radix(digits, 16).ok());
        match escaped {
            Some(value) => {
                decoded.push(value);
                index += 3;
            }
            None => {
                decoded.push(if byte == b'+' { b' ' } else { byte });
                index += 1;
            }
        }
    }
    let mut text = String::with_capacity(decoded.len());
    let mut rest = decoded.as_slice();
    loop {
        match std::str::from_utf8(rest) {
            Ok(valid) => {
                text.push_str(valid);
                return text;
            }
            Err(error) => {
                let (valid, after) = rest.split_at(error.valid_up_to());
                text.push_str(std::str::from_utf8(valid).unwrap_or_default());
                let invalid = error.error_len().unwrap_or(after.len());
                let (bad, tail) = after.split_at(invalid.min(after.len()));
                for byte in bad {
                    text.push_str(&format!("%{byte:02X}"));
                }
                rest = tail;
            }
        }
    }
}

/// What `int(text)` gives for a text.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PyInt {
    /// An integer in the range of `i64`.
    Value(i64),
    /// Python fails with a `ValueError`.
    Invalid,
    /// An integer outside the range of `i64`, which Python reads exactly, or fails
    /// to read when it has more than 4300 digits.
    OutOfRange,
}

/// `int(text)`: whitespace around, a sign, and decimal digits of any script with
/// single underscores between them. CPython keeps ASCII characters as they are
/// and strips only ASCII whitespace among them, so unlike `str.strip()` it keeps
/// the separators U+001C to U+001F, as Rust's `trim` does.
pub fn py_int(text: &str) -> PyInt {
    let stripped = text.trim();
    let (negative, digits) = match stripped.chars().next() {
        Some('-') => (true, stripped.get(1..).unwrap_or_default()),
        Some('+') => (false, stripped.get(1..).unwrap_or_default()),
        _ => (false, stripped),
    };
    let mut value: i128 = 0;
    let mut out_of_range = false;
    let mut previous_digit = false;
    let mut any = false;
    // CPython refuses more than 4300 digits, leading zeros included; Rust leaves
    // such a text to Python as one out of range.
    let mut count: usize = 0;
    for character in digits.chars() {
        if character == '_' {
            if !previous_digit {
                return PyInt::Invalid;
            }
            previous_digit = false;
            continue;
        }
        let Some(digit) = core_domain::unicode_digits::decimal_digit(character) else {
            return PyInt::Invalid;
        };
        previous_digit = true;
        any = true;
        count += 1;
        out_of_range |= count > 4300;
        if !out_of_range {
            value = value * 10 + i128::from(digit);
            out_of_range = value > i128::from(i64::MAX) + 1;
        }
    }
    if !any || !previous_digit {
        return PyInt::Invalid;
    }
    let signed = if negative { -value } else { value };
    match i64::try_from(signed) {
        Ok(value) if !out_of_range => PyInt::Value(value),
        _ => PyInt::OutOfRange,
    }
}

/// `Enum(value)` failure message: `'X' is not a valid InterestType`.
pub fn invalid_enum(value: &Value, enum_name: &str) -> String {
    format!("{} is not a valid {enum_name}", repr(value))
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn reads_integers_like_python() {
        assert_eq!(py_int(" +1_000 "), PyInt::Value(1000));
        assert_eq!(py_int("-0"), PyInt::Value(0));
        assert_eq!(py_int("007"), PyInt::Value(7));
        assert_eq!(
            py_int("\u{3000}\u{0662}\u{0663}\u{85}\u{0B}"),
            PyInt::Value(23)
        );
        assert_eq!(py_int("\u{1C}1"), PyInt::Invalid);
        assert_eq!(py_int("-9223372036854775808"), PyInt::Value(i64::MIN));
        assert_eq!(py_int("9223372036854775808"), PyInt::OutOfRange);
        assert_eq!(py_int(&"9".repeat(5000)), PyInt::OutOfRange);
        assert_eq!(py_int(&"0".repeat(4300)), PyInt::Value(0));
        assert_eq!(py_int(&"0".repeat(4301)), PyInt::OutOfRange);
        for text in [
            "",
            " ",
            "+",
            "-",
            "_1",
            "1_",
            "1__0",
            "+_1",
            "1 0",
            "1.0",
            "0x1",
            "--1",
            "\u{200B}1",
        ] {
            assert_eq!(py_int(text), PyInt::Invalid, "{text:?}");
        }
    }

    #[test]
    fn reads_query_values_like_quart() {
        let values = |query: &str| query_values(query, "key");
        assert_eq!(values("key=a&other=b&key=c"), ["a", "c"]);
        assert_eq!(values("&&key&key=&"), ["", ""]);
        assert_eq!(values("k%65y=gen+eral%2B"), ["gen eral+"]);
        assert_eq!(values("key==x;key=y"), ["=x;key=y"]);
        assert_eq!(
            values("key=%C3%A9%ff%e2%82x%zz%4"),
            ["\u{e9}%FF%E2%82x%zz%4"]
        );
        assert_eq!(values("KEY=a"), Vec::<String>::new());
    }

    #[test]
    fn validates_bools_like_pydantic() {
        // Recorded from pydantic with a `bool` field of a pydantic dataclass.
        let cases = [
            ("true", Some(true)),
            ("false", Some(false)),
            ("\"false\"", Some(false)),
            ("\"FALSE\"", Some(false)),
            ("\"yes\"", Some(true)),
            ("\"Off\"", Some(false)),
            ("\"t\"", Some(true)),
            ("\"n\"", Some(false)),
            ("\"1\"", Some(true)),
            ("\"0\"", Some(false)),
            ("\"on\"", Some(true)),
            ("1", Some(true)),
            ("0", Some(false)),
            ("1.0", Some(true)),
            ("0.0", Some(false)),
            ("2", None),
            ("0.5", None),
            ("null", None),
            ("\"\"", None),
            ("\" true\"", None),
            ("[]", None),
            ("{}", None),
        ];
        for (json, expected) in cases {
            let value: Value = serde_json::from_str(json).unwrap();
            assert_eq!(lax_bool(&value), expected, "{json}");
        }
    }

    #[test]
    fn truthiness_follows_python() {
        for falsy in [
            json!(null),
            json!(false),
            json!(0),
            json!(0.0),
            json!(""),
            json!([]),
            json!({}),
        ] {
            assert!(!truthy(&falsy), "{falsy}");
        }
        // Python reads -0 as the integer 0, which has no sign.
        let negative_zero: Value = serde_json::from_str("-0").unwrap();
        assert_eq!(repr(&negative_zero), "0");
        // Display shows the sign of zero; is_sign_negative ignores it.
        assert_eq!(dezimal(&negative_zero).unwrap().to_string(), "0");
        let negative_float_zero: Value = serde_json::from_str("-0.0").unwrap();
        assert_eq!(repr(&negative_float_zero), "-0.0");
        let huge: Value = serde_json::from_str("1e400").unwrap();
        assert!(truthy(&huge));
        let tiny: Value = serde_json::from_str("1e-400").unwrap();
        assert!(!truthy(&tiny));
        for truthy_value in [
            json!(true),
            json!(1),
            json!(-0.5),
            json!("0"),
            json!([0]),
            json!({"a": 0}),
        ] {
            assert!(truthy(&truthy_value), "{truthy_value}");
        }
    }

    #[test]
    fn dezimal_conversion_matches_python() {
        let d = |text: &str| Dezimal::from_python_str(text).unwrap();
        assert_eq!(dezimal(&json!(0.1)).unwrap(), d("0.1"));
        assert_eq!(dezimal(&json!(1e-5)).unwrap(), d("0.00001"));
        assert_eq!(dezimal(&json!(250000)).unwrap(), d("250000"));
        assert_eq!(dezimal(&json!("0.035")).unwrap(), d("0.035"));
        assert_eq!(dezimal(&json!(true)).unwrap(), Dezimal::ONE);
        // Integers beyond 64 bits keep every digit.
        let big: Value = serde_json::from_str("18446744073709551617").unwrap();
        assert_eq!(dezimal(&big).unwrap(), d("18446744073709551617"));
        let fraction: Value = serde_json::from_str("1234.5600").unwrap();
        assert_eq!(dezimal(&fraction).unwrap(), d("1234.56"));
        assert_eq!(
            dezimal(&json!("abc")).unwrap_err(),
            "Expecting int, float, Decimal or a str.Found <class 'str'>."
        );
        assert_eq!(
            dezimal(&json!(null)).unwrap_err(),
            "Invalid type <class 'NoneType'>"
        );
        assert_eq!(
            dezimal(&json!([1])).unwrap_err(),
            "Invalid type <class 'list'>"
        );
    }

    #[test]
    fn int_conversion_matches_python() {
        assert_eq!(int(&json!(7)), Ok(7));
        assert_eq!(int(&json!(7.9)), Ok(7));
        assert_eq!(int(&json!(-7.9)), Ok(-7));
        assert_eq!(int(&json!(" 1_000 ")), Ok(1000));
        assert_eq!(int(&json!(true)), Ok(1));
        assert_eq!(int(&json!("\u{0662}")), Ok(2));
        assert_eq!(int(&json!("-9223372036854775808")), Ok(i64::MIN));
        // An integer below i64::MIN is not rounded into range through a float.
        let below: Value = serde_json::from_str("-9223372036854775809").unwrap();
        assert!(int(&below).is_err());
        assert_eq!(lax_int(&below), None);
        let minimum: Value = serde_json::from_str("-9223372036854775808").unwrap();
        assert_eq!(int(&minimum), Ok(i64::MIN));
        assert_eq!(int(&json!(9e18)), Ok(9_000_000_000_000_000_000));
        assert_eq!(int(&json!(-9.223_372_036_854_776e18)), Ok(i64::MIN));
        assert!(int(&json!(9.223_372_036_854_776e18)).is_err());
        assert_eq!(int(&json!("\u{0662}_\u{0663}")), Ok(23));
        assert!(int(&json!("1__0")).is_err());
        assert_eq!(
            int(&json!("1.5")).unwrap_err(),
            "invalid literal for int() with base 10: '1.5'"
        );
        // CPython shows at most 200 characters of the value.
        let long = "x".repeat(1_000_000);
        let message = int(&json!(long)).unwrap_err();
        assert_eq!(
            message,
            format!(
                "invalid literal for int() with base 10: '{}",
                "x".repeat(199)
            )
        );
        assert_eq!(
            int(&json!([])).unwrap_err(),
            "int() argument must be a string, a bytes-like object or a real number, not 'list'"
        );
    }

    #[test]
    fn lax_int_errors_name_the_problem_like_pydantic() {
        assert_eq!(
            lax_int_error(&json!(2.5)),
            "Input should be a valid integer, got a number with a fractional part"
        );
        assert_eq!(
            lax_int_error(&json!("abc")),
            "Input should be a valid integer, unable to parse string as an integer"
        );
        assert_eq!(
            lax_int_error(&json!([1])),
            "Input should be a valid integer"
        );
        let infinite: Value = serde_json::from_str("1e400").unwrap();
        assert_eq!(lax_int_error(&infinite), "Input should be a finite number");
        assert_eq!(
            lax_int_error(&json!(1e20)),
            "Unable to parse input string as an integer, exceeded maximum size"
        );
    }

    #[test]
    fn lax_int_matches_pydantic() {
        assert_eq!(lax_int(&json!(10)), Some(10));
        assert_eq!(lax_int(&json!(10.0)), Some(10));
        assert_eq!(lax_int(&json!(10.5)), None);
        assert_eq!(lax_int(&json!("10")), Some(10));
        assert_eq!(lax_int(&json!(" 10.00 ")), Some(10));
        assert_eq!(lax_int(&json!("10 .00")), None);
        assert_eq!(lax_int(&json!("10.")), None);
        assert_eq!(lax_int(&json!("10.5")), None);
        assert_eq!(lax_int(&json!("1e1")), None);
        assert_eq!(lax_int(&json!(null)), None);
        // pydantic rejects digits of other scripts.
        assert_eq!(lax_int(&json!("\u{0662}")), None);
    }

    #[test]
    fn iso_dates_match_python() {
        let date = |y, m, d| Date::new(y, m, d).unwrap();
        assert_eq!(iso_date("2024-02-29"), Ok(date(2024, 2, 29)));
        assert_eq!(iso_date("20240229"), Ok(date(2024, 2, 29)));
        assert_eq!(iso_date("2025-W01-1"), Ok(date(2024, 12, 30)));
        assert_eq!(iso_date("2025W017"), Ok(date(2025, 1, 5)));
        assert_eq!(iso_date("2020-W53-1"), Ok(date(2020, 12, 28)));
        assert_eq!(iso_date("20240101xx"), Ok(date(2024, 1, 1)));
        assert_eq!(iso_date("2025W011xx"), Ok(date(2024, 12, 30)));
        assert_eq!(iso_date("2025W0111x"), Ok(date(2024, 12, 30)));
        assert_eq!(iso_date("2025W011\u{e9}"), Ok(date(2024, 12, 30)));
        assert_eq!(iso_date("2026W5311x"), Ok(date(2026, 12, 28)));
        for invalid in ["2025W01xxx", "2025W01x1x", "2025W0119", "2025W019xx"] {
            assert_eq!(
                iso_date(invalid),
                Err(format!("Invalid isoformat string: '{invalid}'"))
            );
        }
        assert_eq!(iso_date("2020-W01"), Ok(date(2019, 12, 30)));
        assert_eq!(iso_date("9999-W52-5"), Ok(date(9999, 12, 31)));
        assert_eq!(
            iso_date("9999-W52-6").unwrap_err(),
            "year must be in 1..9999, not 10000"
        );
        assert_eq!(
            iso_date("2020-02-30").unwrap_err(),
            "day 30 must be in range 1..29 for month 2 in year 2020"
        );
        assert_eq!(
            iso_date("20201301").unwrap_err(),
            "month must be in 1..12, not 13"
        );
        assert_eq!(
            iso_date("0000-01-01").unwrap_err(),
            "year must be in 1..9999, not 0"
        );
        for invalid in [
            "2025-1-01",
            "2021-W53-1",
            "2020-W01-0",
            "",
            "2025-01-01T00:00",
            "abcd-ef-gh",
        ] {
            assert_eq!(
                iso_date(invalid).unwrap_err(),
                format!("Invalid isoformat string: {}", repr_str(invalid))
            );
        }
    }

    #[test]
    fn messages_quote_like_python_repr() {
        assert_eq!(
            invalid_enum(&json!("fixed"), "InterestType"),
            "'fixed' is not a valid InterestType"
        );
        assert_eq!(
            invalid_enum(&json!(null), "InterestType"),
            "None is not a valid InterestType"
        );
        assert_eq!(repr_str("it's"), "\"it's\"");
        assert_eq!(repr_str("it's \"q\"\\"), "'it\\'s \"q\"\\\\'");
        assert_eq!(repr_str_limited("ab\u{7}", 4), "'ab\\");
        assert_eq!(repr_str_limited("ab", 4), "'ab'");
        assert_eq!(
            invalid_enum(&json!([true, null, 1.5, "a"]), "InterestType"),
            "[True, None, 1.5, 'a'] is not a valid InterestType"
        );
        assert_eq!(repr(&json!({"a": 1})), "{'a': 1}");
        assert_eq!(repr(&json!(1e16)), "1e+16");
        assert_eq!(repr(&json!(0.00001)), "1e-05");
        assert_eq!(repr(&json!(5.0)), "5.0");
        let huge: Value = serde_json::from_str("1e400").unwrap();
        assert_eq!(
            int(&huge).unwrap_err(),
            "cannot convert float infinity to integer"
        );
        let infinite: Value = serde_json::from_str("-1e400").unwrap();
        assert_eq!(
            invalid_enum(&infinite, "InterestType"),
            "-inf is not a valid InterestType"
        );
        assert_eq!(
            repr_str("a\u{00A0}b\u{200B}\u{E0001}"),
            "'a\\xa0b\\u200b\\U000e0001'"
        );
        assert_eq!(repr_str("caf\u{00E9} \u{1F600}"), "'caf\u{00E9} \u{1F600}'");
        assert_eq!(key_error("start"), "'start'");
    }
}
