from typing import List, Optional
from unittest.mock import MagicMock

import pytest

from application.use_cases.calculate_savings import CalculateSavingsImpl
from domain.calculations import (
    SavingsCalculationRequest,
    SavingsPeriodicity,
    SavingsRetirementRequest,
    SavingsScenarioRequest,
)
from domain.dezimal import Dezimal
from domain.exception.exceptions import CalculationInputError, MissingFieldsError


def _use_case() -> CalculateSavingsImpl:
    return CalculateSavingsImpl()


def _request(
    base_amount: Optional[Dezimal] = Dezimal(0),
    years: Optional[int] = 1,
    periodicity: SavingsPeriodicity = SavingsPeriodicity.MONTHLY,
    scenarios: Optional[List[SavingsScenarioRequest]] = None,
    retirement: Optional[SavingsRetirementRequest] = None,
) -> SavingsCalculationRequest:
    if scenarios is None:
        scenarios = [
            SavingsScenarioRequest(
                scenario_id="default",
                annual_market_performance=Dezimal(0),
                periodic_contribution=Dezimal(100),
            )
        ]
    return SavingsCalculationRequest(
        base_amount=base_amount,
        years=years,
        periodicity=periodicity,
        scenarios=scenarios,
        retirement=retirement,
    )


def _scenario(
    scenario_id: str = "s1",
    annual_market_performance: Dezimal = Dezimal(0),
    periodic_contribution: Optional[Dezimal] = None,
    target_amount: Optional[Dezimal] = None,
) -> SavingsScenarioRequest:
    return SavingsScenarioRequest(
        scenario_id=scenario_id,
        annual_market_performance=annual_market_performance,
        periodic_contribution=periodic_contribution,
        target_amount=target_amount,
    )


# ---------------------------------------------------------------------------
# TestValidation
# ---------------------------------------------------------------------------


class TestValidation:
    @pytest.mark.asyncio
    async def test_raises_when_base_amount_is_none(self):
        uc = _use_case()
        req = _request(base_amount=None)

        with pytest.raises(MissingFieldsError) as exc_info:
            await uc.execute(req)

        assert "base_amount" in exc_info.value.missing_fields

    @pytest.mark.asyncio
    async def test_raises_when_years_is_zero_or_negative(self):
        uc = _use_case()
        req = _request(years=0)

        with pytest.raises(MissingFieldsError) as exc_info:
            await uc.execute(req)

        assert "years" in exc_info.value.missing_fields

    @pytest.mark.asyncio
    async def test_raises_when_scenarios_empty(self):
        uc = _use_case()
        req = _request(scenarios=[])

        with pytest.raises(MissingFieldsError) as exc_info:
            await uc.execute(req)

        assert "scenarios" in exc_info.value.missing_fields

    @pytest.mark.asyncio
    async def test_raises_when_scenario_missing_annual_performance(self):
        uc = _use_case()
        bad_scenario = MagicMock(spec=SavingsScenarioRequest)
        bad_scenario.scenario_id = "bad"
        bad_scenario.annual_market_performance = None
        bad_scenario.periodic_contribution = Dezimal(100)
        bad_scenario.target_amount = None
        req = _request(scenarios=[bad_scenario])

        with pytest.raises(MissingFieldsError) as exc_info:
            await uc.execute(req)

        assert any(
            "annual_market_performance" in f for f in exc_info.value.missing_fields
        )

    @pytest.mark.asyncio
    async def test_raises_when_retirement_missing_both_fields(self):
        uc = _use_case()
        req = _request(
            retirement=SavingsRetirementRequest(
                withdrawal_amount=None,
                withdrawal_years=None,
            )
        )

        with pytest.raises(MissingFieldsError) as exc_info:
            await uc.execute(req)

        assert len(exc_info.value.missing_fields) >= 1


# ---------------------------------------------------------------------------
# TestBasicCalculation
# ---------------------------------------------------------------------------


class TestBasicCalculation:
    @pytest.mark.asyncio
    async def test_simple_monthly_saving_no_market_return(self):
        uc = _use_case()
        req = _request(
            base_amount=Dezimal(0),
            years=1,
            periodicity=SavingsPeriodicity.MONTHLY,
            scenarios=[
                _scenario(
                    scenario_id="zero_return",
                    annual_market_performance=Dezimal(0),
                    periodic_contribution=Dezimal(100),
                )
            ],
        )

        result = await uc.execute(req)

        assert len(result.scenarios) == 1
        scenario = result.scenarios[0]
        assert len(scenario.accumulation_periods) == 12
        assert scenario.final_balance == Dezimal(1200)
        assert scenario.total_contributions == Dezimal(1200)

    @pytest.mark.asyncio
    async def test_with_base_amount_no_contribution(self):
        uc = _use_case()
        req = _request(
            base_amount=Dezimal(1000),
            years=1,
            periodicity=SavingsPeriodicity.MONTHLY,
            scenarios=[
                _scenario(
                    scenario_id="growth",
                    annual_market_performance=Dezimal("0.12"),
                    periodic_contribution=Dezimal(0),
                )
            ],
        )

        result = await uc.execute(req)

        scenario = result.scenarios[0]
        assert scenario.final_balance > Dezimal(1000)

    @pytest.mark.asyncio
    async def test_yearly_periodicity(self):
        uc = _use_case()
        req = _request(
            base_amount=Dezimal(1000),
            years=5,
            periodicity=SavingsPeriodicity.YEARLY,
            scenarios=[
                _scenario(
                    scenario_id="yearly",
                    annual_market_performance=Dezimal("0.05"),
                    periodic_contribution=Dezimal(500),
                )
            ],
        )

        result = await uc.execute(req)

        scenario = result.scenarios[0]
        assert len(scenario.accumulation_periods) == 5

    @pytest.mark.asyncio
    async def test_multiple_scenarios(self):
        uc = _use_case()
        req = _request(
            base_amount=Dezimal(1000),
            years=3,
            periodicity=SavingsPeriodicity.MONTHLY,
            scenarios=[
                _scenario(
                    scenario_id="low",
                    annual_market_performance=Dezimal("0.03"),
                    periodic_contribution=Dezimal(200),
                ),
                _scenario(
                    scenario_id="high",
                    annual_market_performance=Dezimal("0.10"),
                    periodic_contribution=Dezimal(200),
                ),
            ],
        )

        result = await uc.execute(req)

        assert len(result.scenarios) == 2
        ids = {s.scenario_id for s in result.scenarios}
        assert ids == {"low", "high"}


# ---------------------------------------------------------------------------
# TestTargetAmount
# ---------------------------------------------------------------------------


class TestTargetAmount:
    @pytest.mark.asyncio
    async def test_solves_contribution_for_target(self):
        uc = _use_case()
        req = _request(
            base_amount=Dezimal(0),
            years=5,
            periodicity=SavingsPeriodicity.MONTHLY,
            scenarios=[
                _scenario(
                    scenario_id="target",
                    annual_market_performance=Dezimal(0),
                    periodic_contribution=None,
                    target_amount=Dezimal(10000),
                )
            ],
        )

        result = await uc.execute(req)

        scenario = result.scenarios[0]
        assert scenario.periodic_contribution > Dezimal(0)
        # 60 periods, 0% return => contribution should be ~166.67
        expected = Dezimal("166.67")
        assert scenario.periodic_contribution == expected

    @pytest.mark.asyncio
    async def test_solves_periods_for_target(self):
        uc = _use_case()
        req = _request(
            base_amount=Dezimal(1000),
            years=None,
            periodicity=SavingsPeriodicity.MONTHLY,
            scenarios=[
                _scenario(
                    scenario_id="solve_periods",
                    annual_market_performance=Dezimal(0),
                    periodic_contribution=Dezimal(200),
                    target_amount=Dezimal(5000),
                )
            ],
        )

        result = await uc.execute(req)

        scenario = result.scenarios[0]
        # base=1000, need 4000 more at 200/period => 20 periods
        assert len(scenario.accumulation_periods) == 20
        assert scenario.final_balance >= Dezimal(5000)


# ---------------------------------------------------------------------------
# TestRetirement
# ---------------------------------------------------------------------------


class TestRetirement:
    @pytest.mark.asyncio
    async def test_retirement_with_withdrawal_amount(self):
        uc = _use_case()
        req = _request(
            base_amount=Dezimal(10000),
            years=10,
            periodicity=SavingsPeriodicity.MONTHLY,
            scenarios=[
                _scenario(
                    scenario_id="retire",
                    annual_market_performance=Dezimal("0.06"),
                    periodic_contribution=Dezimal(500),
                )
            ],
            retirement=SavingsRetirementRequest(
                withdrawal_amount=Dezimal(200),
                withdrawal_years=20,
            ),
        )

        result = await uc.execute(req)

        scenario = result.scenarios[0]
        assert scenario.retirement is not None
        assert len(scenario.retirement.periods) > 0
        assert scenario.retirement.withdrawal_amount == Dezimal(200)

    @pytest.mark.asyncio
    async def test_retirement_solves_withdrawal_amount(self):
        uc = _use_case()
        req = _request(
            base_amount=Dezimal(10000),
            years=10,
            periodicity=SavingsPeriodicity.MONTHLY,
            scenarios=[
                _scenario(
                    scenario_id="retire_solve",
                    annual_market_performance=Dezimal("0.06"),
                    periodic_contribution=Dezimal(500),
                )
            ],
            retirement=SavingsRetirementRequest(
                withdrawal_amount=None,
                withdrawal_years=20,
            ),
        )

        result = await uc.execute(req)

        scenario = result.scenarios[0]
        assert scenario.retirement is not None
        assert scenario.retirement.withdrawal_amount > Dezimal(0)
        assert len(scenario.retirement.periods) > 0


# ---------------------------------------------------------------------------
# TestSolvedValuesHold: what the calculator solves for is reached by its own
# projection, which rounds every return to cents.
# ---------------------------------------------------------------------------


class TestSolvedValuesHold:
    @pytest.mark.asyncio
    @pytest.mark.parametrize("performance", ["0.06", "-0.04", "0.25"])
    async def test_solved_contribution_reaches_the_target(self, performance):
        target = Dezimal(10000)
        req = _request(
            base_amount=Dezimal(1000),
            years=5,
            scenarios=[
                _scenario(
                    annual_market_performance=Dezimal(performance),
                    target_amount=target,
                )
            ],
        )

        scenario = (await _use_case().execute(req)).scenarios[0]

        assert scenario.final_balance >= target
        # One cent less per period would miss it.
        assert scenario.final_balance - target < Dezimal(60) * Dezimal("0.02")

    @pytest.mark.asyncio
    async def test_a_rate_too_small_to_matter_acts_as_zero(self):
        req = _request(
            years=1,
            scenarios=[
                _scenario(
                    annual_market_performance=Dezimal("1e-30"),
                    target_amount=Dezimal(1200),
                )
            ],
        )

        scenario = (await _use_case().execute(req)).scenarios[0]

        assert scenario.periodic_contribution == Dezimal(100)
        assert scenario.final_balance == Dezimal(1200)

    @pytest.mark.asyncio
    async def test_solved_horizon_reaches_the_target(self):
        target = Dezimal(5000)
        req = _request(
            base_amount=Dezimal(1000),
            years=None,
            scenarios=[
                _scenario(
                    annual_market_performance=Dezimal("0.06"),
                    periodic_contribution=Dezimal(200),
                    target_amount=target,
                )
            ],
        )

        scenario = (await _use_case().execute(req)).scenarios[0]

        assert scenario.final_balance >= target

    @pytest.mark.asyncio
    async def test_a_target_the_rounded_returns_never_reach_has_no_horizon(self):
        # A return of 0.0005 per period rounds to no return at all, so the balance
        # stays at 0.10 in the projection.
        req = _request(
            base_amount=Dezimal("0.10"),
            years=None,
            scenarios=[
                _scenario(
                    annual_market_performance=Dezimal("0.06"),
                    periodic_contribution=Dezimal(0),
                    target_amount=Dezimal("0.11"),
                )
            ],
        )

        with pytest.raises(CalculationInputError):
            await _use_case().execute(req)

    @pytest.mark.asyncio
    async def test_solved_contribution_funds_every_withdrawal(self):
        withdrawal = Dezimal(1000)
        req = _request(
            base_amount=Dezimal(0),
            years=10,
            scenarios=[
                _scenario(annual_market_performance=Dezimal("0.05")),
            ],
            retirement=SavingsRetirementRequest(
                withdrawal_amount=withdrawal, withdrawal_years=5
            ),
        )

        scenario = (await _use_case().execute(req)).scenarios[0]

        periods = scenario.retirement.periods
        assert len(periods) >= 60
        assert all(period.withdrawal == withdrawal for period in periods[:60])

    @pytest.mark.asyncio
    async def test_a_base_that_funds_retirement_needs_no_contribution(self):
        req = _request(
            base_amount=Dezimal(1_000_000),
            years=10,
            scenarios=[_scenario(annual_market_performance=Dezimal("0.05"))],
            retirement=SavingsRetirementRequest(
                withdrawal_amount=Dezimal(100), withdrawal_years=5
            ),
        )

        scenario = (await _use_case().execute(req)).scenarios[0]

        assert scenario.periodic_contribution == Dezimal(0)

    @pytest.mark.asyncio
    async def test_rejects_a_negative_withdrawal(self):
        req = _request(
            retirement=SavingsRetirementRequest(
                withdrawal_amount=Dezimal(-100), withdrawal_years=5
            ),
        )

        with pytest.raises(CalculationInputError):
            await _use_case().execute(req)
