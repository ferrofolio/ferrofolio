// SPDX-License-Identifier: AGPL-3.0-only

//! Readers of stored values shared by the repositories.

use core_domain::Dezimal;
use core_domain::dates::Date;
use rusqlite::types::ValueRef;

/// The date of a `YYYY-MM-DD` text, the form `date.isoformat()` writes, in
/// Python's range of years. Any other form, which `date.fromisoformat` or
/// pydantic may still read, is `None`, so that the caller leaves it to Python.
pub(crate) fn iso_date(text: &str) -> Option<Date> {
    let bytes = text.as_bytes();
    let shape = bytes.len() == 10
        && bytes.get(4) == Some(&b'-')
        && bytes.get(7) == Some(&b'-')
        && bytes
            .iter()
            .enumerate()
            .all(|(index, byte)| index == 4 || index == 7 || byte.is_ascii_digit());
    if !shape {
        return None;
    }
    let number = |range: std::ops::Range<usize>| -> Option<i16> { text.get(range)?.parse().ok() };
    let year = number(0..4)?;
    if year < 1 {
        return None;
    }
    let month = i8::try_from(number(5..7)?).ok()?;
    let day = i8::try_from(number(8..10)?).ok()?;
    Date::new(year, month, day).ok()
}

/// A value read from a column, or the column that holds a form left to Python.
pub(crate) type Read<T> = Result<T, String>;

/// A text column, `None` for NULL. Any other type is unreadable: pydantic does
/// not read a number as a `str`, and a blob is not what the writer stores.
pub(crate) fn text<'v>(value: ValueRef<'v>, what: &str) -> Read<Option<&'v str>> {
    match value {
        ValueRef::Null => Ok(None),
        ValueRef::Text(bytes) => std::str::from_utf8(bytes)
            .map(Some)
            .map_err(|_| what.to_owned()),
        _ => Err(what.to_owned()),
    }
}

/// A text column that must hold a value.
pub(crate) fn required<'v>(value: ValueRef<'v>, what: &str) -> Read<&'v str> {
    text(value, what)?.ok_or_else(|| what.to_owned())
}

/// `Dezimal(text)` of an amount the writer stores as `str(Dezimal)`. A number that
/// a float cannot hold, which Python would answer as `Infinity`, is unreadable.
pub(crate) fn amount(text: &str, what: &str) -> Read<Dezimal> {
    match Dezimal::from_python_str(text) {
        Ok(amount) if amount.to_f64_for_api().is_finite() => Ok(amount),
        _ => Err(what.to_owned()),
    }
}

/// A flag the writer stores as a Python `bool`, which SQLite keeps as `0` or `1`.
pub(crate) fn flag(value: ValueRef<'_>, what: &str) -> Read<bool> {
    match value {
        ValueRef::Integer(0) => Ok(false),
        ValueRef::Integer(1) => Ok(true),
        _ => Err(what.to_owned()),
    }
}

/// A date the writer stores as `date.isoformat()`.
pub(crate) fn date(value: ValueRef<'_>, what: &str) -> Read<Option<Date>> {
    match text(value, what)? {
        None => Ok(None),
        Some(text) => iso_date(text).map(Some).ok_or_else(|| what.to_owned()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_only_the_iso_form() {
        assert_eq!(iso_date("2024-02-29"), Date::new(2024, 2, 29).ok());
        assert_eq!(iso_date("0001-01-01"), Date::new(1, 1, 1).ok());
        for text in [
            "0000-01-01",
            "2026-02-30",
            "20260101",
            "2026-1-01",
            "2026-01-01T00:00",
            " 2026-01-01",
            "２０２６-01-01",
            "",
        ] {
            assert_eq!(iso_date(text), None, "{text}");
        }
    }
}
