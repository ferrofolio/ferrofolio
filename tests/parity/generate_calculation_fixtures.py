# SPDX-License-Identifier: AGPL-3.0-only
"""Generate the parity fixtures for the calculator routes ported to Rust.

The Python routes are the reference: every case is sent through the real Quart
routes with a fixed date, and the status and JSON body of each response are
recorded. The Rust tests in crates/server/tests/parity_calculations.rs replay the
cases and require the same responses.

    python tests/parity/generate_calculation_fixtures.py --output FILE
    python tests/parity/generate_calculation_fixtures.py --check FILE

--check exits with status 1 when FILE does not match what the Python code
produces now, so a change to the Python calculators cannot drift from Rust.
"""

import argparse
import asyncio
import hashlib
from decimal import Decimal
import json
import logging
import random
import sys
import tempfile
from datetime import date, timedelta
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / "finanze"))

from application.use_cases.authorize_api_access import (  # noqa: E402
    AuthorizeApiAccessImpl,
)
from application.use_cases.calculate_loan import CalculateLoanImpl  # noqa: E402
from application.use_cases.calculate_savings import CalculateSavingsImpl  # noqa: E402
from infrastructure.calculations import loan_calculator as loan_module  # noqa: E402
from infrastructure.controller.api_session_guard import (  # noqa: E402
    register_api_session_guard,
)
from infrastructure.controller.config import quart  # noqa: E402
from infrastructure.controller.routes.calculate_loan import calculate_loan  # noqa: E402
from infrastructure.controller.routes.calculate_savings import (  # noqa: E402
    calculate_savings,
)

TODAY = date(2026, 11, 18)
SEED = 20261118
LOAN_PATH = "/api/v1/calculation/loan"
SAVINGS_PATH = "/api/v1/calculations/savings"

FREQUENCIES = [
    "WEEKLY",
    "BIWEEKLY",
    "SEMIMONTHLY",
    "MONTHLY",
    "BIMONTHLY",
    "QUARTERLY",
    "SEMIANNUAL",
    "YEARLY",
]


class FixedDate(date):
    @classmethod
    def today(cls):
        return TODAY


# The token of the session every case sends unless it says session=False.
SESSION_TOKEN = "parity-session"


class Backend:
    """The state the production session guard asks for: whether the data is
    unlocked, and which session tokens are valid."""

    unlocked = True

    def is_valid(self, token):
        return token == SESSION_TOKEN


BACKEND = Backend()


def build_app(static_dir: Path):
    loan_module.date = FixedDate
    app = quart(static_dir)
    register_api_session_guard(app, AuthorizeApiAccessImpl(BACKEND, BACKEND))
    # Some cases provoke unexpected errors on purpose; their tracebacks are noise.
    app.logger.setLevel(logging.CRITICAL)
    loan_uc = CalculateLoanImpl(loan_module.LoanCalculator())
    savings_uc = CalculateSavingsImpl()

    @app.route(LOAN_PATH, methods=["POST"])
    async def loan_route():
        return await calculate_loan(loan_uc)

    @app.route(SAVINGS_PATH, methods=["POST"])
    async def savings_route():
        return await calculate_savings(savings_uc)

    return app


def case(
    name,
    path,
    body,
    compare="full",
    raw=None,
    content_type=None,
    deviation=None,
    session=True,
    unlocked=None,
    pad=0,
):
    """A request and how the Rust response is compared with the Python one.

    compare is "full" (status and body), "status_code" (status and error code, for
    messages that come from pydantic), "status" (non-JSON responses) or
    "deviation", where Rust deliberately answers differently; deviation then
    holds the reason and the status and code Rust returns. session=False sends the
    request without a session token; unlocked says whether the data is unlocked,
    by default exactly when a token is sent. pad appends that many spaces to a raw
    body, which JSON ignores.
    """
    prefix = "loan" if path == LOAN_PATH else "savings"
    entry = {"name": f"{prefix}: {name}", "path": path, "compare": compare}
    if not session:
        entry["session"] = False
    if unlocked is not None:
        entry["unlocked"] = unlocked
    if pad:
        entry["pad_bytes"] = pad
    if deviation is not None:
        entry["deviation"] = deviation
    if raw is not None:
        entry["raw_body"] = raw
        entry["content_type"] = content_type or "application/json"
    else:
        entry["body"] = body
        if content_type:
            entry["content_type"] = content_type
    return entry


def loan(**fields):
    body = {
        "loan_amount": 200000,
        "interest_rate": 0.03,
        "interest_type": "FIXED",
        "start": "2020-01-15",
        "end": "2050-01-15",
    }
    body.update(fields)
    return {key: value for key, value in body.items() if value is not ...}


FLOAT_OVERFLOW = {
    "reason": "Python reads an integer field beyond the float range as infinity and "
    "fails with an unexpected error; Rust reports the invalid value",
    "status": 400,
    "code": "INVALID_REQUEST",
}

NON_JSON_LITERAL = {
    "reason": "Python's json module accepts NaN and Infinity, which are not JSON, and "
    "the loan calculator rejects the value as not finite; Rust rejects the body",
    "status": 400,
    "code": "INVALID_REQUEST",
}


def loan_cases():
    cases = [
        case("fixed monthly", LOAN_PATH, loan()),
        case("fixed with outstanding", LOAN_PATH, loan(principal_outstanding=150000)),
        case(
            "fixed outstanding only",
            LOAN_PATH,
            loan(loan_amount=..., principal_outstanding=123456.78),
        ),
        case(
            "fixed outstanding only zero rate",
            LOAN_PATH,
            loan(loan_amount=..., principal_outstanding=5000, interest_rate=0),
        ),
        case(
            "variable simulated from start",
            LOAN_PATH,
            loan(interest_type="VARIABLE", interest_rate=0.009, euribor_rate=0.025),
        ),
        case(
            "variable with outstanding",
            LOAN_PATH,
            loan(
                interest_type="VARIABLE",
                interest_rate="0.0099",
                euribor_rate="0.0231",
                principal_outstanding="98765.43",
            ),
        ),
        case(
            "mixed in fixed period",
            LOAN_PATH,
            loan(
                interest_type="MIXED",
                start="2022-03-01",
                end="2052-03-01",
                fixed_years=10,
                fixed_interest_rate=0.0185,
                interest_rate=0.008,
                euribor_rate=0.03,
            ),
        ),
        case(
            "mixed after fixed period",
            LOAN_PATH,
            loan(
                interest_type="MIXED",
                start="2012-03-01",
                end="2042-03-01",
                fixed_years=5,
                interest_rate=0.008,
                euribor_rate=0.03,
            ),
        ),
        case(
            "mixed without fixed rate",
            LOAN_PATH,
            loan(
                interest_type="MIXED",
                start="2024-06-30",
                fixed_years="3",
                interest_rate=0.021,
                euribor_rate=-0.004,
            ),
        ),
        case("start in the future", LOAN_PATH, loan(start="2027-01-01")),
        case(
            "end in the past",
            LOAN_PATH,
            loan(start="2001-01-31", end="2021-01-31"),
        ),
        case(
            "month end start",
            LOAN_PATH,
            loan(
                start="2021-01-31",
                end="2041-01-31",
                interest_type="VARIABLE",
                euribor_rate=0.01,
            ),
        ),
        case("leap day start", LOAN_PATH, loan(start="2024-02-29", end="2044-02-29")),
        case("zero rate fixed", LOAN_PATH, loan(interest_rate=0)),
        case(
            "zero rate variable",
            LOAN_PATH,
            loan(interest_type="VARIABLE", interest_rate=0, euribor_rate=0),
        ),
        case("compact dates", LOAN_PATH, loan(start="20200115", end="20500115")),
        case("week dates", LOAN_PATH, loan(start="2020-W03-3", end="2050W033")),
        case(
            "float years",
            LOAN_PATH,
            loan(interest_type="MIXED", fixed_years=4.9, euribor_rate=0.02),
        ),
        case("empty frequency", LOAN_PATH, loan(installment_frequency="")),
        case("boolean amount", LOAN_PATH, loan(loan_amount=True)),
        case("missing interest rate", LOAN_PATH, loan(interest_rate=...)),
        case("missing interest type", LOAN_PATH, loan(interest_type=...)),
        case("missing start", LOAN_PATH, loan(start=...)),
        case("missing end", LOAN_PATH, loan(end=...)),
        case("null start", LOAN_PATH, loan(start=None)),
        case("invalid interest type", LOAN_PATH, loan(interest_type="fixed")),
        case("null interest type", LOAN_PATH, loan(interest_type=None)),
        case("invalid frequency", LOAN_PATH, loan(installment_frequency="DAILY")),
        case("invalid date", LOAN_PATH, loan(start="2020-02-30")),
        # CPython reads the first eight characters of a ten-character compact date.
        case(
            "compact date with two extra characters",
            LOAN_PATH,
            loan(start="20200115xx"),
        ),
        case(
            "compact week date with two extra characters",
            LOAN_PATH,
            loan(start="2020W031xx"),
        ),
        case(
            "monthly schedule ending on the last day of the calendar",
            LOAN_PATH,
            loan(start="9999-10-31", end="9999-12-31"),
        ),
        case(
            "fixed period beyond the end of the calendar",
            LOAN_PATH,
            loan(
                interest_type="MIXED",
                euribor_rate=0.01,
                fixed_years=9000,
                fixed_interest_rate=0.02,
            ),
        ),
        case(
            # The only period begins on the calendar's last day, which a fixed
            # period ending beyond the calendar still covers.
            "fixed period beyond the calendar on its last day",
            LOAN_PATH,
            loan(
                interest_type="MIXED",
                euribor_rate=0.01,
                fixed_years=1,
                fixed_interest_rate=0.02,
                start="9999-12-31",
                end="9999-12-31",
            ),
        ),
        # The schedule (TODAY is 2026-11-18): installment i is due i periods after
        # the start; the ones due before today are paid.
        case(
            "installment due today",
            LOAN_PATH,
            loan(start="2026-08-18", end="2027-08-18", interest_rate=0),
        ),
        case(
            "installment due yesterday",
            LOAN_PATH,
            loan(start="2026-08-17", end="2027-08-17", interest_rate=0),
        ),
        case(
            "loan starting today",
            LOAN_PATH,
            loan(start="2026-11-18", end="2027-11-18"),
        ),
        case(
            "loan starting next month",
            LOAN_PATH,
            loan(start="2026-12-18", end="2027-12-18"),
        ),
        case(
            "month-end start",
            LOAN_PATH,
            loan(start="2026-01-31", end="2027-01-31"),
        ),
        case(
            "month-end start, variable",
            LOAN_PATH,
            loan(
                start="2026-01-31",
                end="2027-01-31",
                interest_type="VARIABLE",
                euribor_rate=0.01,
            ),
        ),
        case(
            "payment recovered from the outstanding on a month-end schedule",
            LOAN_PATH,
            loan(
                start="2025-11-30",
                end="2027-11-30",
                loan_amount=...,
                principal_outstanding=150000,
            ),
        ),
        case(
            # The fixed period of 0 % ends today, on a due date: the installment due
            # today still pays the fixed rate for its period.
            "mixed loan whose fixed period ends today",
            LOAN_PATH,
            loan(
                loan_amount=12000,
                start="2025-11-18",
                end="2027-11-18",
                interest_type="MIXED",
                interest_rate=0.12,
                euribor_rate=0,
                fixed_interest_rate=0,
                fixed_years=1,
            ),
        ),
        case(
            "mixed loan whose fixed period ended yesterday",
            LOAN_PATH,
            loan(
                loan_amount=12000,
                start="2025-11-17",
                end="2027-11-17",
                interest_type="MIXED",
                interest_rate=0.12,
                euribor_rate=0,
                fixed_interest_rate=0,
                fixed_years=1,
            ),
        ),
        # Once every installment is past, a loan computed from its amount is
        # repaid; a given outstanding stays due with the last installment.
        case(
            "loan past its last installment",
            LOAN_PATH,
            loan(start="2024-01-15", end="2025-01-15"),
        ),
        case(
            "variable loan past its last installment",
            LOAN_PATH,
            loan(
                start="2024-01-15",
                end="2025-01-15",
                interest_type="VARIABLE",
                euribor_rate=0.01,
            ),
        ),
        case(
            "given outstanding past the last installment",
            LOAN_PATH,
            loan(
                loan_amount=...,
                principal_outstanding=300,
                start="2024-01-15",
                end="2025-01-15",
                interest_type="VARIABLE",
                euribor_rate=0.01,
            ),
        ),
        case(
            "given outstanding and amount past the last installment",
            LOAN_PATH,
            loan(
                principal_outstanding=300,
                start="2024-01-15",
                end="2025-01-15",
            ),
        ),
        case(
            "last installment due today",
            LOAN_PATH,
            loan(start="2025-11-18", end="2026-11-18", interest_rate=0),
        ),
        case(
            "negative fixed period",
            LOAN_PATH,
            loan(interest_type="MIXED", euribor_rate=0.01, fixed_years=-2020),
        ),
        case(
            "loan ending on its start date",
            LOAN_PATH,
            loan(start="2027-02-01", end="2027-02-01"),
        ),
        case(
            "weekly schedule ending on the last day of the calendar",
            LOAN_PATH,
            loan(
                installment_frequency="WEEKLY",
                start="9999-12-20",
                end="9999-12-31",
            ),
        ),
        case(
            "no session while the data is locked",
            LOAN_PATH,
            loan(),
            session=False,
            compare="deviation",
            deviation={
                "reason": "Python's session guard lets every request through while the "
                "data is locked, so the route computes; ported routes always need an "
                "unlocked backend and a session",
                "status": 401,
                "code": "NOT_LOGGED",
            },
        ),
        case(
            "no session while the data is unlocked",
            LOAN_PATH,
            loan(),
            session=False,
            unlocked=True,
        ),
        case(
            "session while the data is locked",
            LOAN_PATH,
            loan(),
            unlocked=False,
            compare="deviation",
            deviation={
                "reason": "Python's session guard lets every request through while the "
                "data is locked; ported routes always need an unlocked backend",
                "status": 401,
                "code": "NOT_LOGGED",
            },
        ),
        case(
            "body above one MiB",
            LOAN_PATH,
            None,
            raw='{"loan_amount": 200000, "interest_rate": 0.03, '
            '"interest_type": "FIXED", "start": "2020-01-15", "end": "2050-01-15"}',
            pad=1024 * 1024,
            compare="deviation",
            deviation={
                "reason": "Python accepts bodies up to 50 MB; a decoded JSON value takes "
                "many times the memory of its text, so ported routes refuse bodies "
                "above 1 MiB",
                "status": 413,
                "code": "PAYLOAD_TOO_LARGE",
            },
        ),
        case(
            "first key equal to serde_json's private number token",
            LOAN_PATH,
            None,
            raw='{"$serde_json::private::Number": "1", "loan_amount": 200000, '
            '"interest_rate": 0.03, "interest_type": "FIXED", '
            '"start": "2020-01-15", "end": "2050-01-15"}',
            compare="deviation",
            deviation={
                "reason": "Python ignores the unknown field; serde_json reads an object "
                "whose first key is its private number token as a number",
                "status": 400,
                "code": "INVALID_REQUEST",
            },
        ),
        case(
            "numeric start date",
            LOAN_PATH,
            loan(start=20200115),
        ),
        case(
            "interest type with a non-breaking space",
            LOAN_PATH,
            loan(interest_type="FIXED\u00a0"),
        ),
        case(
            "amount with more digits than a float holds",
            LOAN_PATH,
            None,
            raw='{"loan_amount": 12345678901234567.89, "interest_rate": 0.03, '
            '"interest_type": "FIXED", "start": "2020-01-15", "end": "2050-01-15"}',
            compare="deviation",
            deviation={
                "reason": "Python reads a JSON number with a fraction as a float, which "
                "keeps about 17 digits; Rust keeps every digit, since money is never a float",
                "status": 200,
                # Rust must answer like Python does for the exact amount.
                "equivalent_request": loan(loan_amount="12345678901234567.89"),
            },
        ),
        case(
            "amount in exponent notation with more digits than a float holds",
            LOAN_PATH,
            None,
            raw='{"loan_amount": 9007199254740993e0, "interest_rate": 0.03, '
            '"interest_type": "FIXED", "start": "2020-01-15", "end": "2050-01-15"}',
            compare="deviation",
            deviation={
                "reason": "Python reads a JSON number with an exponent as a float, which "
                "cannot hold 2^53 + 1; Rust keeps every digit, since money is never a float",
                "status": 200,
                "equivalent_request": loan(loan_amount="9007199254740993"),
            },
        ),
        case(
            "fixed years beyond the float range",
            LOAN_PATH,
            None,
            raw='{"loan_amount": 200000, "interest_rate": 0.03, "interest_type": "MIXED", '
            '"euribor_rate": 0.01, "fixed_years": 1e400, '
            '"start": "2020-01-15", "end": "2050-01-15"}',
            compare="deviation",
            deviation=FLOAT_OVERFLOW,
        ),
        case(
            "ignored field nested deeper than 128 levels",
            LOAN_PATH,
            None,
            raw='{"loan_amount": 200000, "interest_rate": 0.03, "interest_type": "FIXED", '
            '"start": "2020-01-15", "end": "2050-01-15", "extra": '
            + "[" * 130
            + "]" * 130
            + "}",
            compare="deviation",
            deviation={
                "reason": "Python ignores the unknown field; serde_json stops at 128 "
                "levels of nesting, so that a crafted body cannot exhaust the stack",
                "status": 400,
                "code": "INVALID_REQUEST",
            },
        ),
        case(
            "NaN literal as an amount",
            LOAN_PATH,
            None,
            raw='{"loan_amount": NaN, "interest_rate": 0.03, "interest_type": "FIXED", '
            '"start": "2020-01-15", "end": "2050-01-15"}',
            compare="deviation",
            deviation=NON_JSON_LITERAL,
        ),
        case(
            "Infinity literal as a rate",
            LOAN_PATH,
            None,
            raw='{"loan_amount": 200000, "interest_rate": Infinity, '
            '"interest_type": "FIXED", "start": "2020-01-15", "end": "2050-01-15"}',
            compare="deviation",
            deviation=NON_JSON_LITERAL,
        ),
        case(
            "amount with an exponent beyond 32 bits",
            LOAN_PATH,
            loan(loan_amount="1e-2147483649"),
            compare="deviation",
            deviation={
                "reason": "Python's Decimal takes any exponent; Dezimal keeps exponents "
                "within 32 bits and rejects the amount",
                "status": 400,
                "code": "INVALID_REQUEST",
            },
        ),
        case(
            "fixed loan ending before it starts",
            LOAN_PATH,
            loan(interest_rate=0, start="2027-02-01", end="2027-01-01"),
        ),
        case(
            "fixed years beyond 64 bits",
            LOAN_PATH,
            None,
            raw='{"loan_amount": 200000, "interest_rate": 0.03, "interest_type": "MIXED", '
            '"euribor_rate": 0.01, "fixed_years": 100000000000000000000, '
            '"start": "2020-01-15", "end": "2050-01-15"}',
            compare="deviation",
            deviation={
                "reason": "Python's integers are unbounded, and a fixed period beyond the "
                "end of the calendar never ends; Rust rejects integers beyond 64 bits",
                "status": 400,
                "code": "INVALID_REQUEST",
            },
        ),
        case("invalid amount", LOAN_PATH, loan(loan_amount="lots")),
        case("null interest rate", LOAN_PATH, loan(interest_rate=None)),
        case("list amount", LOAN_PATH, loan(loan_amount=[1])),
        case(
            "invalid years", LOAN_PATH, loan(interest_type="MIXED", fixed_years="ten")
        ),
        case(
            "no amounts",
            LOAN_PATH,
            loan(loan_amount=..., interest_type="MIXED", euribor_rate=...),
        ),
        case("variable without euribor", LOAN_PATH, loan(interest_type="VARIABLE")),
        case(
            "mixed with zero fixed years",
            LOAN_PATH,
            loan(interest_type="MIXED", fixed_years=0, euribor_rate=0.01),
        ),
        case(
            "full period rate of minus one",
            LOAN_PATH,
            loan(interest_rate=-12),
            compare="deviation",
            deviation={
                "reason": "Python raises zero to a negative power to Infinity and returns "
                "zeros; Rust has no infinite amounts and reports a division by zero",
                "status": 500,
                "code": "UNEXPECTED_ERROR",
            },
        ),
        case(
            "fixed loan right after its first installment",
            LOAN_PATH,
            loan(
                loan_amount=1200, interest_rate=0, start="2026-11-15", end="2027-11-15"
            ),
        ),
        case(
            "amounts with underscores and other digits",
            LOAN_PATH,
            loan(loan_amount="2٠٠_٠٠٠", interest_rate="0.0_3"),
        ),
        case(
            "amount with more digits than the context precision",
            LOAN_PATH,
            loan(loan_amount="200000.0049999999999999999999999"),
        ),
        case(
            "amount with more than 38 significant digits",
            LOAN_PATH,
            loan(loan_amount="200000.00499999999999999999999999999999999"),
            compare="deviation",
            deviation={
                "reason": "Python keeps every digit of an amount; Rust keeps up to 38 "
                "significant digits and rejects longer amounts instead of rounding them",
                "status": 400,
                "code": "INVALID_REQUEST",
            },
        ),
        # A period rate of 100 %: (1 + r)^-n is far below the smallest amount.
        case(
            "extreme fixed rate",
            LOAN_PATH,
            loan(interest_rate=12),
        ),
        case(
            "extreme variable rate",
            LOAN_PATH,
            loan(interest_type="VARIABLE", interest_rate=6, euribor_rate=6),
        ),
        case("empty body", LOAN_PATH, {}),
        case("array body", LOAN_PATH, []),
        case(
            "body without json content type",
            LOAN_PATH,
            None,
            raw=json.dumps(loan()),
            content_type="text/plain",
        ),
        case(
            "malformed json",
            LOAN_PATH,
            None,
            raw="{not json",
            compare="deviation",
            deviation={
                "reason": "Quart answers malformed JSON with an HTML error page; Rust "
                "answers with a JSON error like every other invalid request",
                "status": 400,
                "code": "INVALID_REQUEST",
            },
        ),
    ]
    for frequency in FREQUENCIES:
        cases.append(
            case(
                f"fixed frequency {frequency.lower()}",
                LOAN_PATH,
                loan(installment_frequency=frequency),
            )
        )
        cases.append(
            case(
                f"variable frequency {frequency.lower()}",
                LOAN_PATH,
                loan(
                    installment_frequency=frequency,
                    interest_type="VARIABLE",
                    euribor_rate=0.0215,
                    start="2016-10-31",
                    end="2036-10-31",
                ),
            )
        )

    rng = random.Random(SEED)
    for index in range(150):
        cases.append(case(f"random {index}", LOAN_PATH, random_loan(rng)))
    return cases


def random_amount(rng, low, high):
    """An amount drawn as whole cents, sent as a JSON number, a string or an integer.

    The JSON number is the float whose shortest representation is the exact amount,
    which is what a client sends.
    """
    value = Decimal(rng.randint(low * 100, high * 100)).scaleb(-2)
    return rng.choice([float(value), str(value), int(value)])


def random_rate(rng, low, high):
    """A rate drawn in steps of 0.0001, sent as a JSON number or a string."""
    value = Decimal(rng.randint(round(low * 10000), round(high * 10000))).scaleb(-4)
    return rng.choice([float(value), str(value)])


def random_date(rng, start, end):
    return start + timedelta(days=rng.randint(0, (end - start).days))


def random_loan(rng):
    interest_type = rng.choice(["FIXED", "VARIABLE", "MIXED"])
    start = random_date(rng, date(1995, 1, 1), date(2028, 12, 31))
    end = start + timedelta(days=rng.randint(200, 40 * 366))
    body = {
        "interest_rate": random_rate(rng, 0, 0.07),
        "interest_type": interest_type,
        "start": start.isoformat(),
        "end": end.isoformat(),
    }
    frequency = rng.choice(FREQUENCIES + [None])
    if frequency:
        body["installment_frequency"] = frequency
    has_amount = rng.random() < 0.8
    if has_amount:
        body["loan_amount"] = random_amount(rng, 1000, 600000)
    if not has_amount or rng.random() < 0.3:
        body["principal_outstanding"] = random_amount(rng, 500, 400000)
    if interest_type in ("VARIABLE", "MIXED"):
        body["euribor_rate"] = random_rate(rng, -0.005, 0.04)
    if interest_type == "MIXED":
        body["fixed_years"] = rng.randint(1, 15)
        if rng.random() < 0.7:
            body["fixed_interest_rate"] = random_rate(rng, 0.005, 0.05)
    return body


def savings(**fields):
    body = {
        "base_amount": 10000,
        "years": 10,
        "periodicity": "MONTHLY",
        "scenarios": [
            {
                "id": "base",
                "annual_market_performance": 0.05,
                "periodic_contribution": 300,
            },
        ],
    }
    body.update(fields)
    return {key: value for key, value in body.items() if value is not ...}


def scenario(identifier, performance, **fields):
    entry = {"id": identifier, "annual_market_performance": performance}
    entry.update(fields)
    return entry


HORIZON_LIMIT = {
    "reason": "Rust rejects more than 200 years or 50 scenarios, which let a single "
    "request exhaust memory and CPU; Python has no limit",
    "status": 400,
    "code": "INVALID_REQUEST",
}


def savings_cases():
    cases = [
        case("monthly contributions", SAVINGS_PATH, savings()),
        case(
            "several scenarios",
            SAVINGS_PATH,
            savings(
                years=5,
                periodicity="QUARTERLY",
                scenarios=[
                    scenario("low", 0.02, periodic_contribution=1000),
                    scenario("mid", "0.05", periodic_contribution="1000"),
                    scenario("high", 0.09, periodic_contribution=1000.5),
                ],
            ),
        ),
        case(
            "target with years",
            SAVINGS_PATH,
            savings(scenarios=[scenario("goal", 0.06, target_amount=150000)]),
        ),
        case(
            "target without years",
            SAVINGS_PATH,
            savings(
                years=...,
                periodicity="YEARLY",
                scenarios=[
                    scenario(
                        "goal", 0.04, periodic_contribution=5000, target_amount=100000
                    )
                ],
            ),
        ),
        case(
            "target solved for periods and contribution",
            SAVINGS_PATH,
            savings(
                years=None,
                periodicity="YEARLY",
                scenarios=[scenario("goal", 0.05, target_amount=40000)],
            ),
        ),
        case(
            "negative base without years or target",
            SAVINGS_PATH,
            savings(
                base_amount=-100,
                years=None,
                periodicity="YEARLY",
                scenarios=[scenario("debt", 0, periodic_contribution=25)],
            ),
        ),
        case(
            "retirement with a rate too small to change one plus the rate",
            SAVINGS_PATH,
            savings(
                base_amount=0,
                years=1,
                periodicity="YEARLY",
                scenarios=[scenario("tiny", "1e-30")],
                retirement={"withdrawal_amount": 100, "withdrawal_years": 1},
            ),
        ),
        case(
            "target with a rate too small to change one plus the rate",
            SAVINGS_PATH,
            savings(
                base_amount=0,
                years=1,
                periodicity="YEARLY",
                scenarios=[scenario("tiny", "1e-30", target_amount=100)],
            ),
        ),
        case(
            "withdrawal years beyond 64 bits",
            SAVINGS_PATH,
            None,
            raw='{"base_amount": 1000, "years": 1, "periodicity": "YEARLY", '
            '"scenarios": [{"id": "a", "annual_market_performance": 0.05, '
            '"periodic_contribution": 100}], '
            '"retirement": {"withdrawal_amount": 1000000, '
            '"withdrawal_years": 100000000000000000000}}',
            compare="deviation",
            deviation={
                "reason": "Python's integers are unbounded; Rust rejects integers "
                "beyond 64 bits",
                "status": 400,
                "code": "INVALID_REQUEST",
            },
        ),
        case(
            "rate beyond the float range",
            SAVINGS_PATH,
            savings(
                years=1,
                periodicity="YEARLY",
                scenarios=[scenario("huge", "1e400", periodic_contribution=100)],
            ),
        ),
        case(
            "rate beyond the float range in the response",
            SAVINGS_PATH,
            savings(
                base_amount=0,
                years=1,
                periodicity="YEARLY",
                scenarios=[scenario("huge", "1e400", periodic_contribution=0)],
            ),
            compare="deviation",
            deviation={
                "reason": "Python writes the rate as Infinity, which is not JSON; Rust "
                "fails instead of writing a value that is not a JSON number",
                "status": 500,
                "code": "UNEXPECTED_ERROR",
            },
        ),
        case(
            "years beyond the float range",
            SAVINGS_PATH,
            None,
            raw='{"base_amount": 1000, "years": 1e400, "periodicity": "YEARLY", '
            '"scenarios": [{"id": "a", "annual_market_performance": 0.05, '
            '"periodic_contribution": 100}]}',
            compare="deviation",
            deviation=FLOAT_OVERFLOW,
        ),
        case(
            "zero rate target",
            SAVINGS_PATH,
            savings(years=4, scenarios=[scenario("flat", 0, target_amount=58000)]),
        ),
        case(
            "negative performance",
            SAVINGS_PATH,
            savings(
                years=3, scenarios=[scenario("bear", -0.08, periodic_contribution=250)]
            ),
        ),
        case(
            "zero target means none",
            SAVINGS_PATH,
            savings(scenarios=[scenario("none", 0.03, target_amount=0)]),
        ),
        case(
            # Python reads the JSON number as a float, which underflows to 0.0 and is
            # falsy, so the target is dropped; Rust tests the same float.
            "target below the float range means none",
            SAVINGS_PATH,
            None,
            raw='{"base_amount": 10000, "years": 10, "periodicity": "MONTHLY", '
            '"scenarios": [{"id": "none", "annual_market_performance": 0.03, '
            '"target_amount": 1e-400}]}',
        ),
        case(
            "retirement with amount and years",
            SAVINGS_PATH,
            savings(
                years=6,
                periodicity="QUARTERLY",
                scenarios=[scenario("plan", 0.05, periodic_contribution=1500)],
                retirement={"withdrawal_amount": 2000, "withdrawal_years": 4},
            ),
        ),
        case(
            "retirement solves the withdrawal",
            SAVINGS_PATH,
            savings(
                years=8,
                periodicity="YEARLY",
                scenarios=[scenario("plan", 0.04, periodic_contribution=6000)],
                retirement={"withdrawal_years": 12},
            ),
        ),
        case(
            "retirement solves the contribution",
            SAVINGS_PATH,
            savings(
                base_amount=5000,
                years=7,
                periodicity="QUARTERLY",
                scenarios=[scenario("plan", 0.06)],
                retirement={"withdrawal_amount": 3000, "withdrawal_years": 5},
            ),
        ),
        case(
            "retirement contribution when the base already funds it",
            SAVINGS_PATH,
            savings(
                base_amount=1000000,
                years=10,
                scenarios=[scenario("rich", 0.05)],
                retirement={"withdrawal_amount": 100, "withdrawal_years": 5},
            ),
        ),
        case(
            "retirement contribution above the first search bound",
            SAVINGS_PATH,
            savings(
                base_amount=0,
                years=1,
                periodicity="YEARLY",
                scenarios=[scenario("late", -0.5)],
                retirement={"withdrawal_amount": 1000, "withdrawal_years": 10},
            ),
        ),
        case(
            # 0.10 at 3 % a year grows past 30 in 200 years, but each return rounds
            # to no return at all, so the projection needs contributions.
            "target the base reaches only without rounding",
            SAVINGS_PATH,
            savings(
                base_amount="0.10",
                years=200,
                periodicity="YEARLY",
                scenarios=[scenario("dust", 0.03, target_amount=30)],
            ),
        ),
        case(
            # The estimate 0.125 rounds to 0.13, but the return on 0.12 rounds to
            # zero, so 0.12 reaches the target too.
            "target a contribution below the estimate reaches",
            SAVINGS_PATH,
            savings(
                base_amount=0,
                years=1,
                periodicity="YEARLY",
                scenarios=[scenario("below", "-0.04", target_amount="0.12")],
            ),
        ),
        case(
            # The bisection settles on 0.26, but 0.25 funds both withdrawals too.
            "retirement a contribution below the estimate funds",
            SAVINGS_PATH,
            savings(
                base_amount=0,
                years=1,
                periodicity="YEARLY",
                scenarios=[scenario("below", "-0.04")],
                retirement={"withdrawal_amount": "0.12", "withdrawal_years": 2},
            ),
        ),
        case(
            "negative withdrawal",
            SAVINGS_PATH,
            savings(
                scenarios=[scenario("plan", 0.05, periodic_contribution=100)],
                retirement={"withdrawal_amount": -100, "withdrawal_years": 5},
            ),
        ),
        case(
            "target with a negative return",
            SAVINGS_PATH,
            savings(
                base_amount=1000,
                years=5,
                scenarios=[scenario("down", -0.04, target_amount=10000)],
            ),
        ),
        case(
            "target the rounded returns never reach",
            SAVINGS_PATH,
            savings(
                base_amount="0.10",
                years=...,
                scenarios=[
                    scenario(
                        "dust", 0.06, periodic_contribution=0, target_amount="0.11"
                    )
                ],
            ),
        ),
        case(
            "retirement solves the contribution at zero rate",
            SAVINGS_PATH,
            savings(
                years=5,
                periodicity="YEARLY",
                scenarios=[scenario("plan", 0)],
                retirement={"withdrawal_amount": 4000, "withdrawal_years": "6"},
            ),
        ),
        case(
            "retirement years as a zero-fraction string",
            SAVINGS_PATH,
            savings(
                years=2,
                periodicity="YEARLY",
                scenarios=[scenario("plan", 0.03, periodic_contribution=2000)],
                retirement={"withdrawal_years": "3.0"},
            ),
        ),
        case(
            "retirement covered by the base amount",
            SAVINGS_PATH,
            savings(
                base_amount=500000,
                years=3,
                periodicity="YEARLY",
                scenarios=[scenario("rich", 0.03)],
                retirement={"withdrawal_amount": 1000, "withdrawal_years": 2.0},
            ),
        ),
        case(
            "retirement amount only",
            SAVINGS_PATH,
            savings(
                years=2,
                periodicity="YEARLY",
                scenarios=[scenario("plan", 0.05, periodic_contribution=10000)],
                retirement={"withdrawal_amount": 9000},
            ),
        ),
        case(
            "retirement with zero withdrawal",
            SAVINGS_PATH,
            savings(
                years=1,
                periodicity="YEARLY",
                scenarios=[scenario("plan", 0.05, periodic_contribution=100)],
                retirement={"withdrawal_amount": 0},
            ),
        ),
        case("missing base amount", SAVINGS_PATH, savings(base_amount=...)),
        case("null base amount", SAVINGS_PATH, savings(base_amount=None)),
        case("zero years", SAVINGS_PATH, savings(years=0)),
        case(
            "negative years and no scenarios",
            SAVINGS_PATH,
            savings(years=-1, scenarios=[]),
        ),
        case("missing periodicity", SAVINGS_PATH, savings(periodicity=...)),
        case("invalid periodicity", SAVINGS_PATH, savings(periodicity="WEEKLY")),
        case("missing scenarios", SAVINGS_PATH, savings(scenarios=...)),
        case("null scenarios", SAVINGS_PATH, savings(scenarios=None)),
        case("string scenarios", SAVINGS_PATH, savings(scenarios="abc")),
        case(
            "scenario without id",
            SAVINGS_PATH,
            savings(scenarios=[{"annual_market_performance": 0.05}]),
        ),
        case(
            "scenario without performance",
            SAVINGS_PATH,
            savings(scenarios=[{"id": "x"}]),
        ),
        case(
            "scenario with invalid performance",
            SAVINGS_PATH,
            savings(scenarios=[scenario("x", "fast")]),
        ),
        case(
            "scenario with numeric id",
            SAVINGS_PATH,
            savings(scenarios=[scenario(7, 0.05)]),
            compare="status_code",
        ),
        case("scenario that is a list", SAVINGS_PATH, savings(scenarios=[[1]])),
        case(
            "retirement without amount and years",
            SAVINGS_PATH,
            savings(retirement={}),
        ),
        case(
            "retirement with invalid years",
            SAVINGS_PATH,
            savings(retirement={"withdrawal_years": 2.5}),
            compare="status_code",
        ),
        case(
            "no years and no target",
            SAVINGS_PATH,
            savings(
                years=..., scenarios=[scenario("x", 0.05, periodic_contribution=100)]
            ),
        ),
        case(
            "contribution needs years",
            SAVINGS_PATH,
            savings(years=..., scenarios=[scenario("x", 0, target_amount=1000)]),
        ),
        case("invalid years", SAVINGS_PATH, savings(years="many")),
        # A period rate of 100 %: the discount factor (1 + r)^-n underflows.
        case(
            "extreme rate solves the withdrawal",
            SAVINGS_PATH,
            savings(
                base_amount=1000,
                years=1,
                scenarios=[scenario("double", 12, periodic_contribution=0)],
                retirement={"withdrawal_years": 10},
            ),
        ),
        case(
            "monthly return that is exactly half a cent",
            SAVINGS_PATH,
            savings(
                base_amount=150,
                years=1,
                scenarios=[scenario("low", 0.01, periodic_contribution=0)],
            ),
        ),
        case(
            "years beyond the supported horizon",
            SAVINGS_PATH,
            savings(years=201, periodicity="YEARLY"),
            compare="deviation",
            deviation=HORIZON_LIMIT,
        ),
        case(
            "retirement beyond the supported horizon",
            SAVINGS_PATH,
            savings(
                years=1,
                periodicity="YEARLY",
                retirement={"withdrawal_amount": 1, "withdrawal_years": 201},
            ),
            compare="deviation",
            deviation=HORIZON_LIMIT,
        ),
        case(
            "too many scenarios",
            SAVINGS_PATH,
            savings(
                years=1,
                periodicity="YEARLY",
                scenarios=[scenario(f"s{index}", 0.01) for index in range(51)],
            ),
            compare="deviation",
            deviation=HORIZON_LIMIT,
        ),
        case(
            "fixed years in other digits",
            SAVINGS_PATH,
            savings(years="３", periodicity="YEARLY"),
        ),
        case(
            "array body",
            SAVINGS_PATH,
            [1],
            compare="deviation",
            deviation={
                "reason": "Python fails with an unexpected error on a body that is not an "
                "object; Rust reports an invalid request",
                "status": 400,
                "code": "INVALID_REQUEST",
            },
        ),
        case("empty body", SAVINGS_PATH, {}),
    ]
    rng = random.Random(SEED + 1)
    for index in range(30):
        cases.append(case(f"random {index}", SAVINGS_PATH, random_savings(rng)))
    return cases


def random_savings(rng):
    periodicity = rng.choice(["MONTHLY", "QUARTERLY", "YEARLY"])
    max_years = {"MONTHLY": 6, "QUARTERLY": 15, "YEARLY": 30}[periodicity]
    body = {
        "base_amount": random_amount(rng, 0, 80000),
        "periodicity": periodicity,
        "years": rng.randint(1, max_years),
        "scenarios": [],
    }
    for number in range(rng.randint(1, 2)):
        entry = {
            "id": f"s{number}",
            "annual_market_performance": random_rate(rng, -0.03, 0.1),
        }
        if rng.random() < 0.6:
            entry["periodic_contribution"] = random_amount(rng, 0, 3000)
        elif rng.random() < 0.5:
            entry["target_amount"] = random_amount(rng, 1000, 300000)
        body["scenarios"].append(entry)
    roll = rng.random()
    if roll < 0.2:
        body["retirement"] = {
            "withdrawal_amount": random_amount(rng, 100, 3000),
            "withdrawal_years": rng.randint(1, max_years),
        }
    elif roll < 0.35:
        body["retirement"] = {"withdrawal_years": rng.randint(1, max_years)}
    return body


async def run_case(client, entry):
    # A case with a session runs against unlocked data with a valid token; one
    # without runs against locked data without a token.
    session = entry.get("session", True)
    BACKEND.unlocked = entry.get("unlocked", session)
    headers = {"Authorization": f"Bearer {SESSION_TOKEN}"} if session else {}
    if "raw_body" in entry:
        headers["Content-Type"] = entry["content_type"]
        data = entry["raw_body"] + " " * entry.get("pad_bytes", 0)
        response = await client.post(entry["path"], data=data, headers=headers)
    elif "content_type" in entry:
        headers["Content-Type"] = entry["content_type"]
        response = await client.post(
            entry["path"], data=json.dumps(entry["body"]), headers=headers
        )
    else:
        response = await client.post(entry["path"], json=entry["body"], headers=headers)
    data = await response.get_data()
    try:
        body = json.loads(data)
    except ValueError:
        body = None
    return {"status": response.status_code, "body": body, "raw": data}


async def generate():
    cases = loan_cases() + savings_cases()
    names = [entry["name"] for entry in cases]
    if len(names) != len(set(names)):
        raise SystemExit("case names must be unique")
    with tempfile.TemporaryDirectory() as static_dir:
        client = build_app(Path(static_dir)).test_client()
        for entry in cases:
            entry["response"] = await run_case(client, entry)
            raw = entry["response"].pop("raw")
            if entry["compare"] == "status_code" and "code" not in (
                entry["response"]["body"] or {}
            ):
                raise SystemExit(f"{entry['name']}: Python's answer has no error code")
            equivalent = entry.get("deviation", {}).get("equivalent_request")
            if equivalent is not None:
                # What Python answers for the request as Rust reads it.
                reference = await run_case(
                    client, {"path": entry["path"], "body": equivalent}
                )
                if reference["status"] != entry["deviation"]["status"]:
                    raise SystemExit(
                        f"{entry['name']}: Python answers the equivalent request with "
                        f"{reference['status']}, not {entry['deviation']['status']}"
                    )
                entry["deviation"]["body"] = reference["body"]
            if entry["compare"] == "deviation":
                # Rust only has to match the deviation; a digest of Python's body
                # still makes --check notice when Python's answer changes.
                body = entry["response"].pop("body")
                if body is None:
                    # Not JSON, for example an HTML error page: hash its bytes.
                    digest_input = raw
                else:
                    digest_input = json.dumps(body, sort_keys=True).encode("utf-8")
                entry["response"]["body_sha256"] = hashlib.sha256(
                    digest_input
                ).hexdigest()
    return {
        "generated_by": "tests/parity/generate_calculation_fixtures.py",
        "today": TODAY.isoformat(),
        "cases": cases,
    }


def render(fixtures):
    lines = ["{"]
    lines.append(f'  "generated_by": {json.dumps(fixtures["generated_by"])},')
    lines.append(f'  "today": {json.dumps(fixtures["today"])},')
    lines.append('  "cases": [')
    rendered = [
        "    " + json.dumps(entry, sort_keys=True, ensure_ascii=False)
        for entry in fixtures["cases"]
    ]
    lines.append(",\n".join(rendered))
    lines.append("  ]")
    lines.append("}")
    return "\n".join(lines) + "\n"


def main():
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    group = parser.add_mutually_exclusive_group(required=True)
    group.add_argument("--output", type=Path, help="write the fixtures to this file")
    group.add_argument("--check", type=Path, help="compare the fixtures in this file")
    args = parser.parse_args()

    rendered = render(asyncio.run(generate()))
    if args.output:
        args.output.write_text(rendered, encoding="utf-8")
        return 0
    if args.check.read_text(encoding="utf-8") != rendered:
        print(
            f"{args.check} is out of date; regenerate it with --output",
            file=sys.stderr,
        )
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
