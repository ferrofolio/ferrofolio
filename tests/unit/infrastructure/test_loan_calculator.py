from datetime import date, timedelta

import pytest
from dateutil.relativedelta import relativedelta

from domain.dezimal import Dezimal
from domain.exception.exceptions import CalculationInputError, MissingFieldsError
from domain.global_position import InstallmentFrequency, InterestType
from domain.loan_calculator import LoanCalculationParams
from infrastructure.calculations import loan_calculator
from infrastructure.calculations.loan_calculator import LoanCalculator


def _calculator() -> LoanCalculator:
    return LoanCalculator()


# The day that tests which do not choose their own date compute as of.
_TODAY = date(2026, 6, 15)


def _on(monkeypatch, today: date) -> None:
    """Makes the calculator compute as of `today`."""

    class _Today(date):
        @classmethod
        def today(cls):
            return today

    monkeypatch.setattr(loan_calculator, "date", _Today)


def _params(
    loan_amount=Dezimal(100000),
    interest_rate=Dezimal("0.03"),
    interest_type=InterestType.FIXED,
    euribor_rate=None,
    fixed_years=None,
    start=date(2020, 1, 15),
    end=date(2050, 1, 15),
    principal_outstanding=None,
    fixed_interest_rate=None,
    installment_frequency=InstallmentFrequency.MONTHLY,
) -> LoanCalculationParams:
    return LoanCalculationParams(
        loan_amount=loan_amount,
        interest_rate=interest_rate,
        interest_type=interest_type,
        euribor_rate=euribor_rate,
        fixed_years=fixed_years,
        start=start,
        end=end,
        principal_outstanding=principal_outstanding,
        fixed_interest_rate=fixed_interest_rate,
        installment_frequency=installment_frequency,
    )


# ---------------------------------------------------------------------------
# TestValidation
# ---------------------------------------------------------------------------


class TestValidation:
    @pytest.mark.asyncio
    async def test_raises_when_both_amounts_missing(self):
        calc = _calculator()
        params = _params(loan_amount=None, principal_outstanding=None)

        with pytest.raises(MissingFieldsError) as exc_info:
            await calc.calculate(params)

        assert any(
            "loan_amount" in f and "principal_outstanding" in f
            for f in exc_info.value.missing_fields
        )

    @pytest.mark.asyncio
    async def test_raises_when_euribor_missing_for_variable(self):
        calc = _calculator()
        params = _params(interest_type=InterestType.VARIABLE, euribor_rate=None)

        with pytest.raises(MissingFieldsError) as exc_info:
            await calc.calculate(params)

        assert any("euribor_rate" in f for f in exc_info.value.missing_fields)

    @pytest.mark.asyncio
    async def test_raises_when_euribor_missing_for_mixed(self):
        calc = _calculator()
        params = _params(
            interest_type=InterestType.MIXED,
            euribor_rate=None,
            fixed_years=5,
        )

        with pytest.raises(MissingFieldsError) as exc_info:
            await calc.calculate(params)

        assert any("euribor_rate" in f for f in exc_info.value.missing_fields)

    @pytest.mark.asyncio
    async def test_raises_when_fixed_years_missing_for_mixed(self):
        calc = _calculator()
        params = _params(
            interest_type=InterestType.MIXED,
            euribor_rate=Dezimal("0.01"),
            fixed_years=None,
        )

        with pytest.raises(MissingFieldsError) as exc_info:
            await calc.calculate(params)

        assert any("fixed_years" in f for f in exc_info.value.missing_fields)

    @pytest.mark.asyncio
    async def test_accepts_valid_fixed_params(self):
        calc = _calculator()
        params = _params()

        result = await calc.calculate(params)

        assert result is not None
        assert result.current_installment_payment is not None


# ---------------------------------------------------------------------------
# TestFixedRate
# ---------------------------------------------------------------------------


class TestFixedRate:
    @pytest.mark.asyncio
    async def test_fixed_with_loan_amount(self):
        calc = _calculator()
        params = _params(
            loan_amount=Dezimal(200000),
            interest_rate=Dezimal("0.025"),
        )

        result = await calc.calculate(params)

        assert result.current_installment_payment is not None
        assert result.current_installment_payment.val > 0
        assert result.current_installment_interests is not None
        assert result.current_installment_interests.val > 0
        assert result.principal_outstanding is not None
        assert result.principal_outstanding.val > 0

    @pytest.mark.asyncio
    async def test_fixed_with_principal_outstanding_only(self):
        calc = _calculator()
        params = _params(
            loan_amount=None,
            principal_outstanding=Dezimal(80000),
            interest_rate=Dezimal("0.03"),
        )

        result = await calc.calculate(params)

        assert result.current_installment_payment is not None
        assert result.current_installment_payment.val > 0
        assert result.principal_outstanding == Dezimal(80000)

    @pytest.mark.asyncio
    async def test_fixed_zero_interest(self):
        calc = _calculator()
        params = _params(
            loan_amount=Dezimal(120000),
            interest_rate=Dezimal(0),
        )

        result = await calc.calculate(params)

        assert result.current_installment_interests == Dezimal(0)
        assert result.current_installment_payment is not None
        assert result.current_installment_payment.val > 0

    @pytest.mark.asyncio
    async def test_fixed_result_has_installment_date(self):
        calc = _calculator()
        params = _params()

        result = await calc.calculate(params)

        assert result.installment_date is not None
        assert isinstance(result.installment_date, date)


# ---------------------------------------------------------------------------
# TestVariableRate
# ---------------------------------------------------------------------------


class TestVariableRate:
    @pytest.mark.asyncio
    async def test_variable_with_loan_amount(self):
        calc = _calculator()
        params = _params(
            loan_amount=Dezimal(150000),
            interest_rate=Dezimal("0.01"),
            interest_type=InterestType.VARIABLE,
            euribor_rate=Dezimal("0.035"),
        )

        result = await calc.calculate(params)

        assert result.current_installment_payment is not None
        assert result.current_installment_payment.val > 0
        assert result.current_installment_interests is not None
        assert result.current_installment_interests.val > 0
        assert result.principal_outstanding is not None
        assert result.principal_outstanding.val > 0

    @pytest.mark.asyncio
    async def test_variable_with_principal_outstanding(self):
        calc = _calculator()
        params = _params(
            loan_amount=None,
            principal_outstanding=Dezimal(90000),
            interest_rate=Dezimal("0.015"),
            interest_type=InterestType.VARIABLE,
            euribor_rate=Dezimal("0.02"),
        )

        result = await calc.calculate(params)

        assert result.current_installment_payment is not None
        assert result.current_installment_payment.val > 0
        assert result.principal_outstanding == Dezimal(90000)

    @pytest.mark.asyncio
    async def test_variable_combined_rate_higher_payment(self):
        """A VARIABLE loan whose combined rate exceeds a FIXED rate
        should produce a higher monthly payment for the same principal."""
        calc = _calculator()

        fixed_params = _params(
            loan_amount=None,
            principal_outstanding=Dezimal(100000),
            interest_rate=Dezimal("0.02"),
            interest_type=InterestType.FIXED,
        )

        variable_params = _params(
            loan_amount=None,
            principal_outstanding=Dezimal(100000),
            interest_rate=Dezimal("0.02"),
            interest_type=InterestType.VARIABLE,
            euribor_rate=Dezimal("0.03"),
        )

        fixed_result = await calc.calculate(fixed_params)
        variable_result = await calc.calculate(variable_params)

        assert (
            variable_result.current_installment_payment
            > fixed_result.current_installment_payment
        )


# ---------------------------------------------------------------------------
# TestMixedRate
# ---------------------------------------------------------------------------


class TestMixedRate:
    @pytest.mark.asyncio
    async def test_mixed_during_fixed_period(self):
        """When today is within the fixed period of a MIXED loan,
        the effective rate should equal the base rate (no euribor).
        We verify this by checking the interest component reflects only the
        base rate, not base + euribor."""
        calc = _calculator()
        base_rate = Dezimal("0.02")
        euribor = Dezimal("0.03")
        outstanding = Dezimal(80000)

        params = _params(
            loan_amount=None,
            principal_outstanding=outstanding,
            interest_rate=base_rate,
            interest_type=InterestType.MIXED,
            euribor_rate=euribor,
            fixed_years=50,
            start=date(2020, 1, 15),
            end=date(2050, 1, 15),
        )

        result = await calc.calculate(params)

        assert result.current_installment_payment is not None
        assert result.current_installment_payment.val > 0
        # During the fixed period, interest should be based on base rate only
        # interest_if_euribor_included would be outstanding * (base + euribor) / 12
        interest_with_euribor = outstanding * (base_rate + euribor) / Dezimal(12)
        assert result.current_installment_interests.val < interest_with_euribor.val

    @pytest.mark.asyncio
    async def test_mixed_with_principal_outstanding(self):
        calc = _calculator()
        params = _params(
            loan_amount=None,
            principal_outstanding=Dezimal(75000),
            interest_rate=Dezimal("0.025"),
            interest_type=InterestType.MIXED,
            euribor_rate=Dezimal("0.015"),
            fixed_years=5,
        )

        result = await calc.calculate(params)

        assert result.current_installment_payment is not None
        assert result.current_installment_payment.val > 0
        assert result.principal_outstanding == Dezimal(75000)


# ---------------------------------------------------------------------------
# TestInstallmentDate
# ---------------------------------------------------------------------------


class TestInstallmentDate:
    @pytest.mark.asyncio
    async def test_installment_date_in_future(self, monkeypatch):
        """For a loan that started in the past, the next installment date
        should be on or after today."""
        _on(monkeypatch, _TODAY)
        calc = _calculator()
        params = _params(
            start=date(2020, 1, 15),
            end=date(2050, 1, 15),
        )

        result = await calc.calculate(params)

        today = _TODAY
        assert result.installment_date is not None
        assert result.installment_date >= today


# ---------------------------------------------------------------------------
# TestFixedRateWithOutstandingOnly
# ---------------------------------------------------------------------------


class TestFixedRateWithOutstandingOnly:
    @pytest.mark.asyncio
    async def test_recovers_annuity_and_interests(self):
        calc = _calculator()
        params = _params(
            loan_amount=None,
            principal_outstanding=Dezimal(80000),
            interest_rate=Dezimal("0.03"),
        )

        result = await calc.calculate(params)

        assert result.current_installment_payment is not None
        assert result.current_installment_payment.val > 0
        assert result.current_installment_interests is not None
        assert result.current_installment_interests.val > 0
        assert result.principal_outstanding == Dezimal(80000)

    @pytest.mark.asyncio
    async def test_zero_interest_even_division(self):
        calc = _calculator()
        params = _params(
            loan_amount=None,
            principal_outstanding=Dezimal(60000),
            interest_rate=Dezimal(0),
        )

        result = await calc.calculate(params)

        assert result.current_installment_interests == Dezimal(0)
        assert result.current_installment_payment is not None
        assert result.current_installment_payment.val > 0


# ---------------------------------------------------------------------------
# TestMixedWithOutstandingOnly
# ---------------------------------------------------------------------------


class TestMixedWithOutstandingOnly:
    @pytest.mark.asyncio
    async def test_fixed_period_interests_exclude_euribor(self):
        calc = _calculator()
        base_rate = Dezimal("0.02")
        euribor = Dezimal("0.03")
        outstanding = Dezimal(75000)

        params = _params(
            loan_amount=None,
            principal_outstanding=outstanding,
            interest_rate=base_rate,
            interest_type=InterestType.MIXED,
            euribor_rate=euribor,
            fixed_years=50,
        )

        result = await calc.calculate(params)

        interest_with_euribor = outstanding * (base_rate + euribor) / Dezimal(12)
        assert result.current_installment_interests.val < interest_with_euribor.val

    @pytest.mark.asyncio
    async def test_variable_period_interests_include_euribor(self):
        calc = _calculator()
        base_rate = Dezimal("0.02")
        euribor = Dezimal("0.03")
        outstanding = Dezimal(75000)

        params = _params(
            loan_amount=None,
            principal_outstanding=outstanding,
            interest_rate=base_rate,
            interest_type=InterestType.MIXED,
            euribor_rate=euribor,
            fixed_years=1,
            start=date(2020, 1, 15),
            end=date(2050, 1, 15),
        )

        result = await calc.calculate(params)

        interest_fixed_only = outstanding * base_rate / Dezimal(12)
        assert result.current_installment_interests.val > interest_fixed_only.val


# ---------------------------------------------------------------------------
# TestInstallmentFrequencies
# ---------------------------------------------------------------------------


class TestInstallmentFrequencies:
    @pytest.mark.asyncio
    async def test_quarterly_higher_than_monthly(self):
        calc = _calculator()
        monthly = await calc.calculate(_params())
        quarterly = await calc.calculate(
            _params(installment_frequency=InstallmentFrequency.QUARTERLY)
        )
        assert (
            quarterly.current_installment_payment > monthly.current_installment_payment
        )

    @pytest.mark.asyncio
    async def test_semiannual_higher_than_quarterly(self):
        calc = _calculator()
        quarterly = await calc.calculate(
            _params(installment_frequency=InstallmentFrequency.QUARTERLY)
        )
        semiannual = await calc.calculate(
            _params(installment_frequency=InstallmentFrequency.SEMIANNUAL)
        )
        assert (
            semiannual.current_installment_payment
            > quarterly.current_installment_payment
        )

    @pytest.mark.asyncio
    async def test_yearly_highest(self):
        calc = _calculator()
        semiannual = await calc.calculate(
            _params(installment_frequency=InstallmentFrequency.SEMIANNUAL)
        )
        yearly = await calc.calculate(
            _params(installment_frequency=InstallmentFrequency.YEARLY)
        )
        assert (
            yearly.current_installment_payment > semiannual.current_installment_payment
        )

    @pytest.mark.asyncio
    async def test_biweekly_lower_than_monthly(self):
        calc = _calculator()
        monthly = await calc.calculate(_params())
        biweekly = await calc.calculate(
            _params(installment_frequency=InstallmentFrequency.BIWEEKLY)
        )
        assert (
            biweekly.current_installment_payment < monthly.current_installment_payment
        )

    @pytest.mark.asyncio
    async def test_weekly_lower_than_biweekly(self):
        calc = _calculator()
        biweekly = await calc.calculate(
            _params(installment_frequency=InstallmentFrequency.BIWEEKLY)
        )
        weekly = await calc.calculate(
            _params(installment_frequency=InstallmentFrequency.WEEKLY)
        )
        assert weekly.current_installment_payment < biweekly.current_installment_payment


# ---------------------------------------------------------------------------
# TestInterestBreakdown
# ---------------------------------------------------------------------------


class TestInterestBreakdown:
    @pytest.mark.asyncio
    async def test_interests_approximate_outstanding_times_period_rate(self):
        calc = _calculator()
        outstanding = Dezimal(100000)
        rate = Dezimal("0.036")

        params = _params(
            loan_amount=None,
            principal_outstanding=outstanding,
            interest_rate=rate,
        )

        result = await calc.calculate(params)

        expected = (outstanding * rate / Dezimal(12)).val.quantize(
            Dezimal("0.01").val, rounding="ROUND_HALF_UP"
        )
        assert result.current_installment_interests == Dezimal(expected)

    @pytest.mark.asyncio
    async def test_interests_decrease_with_lower_outstanding(self):
        calc = _calculator()
        rate = Dezimal("0.036")

        high = await calc.calculate(
            _params(
                loan_amount=None,
                principal_outstanding=Dezimal(100000),
                interest_rate=rate,
            )
        )
        low = await calc.calculate(
            _params(
                loan_amount=None,
                principal_outstanding=Dezimal(50000),
                interest_rate=rate,
            )
        )

        assert low.current_installment_interests < high.current_installment_interests


class TestNonFiniteInputs:
    @pytest.mark.asyncio
    @pytest.mark.parametrize(
        "field, value",
        [
            ("loan_amount", "NaN"),
            ("loan_amount", "Infinity"),
            ("principal_outstanding", "NaN"),
            ("interest_rate", "-Infinity"),
        ],
    )
    async def test_rejects_amounts_and_rates_that_are_not_finite(
        self, monkeypatch, field, value
    ):
        # Past its last installment the loan would need no arithmetic at all.
        params = _params(start=date(2020, 1, 15), end=date(2021, 1, 15))
        setattr(params, field, Dezimal(value))

        _on(monkeypatch, _TODAY)
        with pytest.raises(CalculationInputError, match=f"{field} must be a finite"):
            await _calculator().calculate(params)


class TestCalendarBounds:
    @pytest.mark.asyncio
    async def test_rejects_an_end_before_the_start(self):
        params = _params(start=date(2027, 2, 1), end=date(2027, 1, 1))

        with pytest.raises(CalculationInputError):
            await _calculator().calculate(params)

    @pytest.mark.asyncio
    @pytest.mark.parametrize(
        "frequency, start, first_installment",
        [
            (InstallmentFrequency.WEEKLY, date(9999, 12, 20), date(9999, 12, 27)),
            (InstallmentFrequency.MONTHLY, date(9999, 10, 31), date(9999, 11, 30)),
            (InstallmentFrequency.YEARLY, date(9990, 12, 31), date(9991, 12, 31)),
        ],
    )
    async def test_a_schedule_ending_on_the_last_day_of_the_calendar(
        self, frequency, start, first_installment
    ):
        params = _params(
            start=start, end=date(9999, 12, 31), installment_frequency=frequency
        )

        result = await _calculator().calculate(params)

        # The first installment is due one period after the start.
        assert result.installment_date == first_installment

    @pytest.mark.asyncio
    async def test_rejects_a_negative_fixed_period(self):
        params = _params(
            interest_type=InterestType.MIXED,
            euribor_rate=Dezimal("0.01"),
            fixed_years=-2020,
        )

        with pytest.raises(CalculationInputError):
            await _calculator().calculate(params)

    @pytest.mark.asyncio
    async def test_a_fixed_period_beyond_the_calendar_never_ends(self):
        params = _params(
            interest_type=InterestType.MIXED,
            euribor_rate=Dezimal("0.01"),
            fixed_years=9000,
            fixed_interest_rate=Dezimal("0.02"),
        )
        # The fixed rate for the whole loan, computed the same way.
        fixed_only = _params(
            interest_rate=Dezimal("0.02"),
            interest_type=InterestType.VARIABLE,
            euribor_rate=Dezimal(0),
        )

        result = await _calculator().calculate(params)
        expected = await _calculator().calculate(fixed_only)

        assert result == expected

    @pytest.mark.asyncio
    async def test_a_fixed_period_beyond_the_calendar_covers_its_last_day(self):
        # The only installment's period begins on the last day of the calendar,
        # which the fixed period, ending beyond it, still covers.
        params = _params(
            loan_amount=Dezimal(12000),
            interest_rate=Dezimal("0.04"),
            interest_type=InterestType.MIXED,
            euribor_rate=Dezimal("0.01"),
            fixed_years=1,
            fixed_interest_rate=Dezimal("0.02"),
            start=date(9999, 12, 31),
            end=date(9999, 12, 31),
        )

        result = await _calculator().calculate(params)

        # 2 % a year is 20 a month on 12000; the floating 5 % would be 50.
        assert result.current_installment_interests == Dezimal(20)


def _due_dates(start, end, months=1):
    """The schedule by its definition: installment i is due i periods after the
    start, counted from the start each time, the last one on or before the end."""
    dates = []
    index = 1
    while (due := start + relativedelta(months=index * months)) <= end:
        dates.append(due)
        index += 1
    return dates or [end]


class TestSchedule:
    def _next(self, start, end, today, frequency=InstallmentFrequency.MONTHLY):
        return _calculator().next_installment_date(start, end, frequency, today)

    def test_the_first_installment_is_due_one_period_after_the_start(self):
        start, end = date(2025, 1, 15), date(2025, 12, 15)

        assert self._next(start, end, date(2025, 1, 10)) == date(2025, 2, 15)
        assert self._next(start, end, start) == date(2025, 2, 15)

    def test_an_installment_due_today_is_the_current_one(self):
        start, end = date(2025, 1, 15), date(2025, 12, 15)

        assert self._next(start, end, date(2025, 3, 15)) == date(2025, 3, 15)
        assert self._next(start, end, date(2025, 3, 16)) == date(2025, 4, 15)

    def test_month_ends_do_not_drift(self):
        start, end = date(2025, 1, 31), date(2025, 12, 31)

        assert self._next(start, end, date(2025, 2, 1)) == date(2025, 2, 28)
        assert self._next(start, end, date(2025, 3, 1)) == date(2025, 3, 31)
        assert self._next(start, end, date(2025, 4, 1)) == date(2025, 4, 30)

    def test_after_the_last_installment_it_stays_the_current_one(self):
        start, end = date(2025, 1, 15), date(2025, 12, 15)

        assert self._next(start, end, date(2026, 6, 1)) == end

    def test_weekly_installments(self):
        start, end = date(2025, 1, 6), date(2025, 12, 29)

        assert self._next(
            start, end, date(2025, 1, 14), InstallmentFrequency.WEEKLY
        ) == date(2025, 1, 20)

    @pytest.mark.asyncio
    async def test_paid_installments_are_the_ones_due_before_today(self, monkeypatch):
        # Without interest every installment repays the same share.
        today = _TODAY
        _on(monkeypatch, today)
        for start in (
            today - relativedelta(months=3),
            today - relativedelta(months=3, days=1),
            today - timedelta(days=1),
            today + timedelta(days=10),
        ):
            end = start + relativedelta(months=12)
            due = _due_dates(start, end)
            paid = sum(1 for day in due if day < today)
            params = _params(
                loan_amount=Dezimal(12000),
                interest_rate=Dezimal(0),
                start=start,
                end=end,
            )

            result = await _calculator().calculate(params)

            assert result.principal_outstanding == Dezimal(1000) * (12 - paid), start
            assert result.current_installment_payment == Dezimal(1000)
            assert result.installment_date == next(
                (day for day in due if day >= today), due[-1]
            )

    @pytest.mark.asyncio
    async def test_a_month_end_start_counts_every_installment(self, monkeypatch):
        # 31 January to 28 February is one installment, not none.
        today = _TODAY
        _on(monkeypatch, today)
        start = date(today.year + 1, 1, 31)
        params = _params(
            loan_amount=Dezimal(1000),
            interest_rate=Dezimal(0),
            start=start,
            end=date(today.year + 1, 2, 28),
        )

        result = await _calculator().calculate(params)

        assert result.current_installment_payment == Dezimal(1000)
        assert result.installment_date == date(today.year + 1, 2, 28)

    @pytest.mark.asyncio
    async def test_a_simulated_constant_rate_matches_the_closed_form(self, monkeypatch):
        # A variable loan at a constant rate pays the same as a fixed one; the
        # simulation only rounds each installment to cents.
        today = _TODAY
        _on(monkeypatch, today)
        start = today - relativedelta(years=5, days=3)
        end = start + relativedelta(years=30)
        fixed = await _calculator().calculate(
            _params(start=start, end=end, interest_rate=Dezimal("0.03"))
        )
        variable = await _calculator().calculate(
            _params(
                start=start,
                end=end,
                interest_rate=Dezimal("0.03"),
                interest_type=InterestType.VARIABLE,
                euribor_rate=Dezimal(0),
            )
        )

        assert variable.installment_date == fixed.installment_date
        assert abs(variable.principal_outstanding - fixed.principal_outstanding) < (
            Dezimal(1)
        )
        assert abs(
            variable.current_installment_payment - fixed.current_installment_payment
        ) <= Dezimal("0.01")

    @pytest.mark.asyncio
    async def test_a_tracked_loan_continues_from_its_reference(self, monkeypatch):
        # The reference is the outstanding before the installment due on the
        # reference date; one installment later it is one installment less.
        today = _TODAY
        _on(monkeypatch, today)
        start = today - relativedelta(months=6, days=3)
        end = start + relativedelta(months=24)
        due = _due_dates(start, end)
        previous = [day for day in due if day < today][-1]
        params = _params(
            loan_amount=Dezimal(24000),
            interest_rate=Dezimal(0),
            start=start,
            end=end,
        )
        params.tracking_ref_outstanding = Dezimal(19000)
        params.tracking_ref_date = previous

        result = await _calculator().calculate(params)

        assert result.principal_outstanding == Dezimal(18000)

    @pytest.mark.asyncio
    async def test_an_installment_pays_the_rate_of_its_period(self, monkeypatch):
        # The fixed period of 0 % ends on 2026-01-15; the installment due that day
        # still pays the fixed rate for its period, the next one the floating rate.
        params = _params(
            loan_amount=Dezimal(12000),
            interest_type=InterestType.MIXED,
            interest_rate=Dezimal("0.12"),
            euribor_rate=Dezimal(0),
            fixed_interest_rate=Dezimal(0),
            fixed_years=1,
            start=date(2025, 1, 15),
            end=date(2027, 1, 15),
        )

        _on(monkeypatch, date(2026, 1, 15))
        due_on_the_end = await _calculator().calculate(params)
        _on(monkeypatch, date(2026, 1, 16))
        due_after_the_end = await _calculator().calculate(params)

        # Eleven installments of 500 are paid at 0 %.
        assert due_on_the_end.installment_date == date(2026, 1, 15)
        assert due_on_the_end.principal_outstanding == Dezimal(6500)
        assert due_on_the_end.current_installment_interests == Dezimal(0)
        # The twelfth, also at 0 %, is paid; the thirteenth pays 1 % a month.
        assert due_after_the_end.installment_date == date(2026, 2, 15)
        assert due_after_the_end.principal_outstanding == Dezimal(6000)
        assert due_after_the_end.current_installment_interests == Dezimal(60)


class TestRepaidLoan:
    @pytest.mark.asyncio
    @pytest.mark.parametrize(
        "interest_type, euribor_rate",
        [(InterestType.FIXED, None), (InterestType.VARIABLE, Dezimal(0))],
    )
    async def test_a_loan_past_its_last_installment_is_repaid(
        self, monkeypatch, interest_type, euribor_rate
    ):
        params = _params(
            loan_amount=Dezimal(1200),
            interest_rate=Dezimal(0),
            interest_type=interest_type,
            euribor_rate=euribor_rate,
            start=date(2025, 1, 15),
            end=date(2025, 4, 15),
        )

        _on(monkeypatch, date(2025, 4, 15))
        last_due_today = await _calculator().calculate(params)
        _on(monkeypatch, date(2025, 4, 16))
        all_past = await _calculator().calculate(params)

        assert last_due_today.principal_outstanding == Dezimal(400)
        assert last_due_today.current_installment_payment == Dezimal(400)
        assert all_past.installment_date == date(2025, 4, 15)
        assert all_past.principal_outstanding == Dezimal(0)
        assert all_past.current_installment_payment == Dezimal(0)
        assert all_past.current_installment_interests == Dezimal(0)

    @pytest.mark.asyncio
    async def test_a_tracked_loan_past_its_last_installment_is_repaid(
        self, monkeypatch
    ):
        params = _params(
            loan_amount=Dezimal(1200),
            interest_rate=Dezimal("0.12"),
            start=date(2025, 1, 15),
            end=date(2025, 4, 15),
        )
        params.tracking_ref_outstanding = Dezimal(800)
        params.tracking_ref_date = date(2025, 3, 1)

        _on(monkeypatch, date(2025, 5, 1))
        result = await _calculator().calculate(params)

        assert result.principal_outstanding == Dezimal(0)
        assert result.current_installment_payment == Dezimal(0)

    @pytest.mark.asyncio
    async def test_a_loan_tracked_from_after_its_last_installment_is_repaid(
        self, monkeypatch
    ):
        params = _params(
            loan_amount=Dezimal(1200),
            interest_rate=Dezimal("0.12"),
            start=date(2025, 1, 15),
            end=date(2025, 4, 15),
        )
        params.tracking_ref_outstanding = Dezimal(400)
        params.tracking_ref_date = date(2025, 4, 20)

        _on(monkeypatch, date(2025, 5, 1))
        result = await _calculator().calculate(params)

        assert result.principal_outstanding == Dezimal(0)
        assert result.current_installment_payment == Dezimal(0)
        assert result.current_installment_interests == Dezimal(0)

    @pytest.mark.asyncio
    async def test_a_given_outstanding_is_due_in_full_with_the_loan_amount(
        self, monkeypatch
    ):
        # The regular installment would be 400; the 300 left are due at once.
        params = _params(
            loan_amount=Dezimal(1200),
            interest_rate=Dezimal(0),
            principal_outstanding=Dezimal(300),
            start=date(2025, 1, 15),
            end=date(2025, 4, 15),
        )

        _on(monkeypatch, date(2025, 5, 1))
        result = await _calculator().calculate(params)

        assert result.installment_date == date(2025, 4, 15)
        assert result.principal_outstanding == Dezimal(300)
        assert result.current_installment_payment == Dezimal(300)

    @pytest.mark.asyncio
    async def test_a_given_outstanding_is_due_with_the_last_installment(
        self, monkeypatch
    ):
        params = _params(
            loan_amount=None,
            interest_rate=Dezimal(0),
            interest_type=InterestType.VARIABLE,
            euribor_rate=Dezimal(0),
            principal_outstanding=Dezimal(300),
            start=date(2025, 1, 15),
            end=date(2025, 4, 15),
        )

        _on(monkeypatch, date(2025, 5, 1))
        result = await _calculator().calculate(params)

        assert result.installment_date == date(2025, 4, 15)
        assert result.principal_outstanding == Dezimal(300)
        assert result.current_installment_payment == Dezimal(300)
