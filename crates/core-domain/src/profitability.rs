// SPDX-License-Identifier: AGPL-3.0-only

//! Simple interest over a term, from `finanze/domain/profitability.py`.

use jiff::Unit;

use crate::dates::Date;
use crate::datetime::DateTime;
use crate::dezimal::{ArithmeticError, Checked, Dezimal};

/// `DAYS_IN_YEAR`: interest accrues per day of a 365-day year.
pub const DAYS_IN_YEAR: Dezimal = Dezimal::from_scaled(365, 0);

/// The keyword arguments of `annualized_profitability`; each is `None` unless given.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct ProfitabilityTerms {
    pub interest_rate: Option<Dezimal>,
    pub start_dt: Option<DateTime>,
    pub maturity: Option<Date>,
    pub extended_maturity: Option<Date>,
    pub extended_interest_rate: Option<Dezimal>,
    pub late_interest_rate: Option<Dezimal>,
}

/// `annualized_profitability(...)`: the simple interest earned over the term, as a
/// fraction of the amount, rounded to four places.
///
/// The interest rate counts from the start to the maturity. Once `today` has reached
/// the maturity, an extension to a later maturity counts with its own rate, else
/// with the interest rate. A late interest rate counts the days after the last
/// maturity up to `today`, the date Python takes from the local clock. Without any
/// of these the result is zero. Fails like Python when the arithmetic leaves the
/// decimal range.
pub fn annualized_profitability(
    terms: &ProfitabilityTerms,
    today: Date,
) -> Result<Dezimal, ArithmeticError> {
    let mut total = Checked::from(Dezimal::ZERO);
    let mut has_interest = false;
    let mut add_interest = |rate: Option<Dezimal>, days: i64| {
        if let Some(rate) = rate
            && days > 0
        {
            total = total + rate * Dezimal::from(days) / DAYS_IN_YEAR;
            has_interest = true;
        }
    };

    let start_date = terms.start_dt.and_then(|start| start.date());

    if let (Some(rate), Some(start), Some(maturity)) =
        (terms.interest_rate, start_date, terms.maturity)
        && maturity > start
    {
        add_interest(Some(rate), days_between(maturity, start));
    }

    if let (Some(extended), Some(maturity)) = (terms.extended_maturity, terms.maturity)
        && extended > maturity
        && today >= maturity
    {
        add_interest(
            terms.extended_interest_rate.or(terms.interest_rate),
            days_between(extended, maturity),
        );
    }

    let lateness_anchor = terms.extended_maturity.or(terms.maturity);
    if let (Some(rate), Some(anchor)) = (terms.late_interest_rate, lateness_anchor)
        && today > anchor
    {
        add_interest(Some(rate), days_between(today, anchor));
    }

    if !has_interest {
        return Ok(Dezimal::ZERO);
    }
    total.value()?.round(4)
}

/// `(later - earlier).days` for `later` after `earlier`. Every span between two
/// dates of jiff's range fits a span of days, so the fallback is never taken.
fn days_between(later: Date, earlier: Date) -> i64 {
    later
        .since((Unit::Day, earlier))
        .map_or(0, |span| i64::from(span.get_days()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d(value: &str) -> Dezimal {
        Dezimal::from_python_str(value).unwrap()
    }

    fn date(value: &str) -> Date {
        value.parse().unwrap()
    }

    fn start(value: &str) -> Option<DateTime> {
        Some(DateTime::from_isoformat(value).unwrap())
    }

    #[test]
    fn interest_over_the_term() {
        let terms = ProfitabilityTerms {
            interest_rate: Some(d("0.1")),
            start_dt: start("2024-01-01T09:30:00"),
            maturity: Some(date("2024-12-31")),
            ..ProfitabilityTerms::default()
        };
        // 365 days at 10 %.
        assert_eq!(
            annualized_profitability(&terms, date("2024-06-01"))
                .unwrap()
                .to_string(),
            "0.1000"
        );
    }

    #[test]
    fn extension_and_lateness_count_once_due() {
        let terms = ProfitabilityTerms {
            interest_rate: Some(d("0.073")),
            start_dt: start("2024-01-01T00:00:00"),
            maturity: Some(date("2024-01-11")),
            extended_maturity: Some(date("2024-01-21")),
            late_interest_rate: Some(d("0.365")),
            ..ProfitabilityTerms::default()
        };
        // 10 days at 7.3 % before the maturity: 0.002.
        assert_eq!(
            annualized_profitability(&terms, date("2024-01-05")).unwrap(),
            d("0.002")
        );
        // Plus 10 extended days at 7.3 % and 4 late days at 36.5 %.
        assert_eq!(
            annualized_profitability(&terms, date("2024-01-25")).unwrap(),
            d("0.008")
        );
    }

    #[test]
    fn nothing_to_count_is_zero() {
        assert_eq!(
            annualized_profitability(&ProfitabilityTerms::default(), date("2024-01-01")),
            Ok(Dezimal::ZERO)
        );
    }
}
