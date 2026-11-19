// SPDX-License-Identifier: AGPL-3.0-only

//! Python's `datetime.datetime`, which the domain uses for points in time.

use jiff::civil;
use jiff::tz::Offset;

use crate::dates::Date;

/// A `datetime` as the Python backend holds it: a civil date and time, and a UTC
/// offset when the value is aware.
///
/// Python keeps both kinds side by side. The database stores them as ISO 8601 text,
/// which `datetime.fromisoformat` reads back as aware with a fixed offset when the
/// text has one and as naive otherwise; values from the clock
/// (`datetime.now(tzlocal())`) are aware.
#[derive(Debug, Clone, Copy)]
pub struct DateTime {
    civil: civil::DateTime,
    offset: Option<Offset>,
}

impl DateTime {
    /// A naive value, without an offset.
    pub const fn naive(civil: civil::DateTime) -> Self {
        DateTime {
            civil,
            offset: None,
        }
    }

    /// An aware value: `civil` is the local time at `offset` from UTC.
    pub const fn aware(civil: civil::DateTime, offset: Offset) -> Self {
        DateTime {
            civil,
            offset: Some(offset),
        }
    }

    /// The date and time as written, in the value's own offset.
    pub const fn civil(self) -> civil::DateTime {
        self.civil
    }

    /// The offset from UTC, `None` for a naive value.
    pub const fn offset(self) -> Option<Offset> {
        self.offset
    }

    /// `datetime.date()`: the calendar date as written, in the value's own offset.
    pub fn date(self) -> Date {
        self.civil.date()
    }
}

impl PartialEq for DateTime {
    /// Python's `==`: two naive values compare their fields, two aware values the
    /// instants they denote, and a naive value never equals an aware one.
    fn eq(&self, other: &Self) -> bool {
        match (self.offset, other.offset) {
            (None, None) => self.civil == other.civil,
            (Some(own), Some(theirs)) => {
                match (
                    own.to_timestamp(self.civil),
                    theirs.to_timestamp(other.civil),
                ) {
                    (Ok(own), Ok(theirs)) => own == theirs,
                    // Only at the ends of the calendar; the same fields are the same instant.
                    _ => own == theirs && self.civil == other.civil,
                }
            }
            _ => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn civil(value: &str) -> civil::DateTime {
        value.parse().unwrap()
    }

    fn offset(hours: i8) -> Offset {
        Offset::constant(hours)
    }

    #[test]
    fn compares_like_python() {
        let naive = DateTime::naive(civil("2024-03-01T10:00:00"));
        let utc = DateTime::aware(civil("2024-03-01T10:00:00"), offset(0));
        let cet = DateTime::aware(civil("2024-03-01T11:00:00"), offset(1));
        assert_eq!(naive, DateTime::naive(civil("2024-03-01T10:00:00")));
        assert_eq!(utc, cet);
        assert_ne!(naive, utc);
        assert_ne!(
            utc,
            DateTime::aware(civil("2024-03-01T11:00:00"), offset(0))
        );
    }

    #[test]
    fn date_is_taken_in_the_value_s_own_offset() {
        let late = DateTime::aware(civil("2024-03-01T23:30:00"), offset(-5));
        assert_eq!(late.date(), "2024-03-01".parse::<Date>().unwrap());
        assert_eq!(late.offset(), Some(offset(-5)));
        assert_eq!(DateTime::naive(civil("2024-03-01T23:30:00")).offset(), None);
    }
}
