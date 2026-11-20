// SPDX-License-Identifier: AGPL-3.0-only

//! Readers of stored values shared by the repositories.

use core_domain::dates::Date;

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
