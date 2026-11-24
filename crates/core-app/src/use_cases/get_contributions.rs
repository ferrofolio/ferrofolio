// SPDX-License-Identifier: AGPL-3.0-only

//! Port of `finanze/application/use_cases/get_contributions.py`.

use core_domain::auto_contributions::{
    ContributionFrequency, ContributionQueryRequest, EntityContributions, PeriodicContribution,
};
use core_domain::dates::{Date, add_days, add_months};

use crate::ports::{AutoContributionsPort, Clock, EntityPort, ReadError};

/// `GetContributions`: the periodic contributions with their next due date.
#[derive(Debug, Clone, Copy)]
pub struct GetContributions<'a> {
    contributions: &'a dyn AutoContributionsPort,
    entities: &'a dyn EntityPort,
    clock: &'a dyn Clock,
}

impl<'a> GetContributions<'a> {
    pub fn new(
        contributions: &'a dyn AutoContributionsPort,
        entities: &'a dyn EntityPort,
        clock: &'a dyn Clock,
    ) -> Self {
        GetContributions {
            contributions,
            entities,
            clock,
        }
    }

    /// The contributions of the query, with the next date of each one added. A
    /// date arithmetic that leaves Python's range is left to Python, as in the
    /// other read paths.
    pub fn execute(
        &self,
        query: &ContributionQueryRequest,
    ) -> Result<EntityContributions, ReadError> {
        let disabled = self.entities.get_disabled_entities()?;
        let mut filtered = query.clone();
        filtered.excluded_entities = Some(disabled);
        let mut contributions = self.contributions.get_all_grouped_by_entity(&filtered)?;
        let today = self.clock.today();
        for periodic in contributions
            .values_mut()
            .flat_map(|entry| entry.periodic.iter_mut())
        {
            periodic.next_date = next_date(periodic, today).map_err(|_| {
                ReadError::Unreadable("the next date leaves the calendar".to_owned())
            })?;
        }
        Ok(contributions)
    }
}

/// `_next_contribution_date` of the use case, with today's date supplied. `None`
/// means no date is due: an inactive contribution, one that starts later, or one
/// whose next date is after its end.
pub fn next_date(
    contribution: &PeriodicContribution,
    today: Date,
) -> Result<Option<Date>, Overflow> {
    if !contribution.active {
        return Ok(None);
    }
    let since = contribution.since;
    if since > today {
        return Ok(Some(since));
    }
    let next = match contribution.frequency {
        ContributionFrequency::Weekly => periods_after(since, today, 7)?,
        ContributionFrequency::Biweekly => periods_after(since, today, 14)?,
        ContributionFrequency::Monthly => step_past(since, today, |date| add_months(date, 1))?,
        ContributionFrequency::Bimonthly => step_past(since, today, |date| add_months(date, 2))?,
        ContributionFrequency::EveryFourMonths => {
            step_past(since, today, |date| add_months(date, 4))?
        }
        ContributionFrequency::Quarterly => step_past(since, today, |date| add_months(date, 3))?,
        ContributionFrequency::Semiannual => step_past(since, today, |date| add_months(date, 6))?,
        ContributionFrequency::Yearly => step_past(since, today, |date| add_months(date, 12))?,
    };
    match contribution.until {
        Some(until) if next > until => Ok(None),
        _ => Ok(Some(next)),
    }
}

/// Python's date arithmetic left the range of `date`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Overflow;

/// The weekly and biweekly branches: the first date a whole number of `days`
/// periods after `since` that lies after `today`, for `since <= today`.
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

#[cfg(test)]
mod tests {
    use super::*;
    use core_domain::Dezimal;
    use core_domain::entity::{Entity, EntityOrigin, EntityType};
    use core_domain::fetch_record::DataSource;

    /// The date Python takes from the clock in the recorded fixtures.
    fn contribution(
        since: &str,
        frequency: ContributionFrequency,
        active: bool,
    ) -> PeriodicContribution {
        PeriodicContribution {
            id: "550e8400-e29b-41d4-a716-446655440000".parse().unwrap(),
            alias: None,
            target: "t".to_owned(),
            target_name: "Target".to_owned(),
            target_type: core_domain::auto_contributions::ContributionTargetType::Fund,
            target_subtype: None,
            amount: Dezimal::from_python_str("1").unwrap(),
            currency: "EUR".to_owned(),
            since: since.parse().unwrap(),
            until: None,
            frequency,
            active,
            source: DataSource::Real,
            next_date: None,
            entity: Entity {
                id: None,
                name: "Bank".to_owned(),
                natural_id: None,
                r#type: EntityType::FinancialInstitution,
                origin: EntityOrigin::Native,
                icon_url: None,
            },
            entity_account_id: None,
        }
    }

    #[test]
    fn weekly_and_monthly_dates_follow_python() {
        let today = "2026-11-18".parse().unwrap();
        assert_eq!(
            next_date(
                &contribution("2026-11-17", ContributionFrequency::Weekly, true),
                today
            )
            .unwrap(),
            Some("2026-11-24".parse().unwrap())
        );
        assert_eq!(
            next_date(
                &contribution("2026-11-18", ContributionFrequency::Weekly, true),
                today
            )
            .unwrap(),
            Some("2026-11-25".parse().unwrap())
        );
        assert_eq!(
            next_date(
                &contribution("2026-01-31", ContributionFrequency::Monthly, true),
                today
            )
            .unwrap(),
            Some("2026-11-28".parse().unwrap())
        );
        assert_eq!(
            next_date(
                &contribution("2027-01-01", ContributionFrequency::Monthly, true),
                today
            )
            .unwrap(),
            Some("2027-01-01".parse().unwrap())
        );
        assert_eq!(
            next_date(
                &contribution("2026-01-01", ContributionFrequency::Monthly, false),
                today
            )
            .unwrap(),
            None
        );
    }
}
