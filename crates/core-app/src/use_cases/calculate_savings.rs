// SPDX-License-Identifier: AGPL-3.0-only

//! Port of `finanze/application/use_cases/calculate_savings.py`.
//!
//! Projects savings scenarios period by period: contributions are added first, then
//! the period return is applied and rounded to cents. Optionally a retirement phase
//! withdraws from the final balance. The arithmetic, rounding and iteration order
//! follow the Python code exactly.

use core_domain::Dezimal;
use core_domain::calculations::{
    SavingsCalculationRequest, SavingsCalculationResult, SavingsPeriodEntry, SavingsPeriodicity,
    SavingsRetirementPeriodEntry, SavingsRetirementRequest, SavingsRetirementResult,
    SavingsScenarioRequest, SavingsScenarioResult,
};
use core_domain::dezimal::Checked;
use core_domain::exception::{CalculationError, CalculationInputError, MissingFieldsError};

const NON_FINITE: &str = "calculation produced non-finite amount";
/// A solved contribution is rounded to cents and then raised to the smallest number
/// of cents with which the projection, which rounds every return to cents, reaches
/// what it was solved for: the step is doubled until it does, then halved back. More
/// doublings than this mean the goal cannot be reached.
const MAX_STEP_DOUBLINGS: usize = 64;
/// How often the search for a retirement contribution doubles its upper bound until
/// that bound funds every withdrawal.
const MAX_BOUND_DOUBLINGS: usize = 64;

/// Longest accumulation or retirement phase, in years. Python's own search for a
/// target stops at 200 years; longer horizons let one request exhaust memory and
/// CPU, so they are rejected (a documented deviation from Python, which has no
/// limit).
pub const MAX_YEARS: i64 = 200;

/// Most scenarios in one request, for the same reason.
pub const MAX_SCENARIOS: usize = 50;

/// `CalculateSavings`.
#[derive(Debug, Clone, Copy, Default)]
pub struct CalculateSavings;

impl CalculateSavings {
    pub fn execute(
        &self,
        request: &SavingsCalculationRequest,
    ) -> Result<SavingsCalculationResult, CalculationError> {
        validate(request)?;
        // `validate` has rejected a missing base amount.
        let data = Normalized {
            request,
            base_amount: request.base_amount.unwrap_or(Dezimal::ZERO),
        };

        let scenarios = request
            .scenarios
            .iter()
            .map(|scenario| normalize_scenario(scenario, &data))
            .collect::<Result<Vec<_>, _>>()?;

        let periods_per_year = data.request.periodicity.periods_per_year();
        let results = match data.request.years {
            Some(years) if years > 0 => {
                let total_periods = years.checked_mul(periods_per_year).ok_or_else(non_finite)?;
                scenarios
                    .iter()
                    .map(|scenario| run_scenario(&data, scenario, total_periods))
                    .collect::<Result<Vec<_>, _>>()?
            }
            _ => scenarios
                .iter()
                .map(|scenario| {
                    let periods = resolve_periods_for_scenario(&data, scenario)?;
                    run_scenario(&data, scenario, periods)
                })
                .collect::<Result<Vec<_>, _>>()?,
        };

        Ok(SavingsCalculationResult { scenarios: results })
    }
}

/// The request after defaults were applied.
#[derive(Debug, Clone, Copy)]
struct Normalized<'a> {
    request: &'a SavingsCalculationRequest,
    base_amount: Dezimal,
}

impl Normalized<'_> {
    fn periodicity(&self) -> SavingsPeriodicity {
        self.request.periodicity
    }

    fn periods_per_year(&self) -> i64 {
        self.request.periodicity.periods_per_year()
    }
}

/// A scenario whose contribution and target are known.
#[derive(Debug, Clone)]
struct Scenario {
    scenario_id: String,
    annual_market_performance: Dezimal,
    periodic_contribution: Dezimal,
}

fn non_finite() -> CalculationError {
    CalculationInputError::new(NON_FINITE).into()
}

/// Unwraps a checked result. Python does not catch the decimal signals of the
/// arithmetic here, so they end as unexpected errors; only rounding to cents
/// reports invalid input (`round_cents`).
fn value(checked: Checked) -> Result<Dezimal, CalculationError> {
    checked.value().map_err(CalculationError::Arithmetic)
}

fn round_cents(amount: Dezimal) -> Result<Dezimal, CalculationError> {
    amount.round_cents().map_err(|_| non_finite())
}

/// True when the rate does not change `1 + rate`, so it acts as zero.
fn negligible_rate(rate: Dezimal) -> Result<bool, CalculationError> {
    Ok(value(Dezimal::ONE + rate)? == Dezimal::ONE)
}

fn period_rate(
    annual_rate: Dezimal,
    periodicity: SavingsPeriodicity,
) -> Result<Dezimal, CalculationError> {
    let periods = periodicity.periods_per_year();
    if periods <= 0 {
        return Ok(Dezimal::ZERO);
    }
    value(annual_rate / Dezimal::from(periods))
}

fn validate(request: &SavingsCalculationRequest) -> Result<(), CalculationError> {
    let mut missing: Vec<&str> = Vec::new();
    if request.base_amount.is_none() {
        missing.push("base_amount");
    }
    if request.years.is_some_and(|years| years <= 0) {
        missing.push("years");
    }
    if request.scenarios.is_empty() {
        missing.push("scenarios");
    }
    if !missing.is_empty() {
        return Err(MissingFieldsError::new(missing).into());
    }
    if let Some(retirement) = &request.retirement
        && retirement.withdrawal_amount.is_none()
        && retirement.withdrawal_years.is_none()
    {
        return Err(MissingFieldsError::new([
            "retirement.withdrawal_amount | retirement.withdrawal_years",
        ])
        .into());
    }
    if request
        .retirement
        .as_ref()
        .and_then(|retirement| retirement.withdrawal_amount)
        .is_some_and(|amount| amount < Dezimal::ZERO)
    {
        return Err(CalculationInputError::new(
            "retirement.withdrawal_amount must not be negative",
        )
        .into());
    }
    if request.years.is_some_and(|years| years > MAX_YEARS) {
        return Err(
            CalculationInputError::new(format!("years must not exceed {MAX_YEARS}")).into(),
        );
    }
    if request
        .retirement
        .as_ref()
        .and_then(|retirement| retirement.withdrawal_years)
        .is_some_and(|years| years > MAX_YEARS)
    {
        return Err(CalculationInputError::new(format!(
            "retirement.withdrawal_years must not exceed {MAX_YEARS}"
        ))
        .into());
    }
    if request.scenarios.len() > MAX_SCENARIOS {
        return Err(CalculationInputError::new(format!(
            "at most {MAX_SCENARIOS} scenarios are supported"
        ))
        .into());
    }
    Ok(())
}

fn resolve_periods_for_scenario(
    data: &Normalized<'_>,
    normalized: &NormalizedScenario,
) -> Result<i64, CalculationError> {
    let scenario = &normalized.scenario;
    // Python gates this on `scenario.target_amount and request.base_amount`, but
    // both are `Dezimal`s, which define no `__bool__` and are true even when zero.
    // A scenario without a target therefore solves for a target of zero, which a
    // negative base can reach; the fixtures record that Python answers it.
    if let Some(implied) = solve_periods_for_target(
        data,
        scenario.annual_market_performance,
        Some(scenario.periodic_contribution),
        normalized.target_amount,
    )? && implied > 0
    {
        return Ok(implied);
    }
    Err(CalculationInputError::new(
        "years must be provided or derivable from scenario target and contribution",
    )
    .into())
}

/// Number of periods until the balance reaches `target`, or `None` when it never
/// does within 200 years.
fn solve_periods_for_target(
    data: &Normalized<'_>,
    annual_market_performance: Dezimal,
    periodic_contribution: Option<Dezimal>,
    target: Dezimal,
) -> Result<Option<i64>, CalculationError> {
    let rate = period_rate(annual_market_performance, data.periodicity())?;
    let periodic = periodic_contribution.unwrap_or(Dezimal::ZERO);
    if negligible_rate(rate)? && periodic.is_zero() {
        return Ok(None);
    }
    let limit = 200 * data.periods_per_year();
    let mut periods = 0;
    let mut balance = data.base_amount;
    while balance < target && periods < limit {
        balance = value(balance + periodic)?;
        // Rounded to cents like the projection, so that it reaches the target too.
        balance = value(balance + round_cents(value(balance * rate)?)?)?;
        periods += 1;
    }
    if balance < target {
        return Ok(None);
    }
    Ok((periods > 0).then_some(periods))
}

fn normalize_scenario(
    scenario: &SavingsScenarioRequest,
    data: &Normalized<'_>,
) -> Result<NormalizedScenario, CalculationError> {
    let rate = period_rate(scenario.annual_market_performance, data.periodicity())?;
    let mut target_amount = scenario.target_amount;
    let mut periodic_contribution = scenario.periodic_contribution;

    if target_amount.is_none()
        && periodic_contribution.is_none()
        && let Some(retirement) = &data.request.retirement
        && retirement.withdrawal_amount.is_some()
        && retirement.withdrawal_years.is_some()
        && let Some(retirement_target) =
            retirement_target_balance(rate, retirement, data.periodicity())?
    {
        target_amount = Some(retirement_target);
        periodic_contribution = Some(solve_contribution_for_retirement(data, rate)?);
    }

    let periodic_contribution = match periodic_contribution {
        Some(contribution) => contribution,
        None => match target_amount {
            None => Dezimal::ZERO,
            Some(target) => {
                let mut horizon_periods = match data.request.years {
                    Some(years) if years > 0 => Some(
                        years
                            .checked_mul(data.periods_per_year())
                            .ok_or_else(non_finite)?,
                    ),
                    _ => None,
                };
                if horizon_periods.is_none_or(|periods| periods <= 0) {
                    horizon_periods = solve_periods_for_target(
                        data,
                        scenario.annual_market_performance,
                        None,
                        target,
                    )?;
                }
                match horizon_periods {
                    Some(periods) if periods > 0 => solve_required_contribution_periods(
                        data,
                        scenario.annual_market_performance,
                        target,
                        periods,
                    )?,
                    _ => {
                        return Err(CalculationInputError::new(
                            "years required to solve periodic contribution",
                        )
                        .into());
                    }
                }
            }
        },
    };

    Ok(NormalizedScenario {
        scenario: Scenario {
            scenario_id: scenario.scenario_id.clone(),
            annual_market_performance: scenario.annual_market_performance,
            periodic_contribution,
        },
        target_amount: target_amount.unwrap_or(Dezimal::ZERO),
    })
}

/// A scenario with its contribution and target resolved.
#[derive(Debug, Clone)]
struct NormalizedScenario {
    scenario: Scenario,
    target_amount: Dezimal,
}

fn solve_contribution_for_retirement(
    data: &Normalized<'_>,
    rate: Dezimal,
) -> Result<Dezimal, CalculationError> {
    let (Some(retirement), Some(years)) = (&data.request.retirement, data.request.years) else {
        return Ok(Dezimal::ZERO);
    };
    let (Some(withdrawal), Some(withdrawal_years)) =
        (retirement.withdrawal_amount, retirement.withdrawal_years)
    else {
        return Ok(Dezimal::ZERO);
    };
    if years <= 0 {
        return Ok(Dezimal::ZERO);
    }
    let periods_per_year = data.periods_per_year();
    let retirement_periods = withdrawal_years
        .checked_mul(periods_per_year)
        .ok_or_else(non_finite)?;
    let accumulation_periods = years.checked_mul(periods_per_year).ok_or_else(non_finite)?;
    let base = data.base_amount;
    let initial_estimate = estimate_contribution_for_retirement(
        base,
        rate,
        accumulation_periods,
        withdrawal,
        retirement_periods,
    )?;
    let contribution = refine_contribution_via_simulation(
        base,
        rate,
        accumulation_periods,
        withdrawal,
        retirement_periods,
        initial_estimate,
    )?;
    raise_until(round_cents(contribution)?, |candidate| {
        Ok(simulate_full_cycle(
            base,
            rate,
            accumulation_periods,
            candidate,
            withdrawal,
            retirement_periods,
        )? >= Dezimal::ZERO)
    })
}

fn estimate_contribution_for_retirement(
    base: Dezimal,
    rate: Dezimal,
    accumulation_periods: i64,
    withdrawal: Dezimal,
    retirement_periods: i64,
) -> Result<Dezimal, CalculationError> {
    let one = Dezimal::ONE;
    if negligible_rate(rate)? {
        let total_needed = value(withdrawal * Dezimal::from(retirement_periods))?;
        if total_needed <= base {
            return Ok(Dezimal::ZERO);
        }
        let shortfall = value(total_needed - base)?;
        return value(shortfall / Dezimal::from(accumulation_periods));
    }
    let growth = value((one + rate).powi(accumulation_periods))?;
    let base_at_retirement = value(base * growth)?;
    let discount = value(one - (one + rate).powi(-retirement_periods))?;
    let pv_ordinary = value(withdrawal * discount / rate)?;
    let required_balance = value(pv_ordinary * (one + rate))?;
    if required_balance <= base_at_retirement {
        return Ok(Dezimal::ZERO);
    }
    let shortfall = value(required_balance - base_at_retirement)?;
    // Each contribution earns the return of its own period (an annuity due).
    let annuity = value((growth - one) / rate * (one + rate))?;
    if annuity <= Dezimal::ZERO {
        return Ok(shortfall);
    }
    value(shortfall / annuity)
}

fn refine_contribution_via_simulation(
    base: Dezimal,
    rate: Dezimal,
    accumulation_periods: i64,
    withdrawal: Dezimal,
    retirement_periods: i64,
    initial_estimate: Dezimal,
) -> Result<Dezimal, CalculationError> {
    let funded = |contribution: Dezimal| {
        simulate_full_cycle(
            base,
            rate,
            accumulation_periods,
            contribution,
            withdrawal,
            retirement_periods,
        )
    };
    // The base alone already funds every withdrawal.
    if funded(Dezimal::ZERO)? >= Dezimal::ZERO {
        return Ok(Dezimal::ZERO);
    }
    let mut low = Dezimal::ZERO;
    let mut high = value(initial_estimate * Dezimal::from(3))?;
    if high <= Dezimal::ZERO {
        high = value(withdrawal * Dezimal::TWO)?;
    }
    // The search only converges when the upper bound funds every withdrawal.
    for _ in 0..MAX_BOUND_DOUBLINGS {
        if high <= Dezimal::ZERO || funded(high)? >= Dezimal::ZERO {
            break;
        }
        low = high;
        high = value(high * Dezimal::TWO)?;
    }
    let tolerance = Dezimal::CENT;
    let half_cent = value(Dezimal::CENT / Dezimal::TWO)?;
    for _ in 0..100 {
        let mid = value((low + high) / Dezimal::TWO)?;
        let final_balance = funded(mid)?;
        if final_balance.abs() < tolerance {
            return Ok(mid);
        }
        let bounds = (low, high);
        if final_balance > Dezimal::ZERO {
            high = mid;
        } else {
            low = mid;
        }
        // Once the bounds stop moving every further round repeats this one, so the
        // result is the same as after all 100 rounds, as Python computes it.
        if (low, high) == bounds {
            break;
        }
        // The result is rounded to cents, so a narrower bracket changes nothing.
        if value(high - low)? < half_cent {
            break;
        }
    }
    value((low + high) / Dezimal::TWO)
}

fn simulate_full_cycle(
    base: Dezimal,
    rate: Dezimal,
    accumulation_periods: i64,
    contribution: Dezimal,
    withdrawal: Dezimal,
    retirement_periods: i64,
) -> Result<Dezimal, CalculationError> {
    let balance = accumulate(base, rate, accumulation_periods, contribution)?;
    simulate_retirement_balance(balance, rate, retirement_periods, withdrawal)
}

/// The balance after `periods` contributions, as the projection computes it.
fn accumulate(
    base: Dezimal,
    rate: Dezimal,
    periods: i64,
    contribution: Dezimal,
) -> Result<Dezimal, CalculationError> {
    let mut balance = base;
    for _ in 0..periods {
        balance = value(balance + contribution)?;
        balance = value(balance + round_cents(value(balance * rate)?)?)?;
    }
    Ok(balance)
}

/// The smallest contribution in whole cents that reaches its goal under the
/// projection's rounding of returns, searched from the given one in both directions.
fn raise_until(
    contribution: Dezimal,
    reaches: impl Fn(Dezimal) -> Result<bool, CalculationError>,
) -> Result<Dezimal, CalculationError> {
    let start = Dezimal::ZERO.max(contribution);
    let mut step = Dezimal::CENT;
    if reaches(start)? {
        // Rounding returns to cents can let a smaller contribution reach it too.
        let mut reaching = start;
        for _ in 0..MAX_STEP_DOUBLINGS {
            if reaching <= Dezimal::ZERO {
                return Ok(reaching);
            }
            let candidate = Dezimal::ZERO.max(value(start - step)?);
            if !reaches(candidate)? {
                return narrow(candidate, reaching, &reaches);
            }
            reaching = candidate;
            step = value(step * Dezimal::TWO)?;
        }
        if reaching <= Dezimal::ZERO || reaches(Dezimal::ZERO)? {
            return Ok(Dezimal::ZERO);
        }
        return narrow(Dezimal::ZERO, reaching, &reaches);
    }
    let mut failing = start;
    for _ in 0..MAX_STEP_DOUBLINGS {
        let candidate = value(start + step)?;
        if reaches(candidate)? {
            return narrow(failing, candidate, &reaches);
        }
        failing = candidate;
        step = value(step * Dezimal::TWO)?;
    }
    Err(
        CalculationInputError::new("no contribution reaches the goal with this market performance")
            .into(),
    )
}

/// The smallest contribution in whole cents above `failing`, which does not reach the
/// goal, and up to `reaching`, which does.
fn narrow(
    mut failing: Dezimal,
    mut reaching: Dezimal,
    reaches: &impl Fn(Dezimal) -> Result<bool, CalculationError>,
) -> Result<Dezimal, CalculationError> {
    while value(reaching - failing)? > Dezimal::CENT {
        let middle = round_cents(value((failing + reaching) / Dezimal::TWO)?)?;
        // Near 28 significant digits, rounding can keep the middle on a bound.
        if middle == failing || middle == reaching {
            break;
        }
        if reaches(middle)? {
            reaching = middle;
        } else {
            failing = middle;
        }
    }
    Ok(reaching)
}

fn solve_required_contribution_periods(
    data: &Normalized<'_>,
    annual_market_performance: Dezimal,
    target: Dezimal,
    periods: i64,
) -> Result<Dezimal, CalculationError> {
    if periods <= 0 {
        return Ok(Dezimal::ZERO);
    }
    let one = Dezimal::ONE;
    let base = data.base_amount;
    let rate = period_rate(annual_market_performance, data.periodicity())?;
    let growth = value((one + rate).powi(periods))?;
    let target_remaining = value(target - base * growth)?;
    let contribution = if target_remaining <= Dezimal::ZERO {
        // The base alone reaches the target, unless rounding its returns to cents
        // holds it back; the search below starts from zero then.
        Dezimal::ZERO
    } else if negligible_rate(rate)? {
        value(target_remaining / Dezimal::from(periods))?
    } else {
        // Each contribution earns the return of its own period (an annuity due), as
        // in the projection.
        let annuity_factor = value(((one + rate).powi(periods) - one) / rate * (one + rate))?;
        if annuity_factor <= Dezimal::ZERO {
            return Err(CalculationInputError::new(
                "target_amount cannot be reached with this market performance",
            )
            .into());
        }
        value(target_remaining / annuity_factor)?
    };
    raise_until(round_cents(contribution)?, |candidate| {
        Ok(accumulate(base, rate, periods, candidate)? >= target)
    })
}

fn run_scenario(
    data: &Normalized<'_>,
    normalized: &NormalizedScenario,
    total_periods: i64,
) -> Result<SavingsScenarioResult, CalculationError> {
    let scenario = &normalized.scenario;
    let rate = period_rate(scenario.annual_market_performance, data.periodicity())?;
    let base_amount = data.base_amount;
    let contribution = scenario.periodic_contribution;

    let mut accumulation_periods = Vec::new();
    let mut balance = base_amount;
    let mut total_contributed = Dezimal::ZERO;
    let mut total_revaluation = Dezimal::ZERO;
    for index in 1..=total_periods {
        balance = value(balance + contribution)?;
        total_contributed = value(total_contributed + contribution)?;
        let revaluation = round_cents(value(balance * rate)?)?;
        balance = value(balance + revaluation)?;
        total_revaluation = value(total_revaluation + revaluation)?;
        let total_invested = round_cents(value(base_amount + total_contributed)?)?;
        accumulation_periods.push(SavingsPeriodEntry {
            period_index: index,
            contributed: round_cents(contribution)?,
            total_contributed: round_cents(total_contributed)?,
            revaluation: round_cents(revaluation)?,
            total_revaluation: round_cents(total_revaluation)?,
            total_invested,
            balance: round_cents(balance)?,
        });
    }

    let retirement = match &data.request.retirement {
        Some(retirement) => Some(compute_retirement(
            retirement,
            balance,
            rate,
            data.periodicity(),
        )?),
        None => None,
    };

    Ok(SavingsScenarioResult {
        scenario_id: scenario.scenario_id.clone(),
        annual_market_performance: scenario.annual_market_performance,
        periodic_contribution: round_cents(contribution)?,
        accumulation_periods,
        total_contributions: round_cents(total_contributed)?,
        total_revaluation: round_cents(total_revaluation)?,
        final_balance: round_cents(balance)?,
        retirement,
    })
}

/// Balance needed at retirement to withdraw the amount at the start of every
/// period for the given years.
fn retirement_target_balance(
    rate: Dezimal,
    retirement: &SavingsRetirementRequest,
    periodicity: SavingsPeriodicity,
) -> Result<Option<Dezimal>, CalculationError> {
    let (Some(amount), Some(years)) = (retirement.withdrawal_amount, retirement.withdrawal_years)
    else {
        return Ok(None);
    };
    if years <= 0 {
        return Ok(None);
    }
    let periods = years
        .checked_mul(periodicity.periods_per_year())
        .ok_or_else(non_finite)?;
    if periods <= 0 {
        return Ok(None);
    }
    let one = Dezimal::ONE;
    if rate.is_zero() {
        return round_cents(value(amount * Dezimal::from(periods))?).map(Some);
    }
    let discount = value(one - (one + rate).powi(-periods))?;
    if discount.is_zero() {
        return round_cents(value(amount * Dezimal::from(periods))?).map(Some);
    }
    let pv_ordinary = value(amount * discount / rate)?;
    let target = value(pv_ordinary * (one + rate))?;
    round_cents(target).map(Some)
}

fn compute_retirement(
    retirement: &SavingsRetirementRequest,
    balance: Dezimal,
    rate: Dezimal,
    periodicity: SavingsPeriodicity,
) -> Result<SavingsRetirementResult, CalculationError> {
    let periods_per_year = periodicity.periods_per_year();
    // Python's integers do not overflow; a saturated negative target ends the loop
    // at once, as Python's does.
    let periods_target = retirement
        .withdrawal_years
        .map(|years| years.saturating_mul(periods_per_year));
    let amount = match (retirement.withdrawal_amount, periods_target) {
        (Some(amount), _) => amount,
        (None, Some(periods)) if periods > 0 => solve_withdrawal_amount(balance, rate, periods)?,
        _ => Dezimal::ZERO,
    };

    let mut periods = Vec::new();
    let mut current = balance;
    let mut total_withdrawn = Dezimal::ZERO;
    let mut index: i64 = 1;
    let safety_limit = periods_target.unwrap_or(100 * periods_per_year);
    while current > Dezimal::ZERO {
        if index > safety_limit {
            break;
        }
        let withdrawal = if current >= amount { amount } else { current };
        current = value(current - withdrawal)?;
        total_withdrawn = value(total_withdrawn + withdrawal)?;
        let revaluation = round_cents(value(current * rate)?)?;
        current = value(current + revaluation)?;
        periods.push(SavingsRetirementPeriodEntry {
            period_index: index,
            withdrawal: round_cents(withdrawal)?,
            total_withdrawn: round_cents(total_withdrawn)?,
            revaluation: round_cents(revaluation)?,
            balance: round_cents(current)?,
        });
        index += 1;
        if amount.is_zero() {
            break;
        }
    }

    let duration_periods = i64::try_from(periods.len()).map_err(|_| non_finite())?;
    let duration_years = if duration_periods > 0 {
        value(Dezimal::from(duration_periods) / Dezimal::from(periods_per_year))?
    } else {
        Dezimal::ZERO
    };

    Ok(SavingsRetirementResult {
        withdrawal_amount: round_cents(amount)?,
        duration_periods,
        duration_years: round_cents(duration_years)?,
        total_withdrawn: round_cents(total_withdrawn)?,
        periods,
    })
}

fn simulate_retirement_balance(
    balance: Dezimal,
    rate: Dezimal,
    periods: i64,
    withdrawal: Dezimal,
) -> Result<Dezimal, CalculationError> {
    let mut current = balance;
    for index in 1..=periods {
        current = value(current - withdrawal)?;
        if current < Dezimal::ZERO {
            // This withdrawal and all later ones are not funded in full.
            return value(current - withdrawal * Dezimal::from(periods - index));
        }
        if index < periods {
            current = value(current + round_cents(value(current * rate)?)?)?;
        }
    }
    Ok(current)
}

fn solve_withdrawal_amount(
    balance: Dezimal,
    rate: Dezimal,
    periods_target: i64,
) -> Result<Dezimal, CalculationError> {
    if periods_target <= 0 {
        return Ok(Dezimal::ZERO);
    }
    if rate.is_zero() {
        return round_cents(value(balance / Dezimal::from(periods_target))?);
    }
    let one = Dezimal::ONE;
    let discount = value(one - (one + rate).powi(-periods_target))?;
    if discount.is_zero() {
        return round_cents(value(balance / Dezimal::from(periods_target))?);
    }
    let pv_factor = value((discount / rate) * (one + rate))?;
    round_cents(value(balance / pv_factor)?)
}
