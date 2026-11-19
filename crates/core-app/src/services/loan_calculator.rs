// SPDX-License-Identifier: AGPL-3.0-only

//! Port of `finanze/infrastructure/calculations/loan_calculator.py`.
//!
//! Supports FIXED, VARIABLE and MIXED interest:
//!
//! - Rates are annual nominal (0.03 is 3 % APR); the period rate is the annual rate
//!   divided by the payments per year.
//! - VARIABLE, and MIXED after the fixed period, pay `interest_rate + euribor_rate`.
//! - MIXED during the fixed period pays `fixed_interest_rate`, or `interest_rate`
//!   when no fixed rate is given.
//! - The loan is disbursed on `start`; installment `i` is due `i` periods later,
//!   the last one on or before `end` (`Schedule`). The installments due before
//!   today are paid; the one due today or next is the current installment. Once
//!   every installment is past, the last one stays the current one, and a loan
//!   whose outstanding comes from its amount or a tracking reference is repaid:
//!   nothing is outstanding and nothing is due. An outstanding given in the
//!   request is still owed then and due in full.
//! - Without `principal_outstanding` the outstanding amount is computed from the
//!   loan amount: in closed form for a fixed rate, otherwise by simulating each paid
//!   installment with the rate of its period.
//!
//! The arithmetic, the rounding and the iteration order follow the Python code
//! exactly.

use std::sync::Arc;

use core_domain::Dezimal;
use core_domain::dates::{Date, add_days, add_months, add_years, months_and_days_between};
use core_domain::exception::{CalculationError, CalculationInputError, MissingFieldsError};
use core_domain::global_position::{InstallmentFrequency, InterestType};
use core_domain::loan_calculator::{LoanCalculationParams, LoanCalculationResult};

use crate::ports::{Clock, LoanCalculatorPort};

/// The loan calculator.
#[derive(Debug, Clone)]
pub struct LoanCalculator {
    clock: Arc<dyn Clock>,
}

/// One payment period, `relativedelta` in Python.
#[derive(Debug, Clone, Copy)]
enum PeriodStep {
    Days(i64),
    Months(i64),
}

impl PeriodStep {
    fn of(frequency: InstallmentFrequency) -> Self {
        match frequency {
            InstallmentFrequency::Weekly => PeriodStep::Days(7),
            InstallmentFrequency::Biweekly => PeriodStep::Days(14),
            InstallmentFrequency::Semimonthly => PeriodStep::Days(15),
            InstallmentFrequency::Monthly => PeriodStep::Months(1),
            InstallmentFrequency::Bimonthly => PeriodStep::Months(2),
            InstallmentFrequency::Quarterly => PeriodStep::Months(3),
            InstallmentFrequency::Semiannual => PeriodStep::Months(6),
            InstallmentFrequency::Yearly => PeriodStep::Months(12),
        }
    }
}

/// The installments of a loan disbursed on `start` and due on `end`.
///
/// Installment `i` (1, 2, ...) is due `i` periods after `start`, counted from
/// `start` each time, so a schedule starting on 31 January is due on 28 or 29
/// February and then on 31 March. The last installment is the last one on or
/// before `end`; a loan that ends before its first period is over has one
/// installment, on `end`.
#[derive(Debug, Clone, Copy)]
struct Schedule {
    start: Date,
    end: Date,
    frequency: InstallmentFrequency,
    /// The number of installments.
    count: i64,
}

impl Schedule {
    fn new(
        start: Date,
        end: Date,
        frequency: InstallmentFrequency,
    ) -> Result<Self, CalculationError> {
        let mut schedule = Schedule {
            start,
            end,
            frequency,
            count: 1,
        };
        schedule.count = schedule.due_until(end)?.max(1);
        Ok(schedule)
    }

    /// The due date of installment `index` (1 to `count`).
    fn date(self, index: i64) -> Date {
        self.shifted(index)
            .map_or(self.end, |date| date.min(self.end))
    }

    /// The day the period of installment `index` begins: `start` for the first, the
    /// due date of the one before for the others.
    fn period_start(self, index: i64) -> Date {
        if index == 1 {
            self.start
        } else {
            self.date(index - 1)
        }
    }

    /// How many installments are due before `today`.
    fn paid(self, today: Date) -> Result<i64, CalculationError> {
        if today > self.date(self.count) {
            return Ok(self.count);
        }
        if today <= self.start {
            return Ok(0);
        }
        let yesterday = add_days(today, -1).ok_or(CalculationError::DateOutOfRange)?;
        self.due_until(yesterday)
    }

    /// The installment due on or after `today`; once all are past, the last one.
    fn next_index(self, today: Date) -> Result<i64, CalculationError> {
        Ok((self.paid(today)? + 1).min(self.count))
    }

    /// `start` plus that many periods, or `None` beyond the end of the calendar.
    fn shifted(self, periods: i64) -> Option<Date> {
        match PeriodStep::of(self.frequency) {
            PeriodStep::Days(days) => add_days(self.start, periods.checked_mul(days)?),
            PeriodStep::Months(months) => add_months(self.start, periods.checked_mul(months)?),
        }
    }

    /// How many installments are due on or before `when`.
    fn due_until(self, when: Date) -> Result<i64, CalculationError> {
        if when <= self.start {
            return Ok(0);
        }
        match PeriodStep::of(self.frequency) {
            PeriodStep::Days(days) => {
                let elapsed = i64::from(
                    when.since(self.start)
                        .map_err(|_| CalculationError::DateOutOfRange)?
                        .get_days(),
                );
                Ok(elapsed / days)
            }
            PeriodStep::Months(months) => {
                let elapsed = months_and_days_between(when, self.start)
                    .ok_or(CalculationError::DateOutOfRange)?
                    .months;
                let mut count = elapsed / months;
                // A month end shifted into a shorter month can still be due by `when`.
                while self.shifted(count + 1).is_some_and(|date| date <= when) {
                    count += 1;
                }
                Ok(count)
            }
        }
    }
}

impl LoanCalculator {
    pub fn new(clock: Arc<dyn Clock>) -> Self {
        LoanCalculator { clock }
    }

    fn calculate_at(
        &self,
        params: &LoanCalculationParams,
        today: Date,
    ) -> Result<LoanCalculationResult, CalculationError> {
        let schedule = validate(params)?;
        let p = params;
        let frequency = schedule.frequency;

        let next_index = schedule.next_index(today)?;
        // An installment pays interest for its period at the rate in force when the
        // period began, here as in the simulation of the paid installments.
        let current_annual_rate = annual_rate_at(p, schedule, schedule.period_start(next_index))?;
        let payments_per_year = Dezimal::from(i64::from(frequency.payments_per_year()));
        let period_rate = (current_annual_rate / payments_per_year).value()?;

        let next_installment = schedule.date(next_index);
        let installments = schedule.count;
        // Installments due before today, the last one included once it is past.
        let paid = schedule.paid(today)?;
        if paid == installments {
            return after_the_last_installment(p, period_rate, next_installment);
        }

        // Fixed rate with a known original amount: the constant payment of the
        // original schedule.
        if p.interest_type == InterestType::Fixed
            && let Some(loan_amount) = p.loan_amount
        {
            let payment = amortizing_payment(loan_amount, period_rate, installments)?;

            let outstanding = match (p.tracking_ref_outstanding, p.tracking_ref_date) {
                (Some(reference_outstanding), Some(reference_date)) => {
                    // Tracked loan: the reference outstanding is the balance after the
                    // installments due before the reference date.
                    let reference_paid = schedule.paid(reference_date)?;
                    remaining_balance(
                        reference_outstanding,
                        period_rate,
                        installments - reference_paid,
                        (paid - reference_paid).max(0),
                        Some(payment),
                    )?
                }
                _ => match p.principal_outstanding {
                    Some(outstanding) => outstanding,
                    None => remaining_balance(
                        loan_amount,
                        period_rate,
                        installments,
                        paid,
                        Some(payment),
                    )?,
                },
            };

            let interest_part = (outstanding * period_rate).value()?.round_cents()?;
            return Ok(LoanCalculationResult {
                current_installment_payment: Some(payment.round_cents()?),
                current_installment_interests: Some(interest_part),
                principal_outstanding: Some(outstanding.round_cents()?),
                installment_date: Some(next_installment),
            });
        }

        // Installments still to pay, the current one included.
        let remaining_periods = installments - paid;

        // Fixed rate with an outstanding amount but no original amount: recover the
        // original payment from the remaining balance.
        if p.interest_type == InterestType::Fixed
            && p.loan_amount.is_none()
            && let Some(remaining) = p.principal_outstanding
        {
            let r = period_rate;

            if r.is_zero() {
                // Without interest the outstanding is spread over the installments left.
                let payment = (remaining / Dezimal::from(remaining_periods))
                    .value()?
                    .round_cents()?;
                return Ok(LoanCalculationResult {
                    current_installment_payment: Some(payment),
                    current_installment_interests: Some(Dezimal::ZERO),
                    principal_outstanding: Some(remaining),
                    installment_date: Some(next_installment),
                });
            }

            let one = Dezimal::ONE;
            let discount = (one - (one + r).powi(-installments)).value()?;
            let factor = (one + r).powi(paid).value()?;
            let coefficient = (factor - (factor - one) / discount).value()?;

            if coefficient.is_zero() {
                // Fall back to the general formula over the installments left.
                let denominator = (one - (one + r).powi(-remaining_periods)).value()?;
                let payment = if denominator.is_zero() {
                    remaining
                } else {
                    (remaining * r / denominator).value()?
                };
                return Ok(LoanCalculationResult {
                    current_installment_payment: Some(payment.round_cents()?),
                    current_installment_interests: Some((remaining * r).value()?.round_cents()?),
                    principal_outstanding: Some(remaining),
                    installment_date: Some(next_installment),
                });
            }

            let original_amount = (remaining / coefficient).value()?;
            let payment = (original_amount * r / discount).value()?;
            return Ok(LoanCalculationResult {
                current_installment_payment: Some(payment.round_cents()?),
                current_installment_interests: Some((remaining * r).value()?.round_cents()?),
                principal_outstanding: Some(remaining),
                installment_date: Some(next_installment),
            });
        }

        // General path (VARIABLE or MIXED): the outstanding is given or simulated
        // over the paid installments, and the current payment spreads it over the
        // installments left.
        let outstanding = match p.principal_outstanding {
            Some(outstanding) => outstanding,
            None => outstanding_from_start(p, schedule, paid)?,
        };

        if period_rate.is_zero() {
            let payment = (outstanding / Dezimal::from(remaining_periods))
                .value()?
                .round_cents()?;
            return Ok(LoanCalculationResult {
                current_installment_payment: Some(payment),
                current_installment_interests: Some(Dezimal::ZERO),
                principal_outstanding: Some(outstanding),
                installment_date: Some(next_installment),
            });
        }

        // A = P * r / (1 - (1 + r)^-n)
        let one = Dezimal::ONE;
        let r = period_rate;
        let denominator = (one - (one + r).powi(-remaining_periods)).value()?;
        let payment = if denominator.is_zero() {
            outstanding
        } else {
            (outstanding * r / denominator).value()?
        };

        Ok(LoanCalculationResult {
            current_installment_payment: Some(payment.round_cents()?),
            current_installment_interests: Some(
                (outstanding * period_rate).value()?.round_cents()?,
            ),
            principal_outstanding: Some(outstanding),
            installment_date: Some(next_installment),
        })
    }
}

impl LoanCalculatorPort for LoanCalculator {
    fn calculate(
        &self,
        params: &LoanCalculationParams,
    ) -> Result<LoanCalculationResult, CalculationError> {
        self.calculate_at(params, self.clock.today())
    }
}

/// A loan whose installments are all past. Computed from its amount or a tracking
/// reference, it is repaid; an outstanding given in the request is still owed and
/// due in full, with the last period's interest.
fn after_the_last_installment(
    p: &LoanCalculationParams,
    period_rate: Dezimal,
    last_date: Date,
) -> Result<LoanCalculationResult, CalculationError> {
    let tracked = p.interest_type == InterestType::Fixed
        && p.loan_amount.is_some()
        && p.tracking_ref_outstanding.is_some()
        && p.tracking_ref_date.is_some();
    let outstanding = if tracked {
        None
    } else {
        p.principal_outstanding
    };
    let Some(outstanding) = outstanding else {
        return Ok(LoanCalculationResult {
            current_installment_payment: Some(Dezimal::ZERO),
            current_installment_interests: Some(Dezimal::ZERO),
            principal_outstanding: Some(Dezimal::ZERO),
            installment_date: Some(last_date),
        });
    };
    Ok(LoanCalculationResult {
        current_installment_payment: Some(
            (outstanding * (Dezimal::ONE + period_rate))
                .value()?
                .round_cents()?,
        ),
        current_installment_interests: Some((outstanding * period_rate).value()?.round_cents()?),
        principal_outstanding: Some(outstanding),
        installment_date: Some(last_date),
    })
}

fn validate(params: &LoanCalculationParams) -> Result<Schedule, CalculationError> {
    let mut missing: Vec<&str> = Vec::new();
    if params.loan_amount.is_none() && params.principal_outstanding.is_none() {
        missing.push("loan_amount | principal_outstanding (at least one)");
    }
    if params.start.is_none() {
        missing.push("start");
    }
    if params.end.is_none() {
        missing.push("end");
    }
    if matches!(
        params.interest_type,
        InterestType::Variable | InterestType::Mixed
    ) && params.euribor_rate.is_none()
    {
        missing.push("euribor_rate (required for VARIABLE/MIXED)");
    }
    if params.interest_type == InterestType::Mixed && params.fixed_years.unwrap_or(0) == 0 {
        missing.push("fixed_years (required for MIXED)");
    }

    match (params.start, params.end) {
        (Some(start), Some(end)) if missing.is_empty() => {
            if end < start {
                return Err(CalculationInputError::new("end must not be before start").into());
            }
            if params.interest_type == InterestType::Mixed
                && params.fixed_years.is_some_and(|years| years < 0)
            {
                return Err(CalculationInputError::new("fixed_years must not be negative").into());
            }
            Schedule::new(start, end, params.installment_frequency)
        }
        _ => Err(MissingFieldsError::new(missing).into()),
    }
}

/// The annual rate that applies on `when`.
fn annual_rate_at(
    p: &LoanCalculationParams,
    schedule: Schedule,
    when: Date,
) -> Result<Dezimal, CalculationError> {
    let base = p.interest_rate;
    let floating = || -> Result<Dezimal, CalculationError> {
        let euribor = p.euribor_rate.ok_or_else(|| {
            MissingFieldsError::new(["euribor_rate (required for VARIABLE/MIXED)"])
        })?;
        Ok((base + euribor).value()?)
    };
    match p.interest_type {
        InterestType::Fixed => Ok(base),
        InterestType::Variable => floating(),
        InterestType::Mixed => {
            let fixed_years = p
                .fixed_years
                .ok_or_else(|| MissingFieldsError::new(["fixed_years (required for MIXED)"]))?;
            // A fixed period beyond the end of the calendar never ends.
            match add_years(schedule.start, fixed_years) {
                Some(fixed_end) if when >= fixed_end => floating(),
                _ => Ok(p.fixed_interest_rate.unwrap_or(base)),
            }
        }
    }
}

/// The outstanding after the `paid` first installments, each computed with the rate
/// in force when its period began.
fn outstanding_from_start(
    p: &LoanCalculationParams,
    schedule: Schedule,
    paid: i64,
) -> Result<Dezimal, CalculationError> {
    let principal = p.loan_amount.ok_or_else(|| {
        MissingFieldsError::new(["loan_amount | principal_outstanding (at least one)"])
    })?;
    let payments_per_year = Dezimal::from(i64::from(schedule.frequency.payments_per_year()));
    let one = Dezimal::ONE;

    let mut outstanding = principal;
    for index in 1..=paid {
        let annual_rate = annual_rate_at(p, schedule, schedule.period_start(index))?;
        let period_rate = (annual_rate / payments_per_year).value()?;
        // The installments left, this one and the last one included.
        let remaining_periods = schedule.count - index + 1;

        let (payment, interest) = if period_rate.is_zero() {
            let payment = (outstanding / Dezimal::from(remaining_periods))
                .value()?
                .round_cents()?;
            (payment, Dezimal::ZERO)
        } else {
            let denominator = (one - (one + period_rate).powi(-remaining_periods)).value()?;
            let payment = (outstanding * period_rate / denominator)
                .value()?
                .round_cents()?;
            let interest = (outstanding * period_rate).value()?.round_cents()?;
            (payment, interest)
        };

        let principal_paid = (payment - interest).value()?;
        outstanding = Dezimal::ZERO.max((outstanding - principal_paid).value()?);
        if outstanding.is_zero() {
            break;
        }
    }
    Ok(outstanding)
}

/// Constant payment of an amortising loan over `periods` periods.
fn amortizing_payment(
    principal: Dezimal,
    rate: Dezimal,
    periods: i64,
) -> Result<Dezimal, CalculationError> {
    if periods <= 0 {
        return Ok(principal);
    }
    if rate.is_zero() {
        return Ok((principal / Dezimal::from(periods)).value()?);
    }
    let one = Dezimal::ONE;
    let denominator = (one - (one + rate).powi(-periods)).value()?;
    Ok((principal * rate / denominator).value()?)
}

/// Remaining balance after `paid` payments of an amortising loan:
/// `RB_k = P(1+r)^k - A((1+r)^k - 1)/r`.
fn remaining_balance(
    principal: Dezimal,
    rate: Dezimal,
    periods: i64,
    paid: i64,
    payment: Option<Dezimal>,
) -> Result<Dezimal, CalculationError> {
    if paid <= 0 {
        return Ok(principal);
    }
    if paid >= periods {
        return Ok(Dezimal::ZERO);
    }
    if rate.is_zero() {
        let payment = match payment {
            Some(payment) => payment,
            None => (principal / Dezimal::from(periods)).value()?,
        };
        let remaining = (principal - payment * Dezimal::from(paid)).value()?;
        return Ok(Dezimal::ZERO.max(remaining));
    }
    let payment = match payment {
        Some(payment) => payment,
        None => amortizing_payment(principal, rate, periods)?,
    };
    let one = Dezimal::ONE;
    let factor = (one + rate).powi(paid).value()?;
    let remaining = (principal * factor - payment * ((factor - one) / rate)).value()?;
    Ok(Dezimal::ZERO.max(remaining))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ports::FixedClock;

    fn date(value: &str) -> Date {
        value.parse().unwrap()
    }

    fn d(value: &str) -> Dezimal {
        Dezimal::from_python_str(value).unwrap()
    }

    fn calculator(today: &str) -> LoanCalculator {
        LoanCalculator::new(Arc::new(FixedClock(date(today))))
    }

    fn fixed_loan() -> LoanCalculationParams {
        LoanCalculationParams {
            loan_amount: Some(d("200000")),
            interest_rate: d("0.03"),
            interest_type: InterestType::Fixed,
            euribor_rate: None,
            fixed_years: None,
            start: Some(date("2020-01-15")),
            end: Some(date("2050-01-15")),
            principal_outstanding: None,
            fixed_interest_rate: None,
            installment_frequency: InstallmentFrequency::Monthly,
            tracking_ref_outstanding: None,
            tracking_ref_date: None,
        }
    }

    #[test]
    fn fixed_loan_has_the_annuity_payment() {
        let result = calculator("2026-11-18").calculate(&fixed_loan()).unwrap();
        // 200000 at 3 % over 360 months: 843.21 per month.
        assert_eq!(result.current_installment_payment, Some(d("843.21")));
        assert_eq!(result.installment_date, Some(date("2026-12-15")));
        let outstanding = result.principal_outstanding.unwrap();
        assert!(outstanding < d("200000") && outstanding > d("150000"));
    }

    #[test]
    fn reports_all_missing_fields_like_python() {
        let mut params = fixed_loan();
        params.loan_amount = None;
        params.interest_type = InterestType::Mixed;
        params.start = None;
        let error = calculator("2026-11-18").calculate(&params).unwrap_err();
        assert_eq!(
            error.to_string(),
            "Missing required fields: loan_amount | principal_outstanding (at least one), start, \
             euribor_rate (required for VARIABLE/MIXED), fixed_years (required for MIXED)"
        );
    }

    fn schedule(start: &str, end: &str, frequency: InstallmentFrequency) -> Schedule {
        Schedule::new(date(start), date(end), frequency).unwrap()
    }

    #[test]
    fn month_ends_do_not_drift() {
        let schedule = schedule("2025-01-31", "2026-01-31", InstallmentFrequency::Monthly);
        // 31 Jan -> 28 Feb -> 31 Mar -> 30 Apr, each counted from the start.
        assert_eq!(schedule.date(1), date("2025-02-28"));
        assert_eq!(schedule.date(2), date("2025-03-31"));
        assert_eq!(
            schedule.date(schedule.next_index(date("2025-04-01")).unwrap()),
            date("2025-04-30")
        );
        assert_eq!(schedule.count, 12);
    }

    #[test]
    fn the_first_installment_is_due_one_period_after_the_start() {
        let schedule = schedule("2025-01-15", "2025-12-15", InstallmentFrequency::Monthly);
        assert_eq!(schedule.next_index(date("2025-01-10")).unwrap(), 1);
        assert_eq!(schedule.next_index(date("2025-01-15")).unwrap(), 1);
        assert_eq!(schedule.date(1), date("2025-02-15"));
        // Due today: still the current installment.
        assert_eq!(schedule.next_index(date("2025-03-15")).unwrap(), 2);
        assert_eq!(schedule.next_index(date("2025-03-16")).unwrap(), 3);
        // After the last one it stays the current installment.
        assert_eq!(schedule.next_index(date("2026-06-01")).unwrap(), 11);
        assert_eq!(schedule.date(11), date("2025-12-15"));
    }

    #[test]
    fn counts_installments_up_to_the_end() {
        assert_eq!(
            schedule(
                "2025-01-01",
                "2025-02-01",
                InstallmentFrequency::Semimonthly
            )
            .count,
            2
        );
        assert_eq!(
            schedule("2025-01-15", "2025-10-14", InstallmentFrequency::Quarterly).count,
            2
        );
        // 31 January to 28 February is one installment.
        assert_eq!(
            schedule("2025-01-31", "2025-02-28", InstallmentFrequency::Monthly).count,
            1
        );
        // A loan ending before its first period is over has one installment, on `end`.
        let short = schedule("2025-01-15", "2025-02-01", InstallmentFrequency::Monthly);
        assert_eq!((short.count, short.date(1)), (1, date("2025-02-01")));
    }

    #[test]
    fn day_based_counts_match_stepping_the_schedule() {
        let from = date("2024-02-20");
        for frequency in [
            InstallmentFrequency::Weekly,
            InstallmentFrequency::Biweekly,
            InstallmentFrequency::Semimonthly,
        ] {
            let days = match PeriodStep::of(frequency) {
                PeriodStep::Days(days) => days,
                PeriodStep::Months(_) => unreachable!(),
            };
            let mut to = from;
            for _ in 0..400 {
                to = add_days(to, 1).unwrap();
                let mut stepped = 0;
                let mut cursor = from;
                while let Some(next) = add_days(cursor, days).filter(|next| *next <= to) {
                    cursor = next;
                    stepped += 1;
                }
                let schedule = Schedule::new(from, to, frequency).unwrap();
                assert_eq!(schedule.due_until(to).unwrap(), stepped);
            }
        }
        // A step past 31 December 9999 is not taken.
        assert_eq!(
            schedule("9999-12-20", "9999-12-31", InstallmentFrequency::Weekly).count,
            1
        );
        assert_eq!(
            schedule("9999-10-31", "9999-12-31", InstallmentFrequency::Monthly).count,
            2
        );
    }

    #[test]
    fn a_simulated_constant_rate_matches_the_closed_form() {
        let calculator = calculator("2026-11-18");
        let fixed = calculator.calculate(&fixed_loan()).unwrap();
        let mut params = fixed_loan();
        params.interest_type = InterestType::Variable;
        params.euribor_rate = Some(Dezimal::ZERO);
        let variable = calculator.calculate(&params).unwrap();
        assert_eq!(variable.installment_date, fixed.installment_date);
        let difference = (variable.principal_outstanding.unwrap()
            - fixed.principal_outstanding.unwrap())
        .value()
        .unwrap()
        .abs();
        assert!(difference < Dezimal::ONE, "{difference}");
        assert_eq!(
            variable.current_installment_payment,
            fixed.current_installment_payment
        );
    }

    #[test]
    fn zero_rate_divides_the_outstanding_amount() {
        let mut params = fixed_loan();
        params.interest_rate = Dezimal::ZERO;
        params.interest_type = InterestType::Variable;
        params.euribor_rate = Some(Dezimal::ZERO);
        params.principal_outstanding = Some(d("1200"));
        params.end = Some(date("2027-12-15"));
        let result = calculator("2026-11-18").calculate(&params).unwrap();
        // Twelve periods from 15 December to the end, plus the end installment.
        assert_eq!(result.current_installment_payment, Some(d("92.31")));
        assert_eq!(result.current_installment_interests, Some(Dezimal::ZERO));
    }

    /// A loan of 1200 without interest from 15 January to 15 April 2025.
    fn short_loan(interest_type: InterestType) -> LoanCalculationParams {
        LoanCalculationParams {
            loan_amount: Some(d("1200")),
            interest_rate: Dezimal::ZERO,
            interest_type,
            euribor_rate: Some(Dezimal::ZERO),
            start: Some(date("2025-01-15")),
            end: Some(date("2025-04-15")),
            ..fixed_loan()
        }
    }

    #[test]
    fn a_loan_past_its_last_installment_is_repaid() {
        for interest_type in [InterestType::Fixed, InterestType::Variable] {
            let params = short_loan(interest_type);
            let last_due_today = calculator("2025-04-15").calculate(&params).unwrap();
            assert_eq!(last_due_today.principal_outstanding, Some(d("400")));
            assert_eq!(last_due_today.current_installment_payment, Some(d("400")));

            let all_past = calculator("2025-04-16").calculate(&params).unwrap();
            assert_eq!(all_past.installment_date, Some(date("2025-04-15")));
            assert_eq!(all_past.principal_outstanding, Some(Dezimal::ZERO));
            assert_eq!(all_past.current_installment_payment, Some(Dezimal::ZERO));
            assert_eq!(all_past.current_installment_interests, Some(Dezimal::ZERO));
        }
    }

    #[test]
    fn a_tracked_loan_past_its_last_installment_is_repaid() {
        let params = LoanCalculationParams {
            interest_rate: d("0.12"),
            tracking_ref_outstanding: Some(d("800")),
            tracking_ref_date: Some(date("2025-03-01")),
            ..short_loan(InterestType::Fixed)
        };
        let result = calculator("2025-05-01").calculate(&params).unwrap();
        assert_eq!(result.principal_outstanding, Some(Dezimal::ZERO));
        assert_eq!(result.current_installment_payment, Some(Dezimal::ZERO));
    }

    #[test]
    fn a_loan_tracked_from_after_its_last_installment_is_repaid() {
        let params = LoanCalculationParams {
            interest_rate: d("0.12"),
            tracking_ref_outstanding: Some(d("400")),
            tracking_ref_date: Some(date("2025-04-20")),
            ..short_loan(InterestType::Fixed)
        };
        let result = calculator("2025-05-01").calculate(&params).unwrap();
        assert_eq!(result.principal_outstanding, Some(Dezimal::ZERO));
        assert_eq!(result.current_installment_payment, Some(Dezimal::ZERO));
        assert_eq!(result.current_installment_interests, Some(Dezimal::ZERO));
    }

    #[test]
    fn a_given_outstanding_is_due_in_full_with_the_loan_amount() {
        // The regular installment would be 400; the 300 left are due at once.
        let params = LoanCalculationParams {
            principal_outstanding: Some(d("300")),
            ..short_loan(InterestType::Fixed)
        };
        let result = calculator("2025-05-01").calculate(&params).unwrap();
        assert_eq!(result.installment_date, Some(date("2025-04-15")));
        assert_eq!(result.principal_outstanding, Some(d("300")));
        assert_eq!(result.current_installment_payment, Some(d("300")));
    }

    #[test]
    fn a_given_outstanding_is_due_with_the_last_installment() {
        let params = LoanCalculationParams {
            loan_amount: None,
            principal_outstanding: Some(d("300")),
            ..short_loan(InterestType::Variable)
        };
        let result = calculator("2025-05-01").calculate(&params).unwrap();
        assert_eq!(result.installment_date, Some(date("2025-04-15")));
        assert_eq!(result.principal_outstanding, Some(d("300")));
        assert_eq!(result.current_installment_payment, Some(d("300")));
    }

    #[test]
    fn an_installment_pays_the_rate_of_its_period() {
        // The fixed period of 0 % ends on 2026-01-15; the installment due that day
        // still pays the fixed rate for its period, the next one the floating rate.
        let params = LoanCalculationParams {
            loan_amount: Some(d("12000")),
            interest_rate: d("0.12"),
            interest_type: InterestType::Mixed,
            euribor_rate: Some(Dezimal::ZERO),
            fixed_years: Some(1),
            fixed_interest_rate: Some(Dezimal::ZERO),
            start: Some(date("2025-01-15")),
            end: Some(date("2027-01-15")),
            ..fixed_loan()
        };
        let due_on_the_end = calculator("2026-01-15").calculate(&params).unwrap();
        assert_eq!(due_on_the_end.installment_date, Some(date("2026-01-15")));
        assert_eq!(due_on_the_end.principal_outstanding, Some(d("6500")));
        assert_eq!(
            due_on_the_end.current_installment_interests,
            Some(Dezimal::ZERO)
        );

        let due_after_the_end = calculator("2026-01-16").calculate(&params).unwrap();
        assert_eq!(due_after_the_end.installment_date, Some(date("2026-02-15")));
        assert_eq!(due_after_the_end.principal_outstanding, Some(d("6000")));
        assert_eq!(
            due_after_the_end.current_installment_interests,
            Some(d("60"))
        );
    }

    #[test]
    fn a_fixed_period_beyond_the_calendar_covers_its_last_day() {
        // The only installment's period begins on the last day of the calendar,
        // which the fixed period, ending beyond it, still covers.
        let params = LoanCalculationParams {
            loan_amount: Some(d("12000")),
            interest_rate: d("0.04"),
            interest_type: InterestType::Mixed,
            euribor_rate: Some(d("0.01")),
            fixed_years: Some(1),
            fixed_interest_rate: Some(d("0.02")),
            start: Some(date("9999-12-31")),
            end: Some(date("9999-12-31")),
            ..fixed_loan()
        };
        let result = calculator("2026-11-18").calculate(&params).unwrap();
        // 2 % a year is 20 a month on 12000; the floating 5 % would be 50.
        assert_eq!(result.current_installment_interests, Some(d("20")));
    }

    #[test]
    fn negative_full_period_rate_divides_by_zero_instead_of_panicking() {
        let mut params = fixed_loan();
        params.interest_rate = d("-12");
        let error = calculator("2026-11-18").calculate(&params).unwrap_err();
        assert!(matches!(error, CalculationError::Arithmetic(_)));
    }
}
