// SPDX-License-Identifier: AGPL-3.0-only

//! Calendar arithmetic with the semantics of Python's `dateutil.relativedelta`,
//! which the Python backend uses for schedules.

use jiff::Span;
pub use jiff::civil::Date;

/// `date + relativedelta(months=months)`: moves by calendar months and clamps the
/// day to the end of the target month (31 January plus one month is 28 or 29 February).
///
/// Returns `None` outside the supported date range.
pub fn add_months(date: Date, months: i64) -> Option<Date> {
    if !in_python_range(&date) {
        return None;
    }
    let span = Span::new().try_months(months).ok()?;
    date.checked_add(span).ok().filter(in_python_range)
}

/// The last day of Python's calendar, `date.max`.
pub const CALENDAR_END: Date = jiff::civil::date(9999, 12, 31);

/// Python's `date` starts at 1 January of year 1; jiff goes further back.
fn in_python_range(date: &Date) -> bool {
    date.year() >= 1
}

/// `date + relativedelta(years=years)`.
pub fn add_years(date: Date, years: i64) -> Option<Date> {
    add_months(date, years.checked_mul(12)?)
}

/// `date + relativedelta(days=days)`.
pub fn add_days(date: Date, days: i64) -> Option<Date> {
    if !in_python_range(&date) {
        return None;
    }
    let span = Span::new().try_days(days).ok()?;
    date.checked_add(span).ok().filter(in_python_range)
}

/// The calendar distance `relativedelta(later, earlier)` for `later >= earlier`,
/// expressed as whole months plus remaining days.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MonthsAndDays {
    pub months: i64,
    pub days: i64,
}

/// `relativedelta(later, earlier)` for `later >= earlier`.
///
/// Follows dateutil: start from the difference of the year and month fields and step
/// back while `earlier` plus that many months lies after `later`. Returns `None` when
/// `later` is before `earlier` or a date leaves the supported range.
pub fn months_and_days_between(later: Date, earlier: Date) -> Option<MonthsAndDays> {
    if later < earlier || !in_python_range(&later) || !in_python_range(&earlier) {
        return None;
    }
    let mut months = (i64::from(later.year()) - i64::from(earlier.year())) * 12
        + (i64::from(later.month()) - i64::from(earlier.month()));
    let mut shifted = add_months(earlier, months)?;
    while later < shifted {
        months -= 1;
        shifted = add_months(earlier, months)?;
    }
    let days = i64::from(later.since(shifted).ok()?.get_days());
    Some(MonthsAndDays { months, days })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn date(value: &str) -> Date {
        value.parse().unwrap()
    }

    #[test]
    fn adding_months_clamps_to_month_end() {
        assert_eq!(add_months(date("2025-01-31"), 1), Some(date("2025-02-28")));
        assert_eq!(add_months(date("2024-01-31"), 1), Some(date("2024-02-29")));
        assert_eq!(add_months(date("2024-03-31"), -1), Some(date("2024-02-29")));
        assert_eq!(add_years(date("2024-02-29"), 1), Some(date("2025-02-28")));
        assert_eq!(add_days(date("2024-02-28"), 2), Some(date("2024-03-01")));
        assert_eq!(add_days(date("2024-03-01"), -1), Some(date("2024-02-29")));
        assert_eq!(add_years(date("9999-01-01"), 1), None);
        assert_eq!(add_days(date("0001-01-01"), -1), None);
        assert_eq!(add_months(date("0001-01-31"), -1), None);
        // A date before year 1 is outside Python's range even when the result is not.
        assert_eq!(add_days(date("0000-12-31"), 1), None);
        assert_eq!(add_months(date("0000-12-31"), 1), None);
        assert_eq!(
            months_and_days_between(date("0001-01-31"), date("0000-12-31")),
            None
        );
        assert_eq!(add_days(date("0001-01-02"), -1), Some(date("0001-01-01")));
    }

    #[test]
    fn months_between_follows_dateutil() {
        let between = |later, earlier| months_and_days_between(date(later), date(earlier)).unwrap();
        assert_eq!(
            between("2025-03-15", "2025-01-15"),
            MonthsAndDays { months: 2, days: 0 }
        );
        assert_eq!(
            between("2025-03-14", "2025-01-15"),
            MonthsAndDays {
                months: 1,
                days: 27
            }
        );
        // 31 January plus one month is 28 February, so a full month has passed.
        assert_eq!(
            between("2025-02-28", "2025-01-31"),
            MonthsAndDays { months: 1, days: 0 }
        );
        assert_eq!(
            between("2025-03-30", "2025-01-31"),
            MonthsAndDays {
                months: 1,
                days: 30
            }
        );
        assert_eq!(
            between("2027-01-01", "2025-01-01"),
            MonthsAndDays {
                months: 24,
                days: 0
            }
        );
        assert_eq!(
            between("2025-01-01", "2025-01-01"),
            MonthsAndDays { months: 0, days: 0 }
        );
        assert_eq!(
            months_and_days_between(date("2025-01-01"), date("2025-01-02")),
            None
        );
    }
}
