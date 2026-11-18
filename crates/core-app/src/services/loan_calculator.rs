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
//! - Without `principal_outstanding` the outstanding amount is simulated from the
//!   start date with the rates that applied over time.
//!
//! The arithmetic, the rounding and the iteration order follow the Python code
//! exactly, including that schedule dates advance one period at a time, so that a
//! schedule starting on 31 January continues on the 28th after February.

use std::sync::Arc;

use core_domain::Dezimal;
use core_domain::dates::{Date, add_days, add_months, add_years, months_and_days_between};
use core_domain::exception::{CalculationError, MissingFieldsError};
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

    fn advance(self, date: Date) -> Result<Date, CalculationError> {
        match self {
            PeriodStep::Days(days) => add_days(date, days),
            PeriodStep::Months(months) => add_months(date, months),
        }
        .ok_or(CalculationError::DateOutOfRange)
    }
}

/// The validated dates of a calculation.
#[derive(Debug, Clone, Copy)]
struct Schedule {
    start: Date,
    end: Date,
    frequency: InstallmentFrequency,
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

        let current_annual_rate = annual_rate_at(p, schedule, today)?;
        let payments_per_year = Dezimal::from(i64::from(frequency.payments_per_year()));
        let period_rate = (current_annual_rate / payments_per_year).value()?;

        let next_installment = next_installment_date(schedule, today)?;

        // Fixed rate with a known original amount: constant payment of the original
        // schedule, whose first payment is at `start` and last payment at `end`.
        if p.interest_type == InterestType::Fixed
            && let Some(loan_amount) = p.loan_amount
        {
            // Kept from Python: the annuity runs over the intervals from `start` to
            // `end`, one fewer than the installments of a schedule with payments on
            // both dates (docs/rust-backend.md, "Known issues of the Python reference", L5).
            let total_periods = count_periods(frequency, schedule.start, schedule.end)?.max(1);
            // Kept from Python: the installment due on the start date only counts
            // as paid once the next one is reached, so right after a payment the
            // balance still includes it (docs/rust-backend.md, "Known issues of the Python reference", L3).
            let payments_made =
                (count_periods(frequency, schedule.start, next_installment)? - 1).max(0);

            let payment = amortizing_payment(loan_amount, period_rate, total_periods)?;

            let outstanding = match (p.tracking_ref_outstanding, p.tracking_ref_date) {
                (Some(reference_outstanding), Some(reference_date)) => {
                    // Tracked loan: continue from the reference snapshot.
                    let remaining_from_reference =
                        count_periods(frequency, reference_date, schedule.end)?.max(1);
                    let paid_since_reference =
                        count_periods(frequency, reference_date, next_installment)?;
                    remaining_balance(
                        reference_outstanding,
                        period_rate,
                        remaining_from_reference,
                        paid_since_reference,
                        Some(payment),
                    )?
                }
                _ => match p.principal_outstanding {
                    Some(outstanding) => outstanding,
                    None => remaining_balance(
                        loan_amount,
                        period_rate,
                        total_periods,
                        payments_made,
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

        // Fixed rate with an outstanding amount but no original amount: recover the
        // original payment from the remaining balance.
        if p.interest_type == InterestType::Fixed
            && p.loan_amount.is_none()
            && let Some(remaining) = p.principal_outstanding
        {
            let total_periods = count_periods(frequency, schedule.start, schedule.end)?.max(1);
            let paid = (count_periods(frequency, schedule.start, next_installment)? - 1).max(0);
            let r = period_rate;

            if r.is_zero() {
                // Without interest the original amount cannot be recovered; spread the
                // remaining balance over the remaining periods.
                let remaining_periods =
                    count_periods(frequency, next_installment, schedule.end)?.max(1);
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
            let discount = (one - (one + r).powi(-total_periods)).value()?;
            let factor = (one + r).powi(paid).value()?;
            let coefficient = (factor - (factor - one) / discount).value()?;

            if coefficient.is_zero() {
                let remaining_periods =
                    count_periods(frequency, next_installment, schedule.end)?.max(1);
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

        // General path: VARIABLE or MIXED.
        let outstanding = match p.principal_outstanding {
            Some(outstanding) => outstanding,
            None => outstanding_from_start(p, schedule, today)?,
        };

        // The installments from the next one up to the one due on `end`. Python also
        // has a FIXED case with a given outstanding amount here, which the FIXED
        // branches above always answer first.
        let periods_to_end = count_periods(frequency, next_installment, schedule.end)?;
        let remaining_periods = (periods_to_end + 1).max(1);

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
        (Some(start), Some(end)) if missing.is_empty() => Ok(Schedule {
            start,
            end,
            frequency: params.installment_frequency,
        }),
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
            let fixed_end =
                add_years(schedule.start, fixed_years).ok_or(CalculationError::DateOutOfRange)?;
            if when < fixed_end {
                Ok(p.fixed_interest_rate.unwrap_or(base))
            } else {
                floating()
            }
        }
    }
}

const fn is_month_aligned(frequency: InstallmentFrequency) -> bool {
    !matches!(
        frequency,
        InstallmentFrequency::Weekly
            | InstallmentFrequency::Biweekly
            | InstallmentFrequency::Semimonthly
    )
}

/// Number of full periods of `frequency` between `from` and `to`.
fn count_periods(
    frequency: InstallmentFrequency,
    from: Date,
    to: Date,
) -> Result<i64, CalculationError> {
    if to <= from {
        return Ok(0);
    }
    // Kept from Python: monthly schedules are counted in calendar months, while the
    // schedule itself steps from date to date, so a start on the 31st can count one
    // installment less than the schedule produces (docs/rust-backend.md, "Known issues of the Python reference", L2).
    if is_month_aligned(frequency) {
        let full_months = months_and_days_between(to, from)
            .ok_or(CalculationError::DateOutOfRange)?
            .months;
        return Ok(full_months * i64::from(frequency.payments_per_year()) / 12);
    }
    // Python steps a cursor from `from` and counts the steps that stay on or before
    // `to`. With a fixed number of days per step that is a floor division, which
    // avoids walking a weekly schedule once per simulated payment.
    match PeriodStep::of(frequency) {
        PeriodStep::Days(days) if days > 0 => {
            let elapsed = i64::from(
                to.since(from)
                    .map_err(|_| CalculationError::DateOutOfRange)?
                    .get_days(),
            );
            let count = elapsed / days;
            // Python's cursor ends on the first step at or after `to`; like Python,
            // fail when that step leaves the calendar (docs/rust-backend.md, "Known
            // issues of the Python reference", L7).
            let steps = if elapsed % days == 0 {
                count
            } else {
                count + 1
            };
            add_days(from, steps * days).ok_or(CalculationError::DateOutOfRange)?;
            Ok(count)
        }
        step => {
            let mut count = 0;
            let mut cursor = from;
            while cursor < to {
                cursor = step.advance(cursor)?;
                if cursor <= to {
                    count += 1;
                }
            }
            Ok(count)
        }
    }
}

/// The next installment on or after `today`, aligned to `start` and capped at `end`.
fn next_installment_date(schedule: Schedule, today: Date) -> Result<Date, CalculationError> {
    // Kept from Python: an `end` before `start` is not rejected, and the cap does
    // not apply before the start (docs/rust-backend.md, "Known issues of the Python reference", L6).
    if today <= schedule.start {
        return Ok(schedule.start);
    }
    let step = PeriodStep::of(schedule.frequency);
    let mut candidate = schedule.start;
    while candidate < today && candidate < schedule.end {
        candidate = step.advance(candidate)?;
    }
    Ok(candidate.min(schedule.end))
}

/// Simulates the amortisation from `start` up to the last payment before `today`.
fn outstanding_from_start(
    p: &LoanCalculationParams,
    schedule: Schedule,
    today: Date,
) -> Result<Dezimal, CalculationError> {
    let principal = p.loan_amount.ok_or_else(|| {
        MissingFieldsError::new(["loan_amount | principal_outstanding (at least one)"])
    })?;
    if today <= schedule.start {
        return Ok(principal);
    }

    let frequency = schedule.frequency;
    let payments_per_year = Dezimal::from(i64::from(frequency.payments_per_year()));
    let step = PeriodStep::of(frequency);
    let one = Dezimal::ONE;

    let mut outstanding = principal;
    let mut current = schedule.start;
    while current < today && current < schedule.end {
        let annual_rate = annual_rate_at(p, schedule, current)?;
        let period_rate = (annual_rate / payments_per_year).value()?;
        // Kept from Python: unlike the payment reported for today, the simulation
        // does not count the installment due on `end` (docs/rust-backend.md, "Known issues of the Python reference", L1).
        let remaining_periods = count_periods(frequency, current, schedule.end)?.max(1);

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

        current = step.advance(current)?;
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
    // Kept from Python: at the last period the balance is zero whatever is
    // still unpaid (docs/rust-backend.md, "Known issues of the Python reference", L4).
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

    #[test]
    fn schedule_dates_drift_after_short_months_like_dateutil() {
        let schedule = Schedule {
            start: date("2025-01-31"),
            end: date("2026-01-31"),
            frequency: InstallmentFrequency::Monthly,
        };
        // 31 Jan -> 28 Feb -> 28 Mar -> 28 Apr.
        assert_eq!(
            next_installment_date(schedule, date("2025-04-01")).unwrap(),
            date("2025-04-28")
        );
    }

    #[test]
    fn counts_week_based_periods_by_stepping() {
        assert_eq!(
            count_periods(
                InstallmentFrequency::Semimonthly,
                date("2025-01-01"),
                date("2025-02-01")
            )
            .unwrap(),
            2
        );
        assert_eq!(
            count_periods(
                InstallmentFrequency::Quarterly,
                date("2025-01-15"),
                date("2025-10-14")
            )
            .unwrap(),
            2
        );
    }

    #[test]
    fn day_based_counts_match_stepping_the_schedule() {
        let from = date("2024-02-20");
        for frequency in [
            InstallmentFrequency::Weekly,
            InstallmentFrequency::Biweekly,
            InstallmentFrequency::Semimonthly,
        ] {
            let step = PeriodStep::of(frequency);
            let mut to = from;
            for _ in 0..400 {
                to = add_days(to, 1).unwrap();
                let mut stepped = 0;
                let mut cursor = from;
                while cursor < to {
                    cursor = step.advance(cursor).unwrap();
                    if cursor <= to {
                        stepped += 1;
                    }
                }
                assert_eq!(count_periods(frequency, from, to).unwrap(), stepped);
            }
            assert_eq!(count_periods(frequency, to, from).unwrap(), 0);
        }
        // The step after the last full week would pass 9999-12-31.
        assert!(
            count_periods(
                InstallmentFrequency::Weekly,
                date("9999-12-20"),
                date("9999-12-31")
            )
            .is_err()
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

    #[test]
    fn negative_full_period_rate_divides_by_zero_instead_of_panicking() {
        let mut params = fixed_loan();
        params.interest_rate = d("-12");
        let error = calculator("2026-11-18").calculate(&params).unwrap_err();
        assert!(matches!(error, CalculationError::Arithmetic(_)));
    }
}
