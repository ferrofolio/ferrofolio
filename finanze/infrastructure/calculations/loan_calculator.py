from __future__ import annotations

from dataclasses import replace
from datetime import date, timedelta
from decimal import ROUND_HALF_UP

from application.ports.loan_calculator_port import LoanCalculatorPort
from dateutil.relativedelta import relativedelta
from domain.dezimal import Dezimal
from domain.exception.exceptions import CalculationInputError, MissingFieldsError
from domain.global_position import InstallmentFrequency, InterestType
from domain.loan_calculator import LoanCalculationParams, LoanCalculationResult

# Months per installment of the schedules that follow the calendar; the others have a
# fixed number of days per installment.
_MONTHS_PER_INSTALLMENT = {
    InstallmentFrequency.MONTHLY: 1,
    InstallmentFrequency.BIMONTHLY: 2,
    InstallmentFrequency.QUARTERLY: 3,
    InstallmentFrequency.SEMIANNUAL: 6,
    InstallmentFrequency.YEARLY: 12,
}
_DAYS_PER_INSTALLMENT = {
    InstallmentFrequency.WEEKLY: 7,
    InstallmentFrequency.BIWEEKLY: 14,
    InstallmentFrequency.SEMIMONTHLY: 15,
}


class _Schedule:
    """The installments of a loan disbursed on `start` and due on `end`.

    Installment i (1, 2, ...) is due i periods after `start`, counted from `start`
    each time, so a schedule starting on 31 January is due on 28 or 29 February
    and then on 31 March. The last installment is the last one on or before `end`;
    a loan that ends before its first period is over has one installment, on `end`.
    """

    def __init__(self, start: date, end: date, frequency: InstallmentFrequency):
        self.start = start
        self.end = end
        self.frequency = frequency
        self.count = max(1, self._due_until(end))

    def date(self, index: int) -> date:
        """The due date of installment `index` (1 to `count`)."""
        shifted = self._shifted(index)
        return self.end if shifted is None else min(shifted, self.end)

    def period_start(self, index: int) -> date:
        """The day the period of installment `index` begins: `start` for the first,
        the due date of the one before for the others."""
        return self.start if index == 1 else self.date(index - 1)

    def paid(self, today: date) -> int:
        """How many installments are due before `today`."""
        if today > self.date(self.count):
            return self.count
        if today <= self.start:
            return 0
        return self._due_until(today - timedelta(days=1))

    def next_index(self, today: date) -> int:
        """The installment due on or after `today`; once all are past, the last one."""
        return min(self.paid(today) + 1, self.count)

    def _shifted(self, periods: int) -> date | None:
        """`start` plus that many periods, or None beyond the end of the calendar."""
        try:
            months = _MONTHS_PER_INSTALLMENT.get(self.frequency)
            if months is not None:
                return self.start + relativedelta(months=periods * months)
            return self.start + timedelta(
                days=periods * _DAYS_PER_INSTALLMENT[self.frequency]
            )
        except (OverflowError, ValueError):
            return None

    def _due_until(self, when: date) -> int:
        """How many installments are due on or before `when`."""
        if when <= self.start:
            return 0
        months = _MONTHS_PER_INSTALLMENT.get(self.frequency)
        if months is None:
            return (when - self.start).days // _DAYS_PER_INSTALLMENT[self.frequency]
        elapsed = relativedelta(when, self.start)
        count = (elapsed.years * 12 + elapsed.months) // months
        # A month end shifted into a shorter month can still be due by `when`.
        while (shifted := self._shifted(count + 1)) is not None and shifted <= when:
            count += 1
        return count


class LoanCalculator(LoanCalculatorPort):
    """
    Loan calculator for FIXED, VARIABLE and MIXED interest types.

    - The loan is disbursed on `start`; installment i is due i periods later, the
      last one on or before `end` (see `_Schedule`).
    - Rates are annual nominal (0.03 = 3% APR). Period rate = annual / payments_per_year.
    - For VARIABLE and MIXED after the fixed period, annual = interest_rate + euribor_rate.
    - For MIXED during the fixed period, annual = fixed_interest_rate (if provided) or interest_rate.
    - The installments due before today are paid; the one due today or next is the
      current installment. Once every installment is past, the last one stays the
      current one, and a loan whose outstanding comes from its amount or a tracking
      reference is repaid: nothing is outstanding and nothing is due. An
      outstanding given in the request is still owed then and due in full.
    - If principal_outstanding is not provided, it is computed from the loan amount:
      in closed form for a fixed rate, otherwise by simulating each paid installment
      with the rate of its period.
    - Installment interests = current outstanding * current period rate.
    """

    async def calculate(self, params: LoanCalculationParams) -> LoanCalculationResult:
        today = date.today()

        self._validate(params)

        # Normalize params to avoid accidental mutation from callers
        p = replace(params)

        schedule = _Schedule(p.start, p.end, p.installment_frequency)
        ppy = p.installment_frequency.payments_per_year

        next_index = schedule.next_index(today)
        # An installment pays interest for its period at the rate in force when the
        # period began, here as in the simulation of the paid installments.
        period_rate = self._to_period_rate(
            self._annual_rate_at(p, schedule.period_start(next_index)), ppy
        )
        next_inst_date = schedule.date(next_index)
        installments = schedule.count
        # Installments due before today, the last one included once it is past.
        paid = schedule.paid(today)
        if paid == installments:
            return self._after_the_last_installment(p, period_rate, next_inst_date)

        if p.interest_type == InterestType.FIXED and p.loan_amount is not None:
            A = self._amortizing_payment(p.loan_amount, period_rate, installments)

            if (
                p.tracking_ref_outstanding is not None
                and p.tracking_ref_date is not None
            ):
                # Tracked loan: the reference outstanding is the balance after the
                # installments due before the reference date.
                ref_paid = schedule.paid(p.tracking_ref_date)
                outstanding = self._remaining_balance(
                    p.tracking_ref_outstanding,
                    period_rate,
                    installments - ref_paid,
                    max(0, paid - ref_paid),
                    A,
                )
            else:
                outstanding = p.principal_outstanding or self._remaining_balance(
                    p.loan_amount, period_rate, installments, paid, A
                )

            interest_part = self._round_cents(outstanding * period_rate)
            return LoanCalculationResult(
                current_installment_payment=self._round_cents(A),
                current_installment_interests=interest_part,
                principal_outstanding=self._round_cents(outstanding),
                installment_date=next_inst_date,
            )

        # Installments still to pay, the current one included.
        remaining_periods = installments - paid

        # FIXED-rate with provided outstanding but no loan_amount: recover original A from RB_k
        if (
            p.interest_type == InterestType.FIXED
            and p.loan_amount is None
            and p.principal_outstanding is not None
        ):
            N = installments
            k = paid
            r = period_rate
            RBk = p.principal_outstanding

            if r == Dezimal(0):
                # Without interest the outstanding is spread over the installments left.
                period_payment = self._round_cents(RBk / remaining_periods)
                return LoanCalculationResult(
                    current_installment_payment=period_payment,
                    current_installment_interests=Dezimal(0),
                    principal_outstanding=RBk,
                    installment_date=next_inst_date,
                )

            D = Dezimal(1) - (Dezimal(1) + r) ** (-N)
            factor = (Dezimal(1) + r) ** k
            coeff = factor - (factor - Dezimal(1)) / D

            if coeff.val == 0:
                # Fallback to the general formula over the installments left
                denom = Dezimal(1) - (Dezimal(1) + r) ** (-remaining_periods)
                period_payment = RBk if denom.val == 0 else RBk * r / denom
                return LoanCalculationResult(
                    current_installment_payment=self._round_cents(period_payment),
                    current_installment_interests=self._round_cents(RBk * r),
                    principal_outstanding=RBk,
                    installment_date=next_inst_date,
                )

            P0 = RBk / coeff
            A = P0 * r / D
            return LoanCalculationResult(
                current_installment_payment=self._round_cents(A),
                current_installment_interests=self._round_cents(RBk * r),
                principal_outstanding=RBk,
                installment_date=next_inst_date,
            )

        # General path (VARIABLE or MIXED): the outstanding is given or simulated
        # over the paid installments, and the current payment spreads it over the
        # installments left.
        outstanding = p.principal_outstanding
        if outstanding is None:
            outstanding = self._compute_outstanding_from_start(p, schedule, paid)

        # If there is only interest (edge case), handle gracefully
        if period_rate == Dezimal(0):
            # Purely principal division over remaining periods
            period_payment = self._round_cents(outstanding / remaining_periods)
            interest_part = Dezimal(0)
            return LoanCalculationResult(
                current_installment_payment=period_payment,
                current_installment_interests=interest_part,
                principal_outstanding=outstanding,
                installment_date=next_inst_date,
            )

        # Standard amortizing payment formula: A = P * r / (1 - (1+r)^-n)
        r = period_rate
        P = outstanding
        n = remaining_periods
        denom = Dezimal(1) - (Dezimal(1) + r) ** (-n)
        if denom.val == 0:
            period_payment = P
        else:
            period_payment = P * r / denom

        # Round to cents (2 decimals) using bank-style rounding
        period_payment = self._round_cents(period_payment)

        interest_part = self._round_cents(outstanding * period_rate)

        return LoanCalculationResult(
            current_installment_payment=period_payment,
            current_installment_interests=interest_part,
            principal_outstanding=outstanding,
            installment_date=next_inst_date,
        )

    def _after_the_last_installment(
        self, p: LoanCalculationParams, period_rate: Dezimal, last_date: date
    ) -> LoanCalculationResult:
        """A loan whose installments are all past. Computed from its amount or a
        tracking reference, it is repaid; an outstanding given in the request is
        still owed and due in full, with the last period's interest."""
        tracked = (
            p.interest_type == InterestType.FIXED
            and p.loan_amount is not None
            and p.tracking_ref_outstanding is not None
            and p.tracking_ref_date is not None
        )
        outstanding = None if tracked else p.principal_outstanding
        if outstanding is None:
            return LoanCalculationResult(
                current_installment_payment=Dezimal(0),
                current_installment_interests=Dezimal(0),
                principal_outstanding=Dezimal(0),
                installment_date=last_date,
            )
        return LoanCalculationResult(
            current_installment_payment=self._round_cents(
                outstanding * (Dezimal(1) + period_rate)
            ),
            current_installment_interests=self._round_cents(outstanding * period_rate),
            principal_outstanding=outstanding,
            installment_date=last_date,
        )

    def _validate(self, params: LoanCalculationParams) -> None:
        missing: list[str] = []
        if params.loan_amount is None and params.principal_outstanding is None:
            missing.append("loan_amount | principal_outstanding (at least one)")
        if params.start is None:
            missing.append("start")
        if params.end is None:
            missing.append("end")
        if params.interest_rate is None:
            missing.append("interest_rate")
        if params.interest_type in (InterestType.VARIABLE, InterestType.MIXED):
            if params.euribor_rate is None:
                missing.append("euribor_rate (required for VARIABLE/MIXED)")
        if params.interest_type is InterestType.MIXED and not params.fixed_years:
            missing.append("fixed_years (required for MIXED)")
        if missing:
            raise MissingFieldsError(missing)
        if params.end < params.start:
            raise CalculationInputError("end must not be before start")
        if params.interest_type is InterestType.MIXED and params.fixed_years < 0:
            raise CalculationInputError("fixed_years must not be negative")

    def _to_period_rate(self, annual: Dezimal, payments_per_year: int) -> Dezimal:
        return annual / payments_per_year

    def _compute_outstanding_from_start(
        self, p: LoanCalculationParams, schedule: _Schedule, paid: int
    ) -> Dezimal:
        """The outstanding after the `paid` first installments, each computed with
        the rate in force when its period began."""
        assert p.loan_amount is not None
        outstanding = p.loan_amount
        ppy = p.installment_frequency.payments_per_year

        for index in range(1, paid + 1):
            period_start = schedule.period_start(index)
            period_rate = self._to_period_rate(
                self._annual_rate_at(p, period_start), ppy
            )
            # The installments left, this one and the last one included.
            remaining_periods = schedule.count - index + 1

            if period_rate == Dezimal(0):
                payment = self._round_cents(outstanding / remaining_periods)
                interest = Dezimal(0)
            else:
                denom = Dezimal(1) - (Dezimal(1) + period_rate) ** (-remaining_periods)
                payment = self._round_cents(outstanding * period_rate / denom)
                interest = self._round_cents(outstanding * period_rate)

            principal_paid = payment - interest
            outstanding = max(Dezimal(0), outstanding - principal_paid)

            if outstanding == Dezimal(0):
                break

        return outstanding

    def _annual_rate_at(self, p: LoanCalculationParams, when: date) -> Dezimal:
        base = p.interest_rate
        if p.interest_type == InterestType.FIXED:
            return base
        if p.interest_type == InterestType.VARIABLE:
            return base + p.euribor_rate  # type: ignore[arg-type]
        # MIXED
        assert p.fixed_years is not None
        try:
            fixed_end = p.start + relativedelta(years=p.fixed_years)
        except (OverflowError, ValueError):
            # A fixed period beyond the end of the calendar never ends.
            fixed_end = None
        if fixed_end is None or when < fixed_end:
            return p.fixed_interest_rate if p.fixed_interest_rate is not None else base
        return base + p.euribor_rate  # type: ignore[arg-type]

    # Finance helpers
    def _amortizing_payment(self, P: Dezimal, r: Dezimal, n: int) -> Dezimal:
        if n <= 0:
            return P
        if r == Dezimal(0):
            return P / n
        denom = Dezimal(1) - (Dezimal(1) + r) ** (-n)
        return P * r / denom

    def _remaining_balance(
        self, P: Dezimal, r: Dezimal, n: int, k: int, A: Dezimal | None = None
    ) -> Dezimal:
        """Remaining balance after k payments on an amortizing loan.
        If A not provided, compute it from P, r, n.
        Formula: RB_k = P*(1+r)^k - A * ((1+r)^k - 1)/r
        """
        if k <= 0:
            return P
        if k >= n:
            return Dezimal(0)
        if r == Dezimal(0):
            A = A or (P / n)
            return max(Dezimal(0), P - A * k)
        A = A or self._amortizing_payment(P, r, n)
        factor = (Dezimal(1) + r) ** k
        return max(Dezimal(0), P * factor - A * ((factor - Dezimal(1)) / r))

    def _round_cents(self, value: Dezimal) -> Dezimal:
        # Use Decimal quantize via underlying decimal to 2 places, ROUND_HALF_UP
        q = value.val.quantize(Dezimal(0.01).val, rounding=ROUND_HALF_UP)
        return Dezimal(q)

    def next_installment_date(
        self, start: date, end: date, frequency: InstallmentFrequency, today: date
    ) -> date:
        schedule = _Schedule(start, end, frequency)
        return schedule.date(schedule.next_index(today))
