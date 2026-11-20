// SPDX-License-Identifier: AGPL-3.0-only

//! Port of `finanze/application/use_cases/get_periodic_flows.py`.

use core_domain::dates::{Date, add_days, add_months, add_years};
use core_domain::earnings_expenses::{FlowFrequency, PeriodicFlow};

use crate::ports::{Clock, PeriodicFlowPort, ReadError};

/// `get_next_date`: the next date a flow is due after `today`, `None` for a
/// disabled flow or one whose next date lies after its end. A flow that has not
/// started yet is next due on its start, whatever its end.
///
/// `Err` where Python's date arithmetic overflows, which it answers with `500`.
pub fn next_date(flow: &PeriodicFlow, today: Date) -> Result<Option<Date>, Overflow> {
    if !flow.enabled {
        return Ok(None);
    }
    let since = flow.since;
    if since > today {
        return Ok(Some(since));
    }
    let next = match flow.frequency {
        FlowFrequency::Daily => add_days(today, 1).ok_or(Overflow)?,
        FlowFrequency::Weekly => periods_after(since, today, 7)?,
        FlowFrequency::Biweekly => periods_after(since, today, 14)?,
        FlowFrequency::Semimonthly => periods_after(since, today, 15)?,
        FlowFrequency::Monthly => step_past(since, today, |date| add_months(date, 1))?,
        FlowFrequency::EveryTwoMonths => step_past(since, today, |date| add_months(date, 2))?,
        FlowFrequency::Quarterly => step_past(since, today, |date| add_months(date, 3))?,
        FlowFrequency::EveryFourMonths => step_past(since, today, |date| add_months(date, 4))?,
        FlowFrequency::Semiannually => step_past(since, today, |date| add_months(date, 6))?,
        FlowFrequency::Yearly => step_past(since, today, |date| add_years(date, 1))?,
    };
    match flow.until {
        Some(until) if next > until => Ok(None),
        _ => Ok(Some(next)),
    }
}

/// Python's date arithmetic left the range of `date`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Overflow;

/// The weekly, biweekly and semimonthly branches: the first date a whole number
/// of `days` periods after `since` that lies after `today`, for `since <= today`.
fn periods_after(since: Date, today: Date, days: i64) -> Result<Date, Overflow> {
    let passed = i64::from(since.until(today).map_err(|_| Overflow)?.get_days()) / days;
    let next = add_days(since, (passed + 1) * days).ok_or(Overflow)?;
    if next > today {
        return Ok(next);
    }
    // Unreachable for `since <= today`, kept as Python has it.
    add_days(since, (passed + 2) * days).ok_or(Overflow)
}

/// `while next_date <= today: next_date = next_date + delta`, starting at `since`.
/// Each step moves from the date the previous one reached, so a day clamped to the
/// end of a short month stays clamped.
fn step_past(
    since: Date,
    today: Date,
    step: impl Fn(Date) -> Option<Date>,
) -> Result<Date, Overflow> {
    let mut next = since;
    while next <= today {
        next = step(next).ok_or(Overflow)?;
    }
    Ok(next)
}

/// `GetPeriodicFlows`: every periodic flow with its next date.
#[derive(Debug, Clone, Copy)]
pub struct GetPeriodicFlows<'a> {
    flows: &'a dyn PeriodicFlowPort,
    clock: &'a dyn Clock,
}

impl<'a> GetPeriodicFlows<'a> {
    pub fn new(flows: &'a dyn PeriodicFlowPort, clock: &'a dyn Clock) -> Self {
        GetPeriodicFlows { flows, clock }
    }

    /// The flows in the port's order. A next date Python cannot compute is
    /// reported as unreadable, so that Python answers with its error.
    pub fn execute(&self) -> Result<Vec<PeriodicFlow>, ReadError> {
        let today = self.clock.today();
        let mut flows = self.flows.get_all()?;
        for flow in &mut flows {
            flow.next_date = next_date(flow, today)
                .map_err(|Overflow| ReadError::Unreadable("next date".to_owned()))?;
        }
        Ok(flows)
    }
}

#[cfg(test)]
mod tests {
    use core_domain::Dezimal;
    use core_domain::earnings_expenses::FlowType;

    use super::*;
    use crate::ports::FixedClock;

    fn date(value: &str) -> Date {
        value.parse().unwrap()
    }

    fn flow(frequency: FlowFrequency, since: &str, until: Option<&str>) -> PeriodicFlow {
        PeriodicFlow {
            id: "a".to_owned(),
            name: "Flow".to_owned(),
            amount: Dezimal::from_python_str("10").unwrap(),
            currency: "EUR".to_owned(),
            flow_type: FlowType::Expense,
            frequency,
            category: None,
            enabled: true,
            since: date(since),
            until: until.map(date),
            icon: None,
            linked: Some(false),
            real_estate_flow: None,
            next_date: None,
            max_amount: None,
            label_id: None,
        }
    }

    fn next(
        frequency: FlowFrequency,
        since: &str,
        until: Option<&str>,
        today: &str,
    ) -> Option<Date> {
        next_date(&flow(frequency, since, until), date(today)).unwrap()
    }

    #[test]
    fn monthly_steps_keep_a_clamped_day() {
        assert_eq!(
            next(FlowFrequency::Monthly, "2025-01-31", None, "2026-03-01"),
            Some(date("2026-03-28"))
        );
        assert_eq!(
            next(FlowFrequency::Yearly, "2024-02-29", None, "2026-03-01"),
            Some(date("2027-02-28"))
        );
    }

    #[test]
    fn day_counted_periods_start_at_since() {
        assert_eq!(
            next(FlowFrequency::Weekly, "2026-11-04", None, "2026-11-18"),
            Some(date("2026-11-25"))
        );
        assert_eq!(
            next(FlowFrequency::Semimonthly, "2026-11-03", None, "2026-11-18"),
            Some(date("2026-12-03"))
        );
        assert_eq!(
            next(FlowFrequency::Daily, "2020-01-01", None, "2026-11-18"),
            Some(date("2026-11-19"))
        );
    }

    #[test]
    fn a_future_start_ignores_the_end_and_a_late_next_date_is_none() {
        assert_eq!(
            next(
                FlowFrequency::Monthly,
                "2027-01-01",
                Some("2026-01-01"),
                "2026-11-18"
            ),
            Some(date("2027-01-01"))
        );
        assert_eq!(
            next(
                FlowFrequency::Monthly,
                "2026-01-20",
                Some("2026-11-19"),
                "2026-11-18"
            ),
            None
        );
        assert_eq!(
            next(
                FlowFrequency::Monthly,
                "2026-01-20",
                Some("2026-11-20"),
                "2026-11-18"
            ),
            Some(date("2026-11-20"))
        );
    }

    #[test]
    fn a_disabled_flow_has_no_next_date_and_overflow_is_an_error() {
        let mut disabled = flow(FlowFrequency::Daily, "2026-01-01", None);
        disabled.enabled = false;
        assert_eq!(next_date(&disabled, date("2026-11-18")), Ok(None));
        let last = flow(FlowFrequency::Daily, "2026-01-01", None);
        assert_eq!(next_date(&last, date("9999-12-31")), Err(Overflow));
    }

    #[derive(Debug)]
    struct Fixed(Vec<PeriodicFlow>);

    impl PeriodicFlowPort for Fixed {
        fn get_all(&self) -> Result<Vec<PeriodicFlow>, ReadError> {
            Ok(self.0.clone())
        }
    }

    #[test]
    fn fills_the_next_date_of_every_flow() {
        let port = Fixed(vec![flow(FlowFrequency::Quarterly, "2026-01-15", None)]);
        let clock = FixedClock(date("2026-11-18"));
        let flows = GetPeriodicFlows::new(&port, &clock).execute().unwrap();
        assert_eq!(flows[0].next_date, Some(date("2027-01-15")));
        let port = Fixed(vec![flow(FlowFrequency::Daily, "2026-01-15", None)]);
        let clock = FixedClock(date("9999-12-31"));
        assert!(matches!(
            GetPeriodicFlows::new(&port, &clock).execute(),
            Err(ReadError::Unreadable(_))
        ));
    }
}
