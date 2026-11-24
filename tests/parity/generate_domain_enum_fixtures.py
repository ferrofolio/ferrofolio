# SPDX-License-Identifier: AGPL-3.0-only
"""Generate the fixtures for the domain modules ported to crates/core-domain.

Two files are written to the fixture directory:

- domain_enums.json lists every enum of the ported modules, keyed by module and
  class name, with the values of its members in their order. For a module that is
  ported completely every enum it defines is listed, so an enum added in Python
  makes --check fail until the fixture, and with it the Rust port, is updated.
- domain_cases.json records inputs and the outputs Python computes for every
  function and method with logic in the ported modules.

crates/core-domain/tests/domain_parity.rs requires Rust to have the same enums and
to compute the same outputs.

    python tests/parity/generate_domain_enum_fixtures.py --output DIRECTORY
    python tests/parity/generate_domain_enum_fixtures.py --check DIRECTORY
"""

import argparse
import decimal
import inspect
import itertools
import json
import random
import sys
from contextlib import contextmanager
from datetime import date, datetime
from enum import Enum
from pathlib import Path
from uuid import UUID

from ci_python import require_ci_python

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / "finanze"))

# Check the interpreter before importing the domain modules: they need third-party
# packages that a developer running the wrong Python may not have installed.
require_ci_python()

import domain.commodity as commodity  # noqa: E402
import domain.constants as constants  # noqa: E402
import domain.crypto as crypto  # noqa: E402
import domain.earnings_expenses as earnings_expenses  # noqa: E402
import domain.entity as entity  # noqa: E402
import domain.entity_account as entity_account  # noqa: E402
import domain.external_integration as external_integration  # noqa: E402
import domain.fetch_record as fetch_record  # noqa: E402
import domain.global_position as global_position  # noqa: E402
import domain.profitability as profitability  # noqa: E402
import domain.public_key as public_key  # noqa: E402
from domain.dezimal import Dezimal  # noqa: E402
from domain.exception.exceptions import MissingFieldsError  # noqa: E402

ENUMS = "domain_enums.json"
CASES = "domain_cases.json"

# Modules ported completely: every enum they define must exist in Rust.
COMPLETE_MODULES = [
    commodity,
    crypto,
    entity,
    entity_account,
    fetch_record,
    global_position,
    profitability,
    public_key,
]

# Modules ported only as far as the complete ones need them.
PARTIAL_MODULES = {
    earnings_expenses: ["FlowFrequency"],
    external_integration: ["ExternalIntegrationId"],
}

# The date Python's clock reports while cases that depend on today are recorded.
TODAY = date(2026, 11, 18)
SEED = 20261118


def module_name(module) -> str:
    return module.__name__.removeprefix("domain.")


def defined_enums(module) -> list[type[Enum]]:
    """The enums a module defines, in the order it defines them."""
    return [
        value
        for value in vars(module).values()
        if inspect.isclass(value)
        and issubclass(value, Enum)
        and value.__module__ == module.__name__
    ]


def enum_entry(module, enum: type[Enum]) -> tuple[str, list[str]]:
    if not issubclass(enum, str):
        raise SystemExit(f"{enum.__qualname__} is not a str enum; port it by hand")
    return f"{module_name(module)}.{enum.__name__}", [member.value for member in enum]


def enums() -> dict[str, list[str]]:
    entries = {}
    for module in COMPLETE_MODULES:
        for enum in defined_enums(module):
            key, values = enum_entry(module, enum)
            entries[key] = values
    for module, names in PARTIAL_MODULES.items():
        for name in names:
            key, values = enum_entry(module, getattr(module, name))
            entries[key] = values
    return entries


def decimal_text(value: Dezimal | None) -> str | None:
    """A Dezimal as Python's str() of its Decimal, which uses an exponent for very
    large or small values instead of writing out every digit."""
    return None if value is None else str(value.val)


def signal_name(error: decimal.DecimalException) -> str:
    for signal in (decimal.DivisionByZero, decimal.Overflow, decimal.InvalidOperation):
        if isinstance(error, signal):
            return signal.__name__
    raise error


def outcome(compute) -> dict:
    """The result of compute(), or the error it raised: a decimal signal, a
    MissingFieldsError with its fields, or a ValueError for unreadable input."""
    try:
        return {"result": compute()}
    except decimal.DecimalException as error:
        return {"error": signal_name(error)}
    except MissingFieldsError as error:
        return {"error": "MissingFieldsError", "missing_fields": error.missing_fields}
    except ValueError:
        return {"error": "ValueError"}


def optional_decimal(value: str | None) -> Dezimal | None:
    return None if value is None else Dezimal(value)


def optional_date(value: str | None) -> date | None:
    return None if value is None else date.fromisoformat(value)


def optional_datetime(value: str | None) -> datetime | None:
    return None if value is None else datetime.fromisoformat(value)


class FixedDateTime(datetime):
    @classmethod
    def now(cls, tz=None):
        return datetime(TODAY.year, TODAY.month, TODAY.day, 12, tzinfo=tz)


@contextmanager
def fixed_today():
    """Lets domain.profitability read TODAY from the clock."""
    original = profitability.datetime
    profitability.datetime = FixedDateTime
    try:
        yield
    finally:
        profitability.datetime = original


# Larger than half of the largest decimal, so that adding it to itself overflows.
HUGE = "9.999999999999999999999999999E+999999"


def entity_cases() -> dict:
    return {
        "is_account_scoped_entity": [
            {
                "type": entity_type.value,
                "result": entity.is_account_scoped_entity(
                    entity.Entity(
                        id=None,
                        name="Entity",
                        natural_id=None,
                        type=entity_type,
                        origin=entity.EntityOrigin.NATIVE,
                        icon_url=None,
                    )
                ),
            }
            for entity_type in entity.EntityType
        ],
    }


def constant_cases() -> dict:
    return {
        "constants": {
            "CAPITAL_GAINS_BASE_TAX": str(constants.CAPITAL_GAINS_BASE_TAX),
            "SUPPORTED_CURRENCIES": list(constants.SUPPORTED_CURRENCIES),
            "PUSD_SYMBOL": constants.PUSD_SYMBOL,
            "PUSD_CONTRACT_ADDRESS": constants.PUSD_CONTRACT_ADDRESS,
        },
    }


def commodity_cases() -> dict:
    amounts = [
        "0",
        "1",
        "2.5",
        "31.1034768",
        "-3",
        "123456789.123456789",
        "0.000001",
        "1E-999999",
        HUGE,
    ]
    return {
        "COMMODITY_HISTORIC_CUTOFF": commodity.COMMODITY_HISTORIC_CUTOFF.isoformat(),
        "commodity_symbols": {
            commodity_type.value: symbol
            for commodity_type, symbol in commodity.COMMODITY_SYMBOLS.items()
        },
        "weight_conversions": [
            {"from": source.value, "to": target.value, "factor": decimal_text(factor)}
            for source, factors in commodity.WEIGHT_CONVERSIONS.items()
            for target, factor in factors.items()
        ],
        "to_troy_ounces": [
            {
                "amount": amount,
                "unit": unit.value,
                **outcome(
                    lambda amount=amount, unit=unit: decimal_text(
                        commodity.to_troy_ounces(Dezimal(amount), unit)
                    )
                ),
            }
            for amount in amounts
            for unit in commodity.WeightUnit
        ],
    }


def crypto_rate_key_cases() -> list[dict]:
    symbols = [None, "", "eth", "Eth", "straße", "ǆ"]
    contracts = [None, "", "0xAbC", "İX", "ΟΔΟΣ"]
    return [
        {
            "type": asset_type.value,
            "symbol": symbol,
            "contract_address": contract,
            "result": crypto.crypto_rate_key(asset_type, symbol, contract),
        }
        for asset_type in crypto.CryptoCurrencyType
        for symbol in symbols
        for contract in contracts
    ]


def asset(symbol: str, balance: str, **fields) -> dict:
    """A CryptoFetchedPosition as the fixture writes it; fields override defaults."""
    spec = {
        "id": None,
        "symbol": symbol,
        "balance": balance,
        "type": "NATIVE",
        "name": None,
        "contract_address": None,
        "chain": None,
        "protocol": None,
        "position_type": "HOLDING",
        "market_value": None,
        "currency": None,
        "icon_url": None,
    }
    unknown = fields.keys() - spec.keys()
    if unknown:
        raise SystemExit(f"unknown asset fields {unknown}")
    spec.update(fields)
    return spec


def fetched(address: str, assets: list[dict], has_txs=None) -> dict:
    return {"address": address, "assets": assets, "has_txs": has_txs}


def build_asset(spec: dict) -> crypto.CryptoFetchedPosition:
    def optional_decimal(value):
        return None if value is None else Dezimal(value)

    return crypto.CryptoFetchedPosition(
        id=None if spec["id"] is None else UUID(spec["id"]),
        symbol=spec["symbol"],
        balance=Dezimal(spec["balance"]),
        type=crypto.CryptoCurrencyType(spec["type"]),
        name=spec["name"],
        contract_address=spec["contract_address"],
        chain=spec["chain"],
        protocol=spec["protocol"],
        position_type=crypto.CryptoPositionType(spec["position_type"]),
        market_value=optional_decimal(spec["market_value"]),
        currency=spec["currency"],
        icon_url=spec["icon_url"],
    )


def build_results(spec: list) -> crypto.CryptoFetchResults:
    return crypto.CryptoFetchResults(
        results={
            address: None
            if result is None
            else crypto.CryptoFetchResult(
                address=result["address"],
                assets=[build_asset(entry) for entry in result["assets"]],
                has_txs=result["has_txs"],
            )
            for address, result in spec
        }
    )


def asset_json(position: crypto.CryptoFetchedPosition) -> dict:
    return {
        "id": None if position.id is None else str(position.id),
        "symbol": position.symbol,
        "balance": decimal_text(position.balance),
        "type": position.type.value,
        "name": position.name,
        "contract_address": position.contract_address,
        "chain": position.chain,
        "protocol": position.protocol,
        "position_type": position.position_type.value,
        "market_value": decimal_text(position.market_value),
        "currency": position.currency,
        "icon_url": position.icon_url,
    }


def results_json(results: crypto.CryptoFetchResults) -> list:
    return [
        [
            address,
            None
            if result is None
            else {
                "address": result.address,
                "assets": [asset_json(entry) for entry in result.assets],
                "has_txs": result.has_txs,
            },
        ]
        for address, result in results.results.items()
    ]


def crypto_fetch_results_add_cases() -> list[dict]:
    token = {"type": "TOKEN"}
    scenarios = {
        "addresses of both, the left ones first": (
            [["a", fetched("a", [asset("ETH", "1")])]],
            [["b", fetched("b", [asset("BTC", "2")])], ["c", None]],
        ),
        "a result replaces a missing one in its place": (
            [["a", None], ["b", fetched("b", [asset("BTC", "2")])]],
            [["a", fetched("a", [asset("ETH", "1")])]],
        ),
        "a missing result keeps the present one": (
            [["a", fetched("a", [asset("ETH", "1")], True)]],
            [["a", None]],
        ),
        "two missing results": ([["a", None]], [["a", None]]),
        "balances and market values add up": (
            [
                [
                    "a",
                    fetched(
                        "a",
                        [
                            asset("ETH", "1.5"),
                            asset("BTC", "0.1"),
                            asset("SOL", "3", market_value="30"),
                            asset("ADA", "10", market_value="4.10"),
                        ],
                    ),
                ]
            ],
            [
                [
                    "a",
                    fetched(
                        "a",
                        [
                            asset("ETH", "0.25"),
                            asset("BTC", "0.2", market_value="5000"),
                            asset("SOL", "1"),
                            asset("ADA", "5", market_value="2.05"),
                            asset("DOT", "7", market_value="35"),
                        ],
                    ),
                ]
            ],
        ),
        "tokens are keyed by their contract": (
            [
                [
                    "a",
                    fetched(
                        "a", [asset("USDC", "10", contract_address="0xA", **token)]
                    ),
                ]
            ],
            [
                [
                    "a",
                    fetched(
                        "a", [asset("USDC.e", "5", contract_address="0xA", **token)]
                    ),
                ]
            ],
        ),
        "a contract and a symbol with the same text share a key": (
            [["a", fetched("a", [asset("0xA", "1", **token)])]],
            [["a", fetched("a", [asset("X", "2", contract_address="0xA", **token)])]],
        ),
        "an empty contract and a native coin's contract do not count": (
            [
                [
                    "a",
                    fetched(
                        "a",
                        [
                            asset("USDT", "1", contract_address="", **token),
                            asset("ETH", "1", contract_address="0xA"),
                        ],
                    ),
                ]
            ],
            [
                [
                    "a",
                    fetched(
                        "a",
                        [
                            asset("USDT", "2", **token),
                            asset("ETH", "2", contract_address="0xB"),
                        ],
                    ),
                ]
            ],
        ),
        "type, position type, chain and protocol separate assets": (
            [["a", fetched("a", [asset("ETH", "1")])]],
            [
                [
                    "a",
                    fetched(
                        "a",
                        [
                            asset("ETH", "2", **token),
                            asset("ETH", "3", position_type="STAKED"),
                            asset("ETH", "4", chain="base"),
                            asset("ETH", "5", protocol="aave"),
                        ],
                    ),
                ]
            ],
        ),
        "assets of the left result with one key collapse into the last": (
            [
                [
                    "a",
                    fetched(
                        "a",
                        [
                            asset("ETH", "1", name="first"),
                            asset("BTC", "2"),
                            asset("ETH", "3", name="second"),
                        ],
                    ),
                ]
            ],
            [["a", fetched("a", [asset("ETH", "4")])]],
        ),
        "assets of the right result with one key add up": (
            [["a", fetched("a", [asset("BTC", "1")])]],
            [["a", fetched("a", [asset("ETH", "1"), asset("ETH", "2")])]],
        ),
        "transactions are known when either result knows them": (
            [
                [address, fetched(address, [], left)]
                for address, left in zip(
                    "abcdefghi", [None, None, None] + [False] * 3 + [True] * 3
                )
            ],
            [
                [address, fetched(address, [], right)]
                for address, right in zip("abcdefghi", [None, False, True] * 3)
            ],
        ),
        "other fields of the left asset are kept": (
            [
                [
                    "a",
                    fetched(
                        "a",
                        [
                            asset(
                                "ETH",
                                "1",
                                id="0b9c2f8e-5a39-4c43-9a5e-1f2d3c4b5a69",
                                name="Ether",
                                currency="EUR",
                                icon_url="https://example.com/eth.png",
                            )
                        ],
                    ),
                ]
            ],
            [["a", fetched("a", [asset("ETH", "1", name="Other", currency="USD")])]],
        ),
        "an overflowing balance": (
            [["a", fetched("a", [asset("ETH", HUGE)])]],
            [["a", fetched("a", [asset("ETH", HUGE)])]],
        ),
        "an overflowing market value": (
            [["a", fetched("a", [asset("ETH", "1", market_value=HUGE)])]],
            [["a", fetched("a", [asset("ETH", "1", market_value=HUGE)])]],
        ),
    }
    return [
        {
            "name": name,
            "left": left,
            "right": right,
            **outcome(
                lambda left=left, right=right: results_json(
                    build_results(left) + build_results(right)
                )
            ),
        }
        for name, (left, right) in scenarios.items()
    ]


def crypto_cases() -> dict:
    return {
        "crypto_rate_key": crypto_rate_key_cases(),
        "crypto_fetch_results_add": crypto_fetch_results_add_cases(),
    }


def dezimal_cases() -> dict:
    values = [
        "0",
        "-0",
        "0.125",
        "0.135",
        "-0.125",
        "2.5",
        "3.5",
        "1.5",
        "-0.001",
        "-0.00005",
        "123.456789",
        "0.00049999",
        "99.995",
        "1E+23",
        "1E+24",
        "1E+30",
        "12345678901234567890.123456789",
        "1E-30",
    ]
    places = [0, 2, 4]
    return {
        "dezimal_round": [
            {
                "value": value,
                "places": count,
                **outcome(
                    lambda value=value, count=count: format(
                        round(Dezimal(value), count).val, "f"
                    )
                ),
            }
            for value in values
            for count in places
        ],
        "dezimal_to_fixed": [
            {
                "value": value,
                "places": count,
                "result": format(Dezimal(value).val, f".{count}f"),
            }
            for value in values
            for count in places
        ],
    }


def profitability_case(terms: dict) -> dict:
    with fixed_today():
        return {
            **terms,
            **outcome(
                lambda: decimal_text(
                    profitability.annualized_profitability(
                        interest_rate=optional_decimal(terms["interest_rate"]),
                        start_dt=optional_datetime(terms["start_dt"]),
                        maturity=optional_date(terms["maturity"]),
                        extended_maturity=optional_date(terms["extended_maturity"]),
                        extended_interest_rate=optional_decimal(
                            terms["extended_interest_rate"]
                        ),
                        late_interest_rate=optional_decimal(
                            terms["late_interest_rate"]
                        ),
                    )
                )
            ),
        }


def profitability_cases() -> dict:
    options = {
        "interest_rate": [None, "0", "0.05", "0.1234", "-0.02"],
        # Naive and aware; the aware ones fall on another day in UTC.
        "start_dt": [
            None,
            "2024-01-15T10:00:00",
            "2026-11-30T23:30:00-05:00",
            "2027-01-01T00:30:00+14:00",
        ],
        "maturity": [None, "2025-01-15", TODAY.isoformat(), "2026-12-31", "2027-06-30"],
        "extended_maturity": [None, "2026-11-01", TODAY.isoformat(), "2027-01-31"],
        "extended_interest_rate": [None, "0.08"],
        "late_interest_rate": [None, "0", "0.2"],
    }
    grid = [
        dict(zip(options, values)) for values in itertools.product(*options.values())
    ]
    rng = random.Random(SEED)
    chosen = rng.sample(grid, 300)
    blank = dict.fromkeys(options)
    edge = [
        # Every part at once, past every maturity.
        {
            "interest_rate": "0.05",
            "start_dt": "2024-01-15T10:00:00",
            "maturity": "2025-01-15",
            "extended_maturity": "2026-01-15",
            "extended_interest_rate": "0.08",
            "late_interest_rate": "0.2",
        },
        # A maturity before the start earns nothing for the term.
        {
            **blank,
            "interest_rate": "0.05",
            "start_dt": "2025-01-15T00:00:00",
            "maturity": "2024-01-15",
        },
        # A result beyond 28 digits cannot be rounded to four places.
        {
            **blank,
            "interest_rate": "1E+30",
            "start_dt": "2024-01-15T00:00:00",
            "maturity": "2025-01-15",
        },
        # A rate that overflows when multiplied by the days.
        {
            **blank,
            "interest_rate": HUGE,
            "start_dt": "2024-01-15T00:00:00",
            "maturity": "2025-01-15",
        },
    ]
    return {
        "today": TODAY.isoformat(),
        "annualized_profitability": [
            profitability_case(terms) for terms in edge + chosen
        ],
    }


def loan_hash_cases() -> list[dict]:
    amounts = [
        "100",
        "100.005",
        "100.015",
        "0.125",
        "-0.001",
        "-0",
        "0E-10",
        "1E+30",
        "1234567.891",
        " 12_000.5 ",
        "١٢٣.٥",
        "abc",
    ]
    inputs = [("e1", amount, "2024-01-01") for amount in amounts] + [
        ("", "100", "2024-01-01"),
        ("ünï|code", "100", "2024-01-01"),
        ("e1", "100", "0005-03-04"),
        ("e1", "100", "2024-01-01T00:00:00"),
    ]
    return [
        {
            "entity_id": entity_id,
            "amount": amount,
            "date": creation,
            **outcome(
                lambda entity_id=entity_id, amount=amount, creation=creation: (
                    global_position.compute_loan_hash(entity_id, amount, creation)
                )
            ),
        }
        for entity_id, amount, creation in inputs
    ]


def investment_hash_cases() -> list[dict]:
    inputs = [
        ("e1", "1000", "2024-01-01", "Fund A"),
        ("e1", "999.995", "2024-01-01", ""),
        ("e1", "-12.345", "2023-12-31", "Café ☕"),
        ("e1", "1000", "2024-01-01", "a|b"),
        ("", "0", "2024-01-01", "x"),
        ("e1", "1e3", "2024-01-01", "Fund A"),
        ("e1", "NaNa", "2024-01-01", "Fund A"),
    ]
    return [
        {
            "entity_id": entity_id,
            "amount": amount,
            "date": start,
            "name": name,
            **outcome(
                lambda entity_id=entity_id, amount=amount, start=start, name=name: (
                    global_position.compute_investment_hash(
                        entity_id, amount, start, name
                    )
                )
            ),
        }
        for entity_id, amount, start, name in inputs
    ]


def loan_cases() -> list[dict]:
    inputs = [
        ("100.50", "50", "2020-01-02", "x", ""),
        ("250000", "180000.37", "2019-07-31", "e2", ""),
        ("1000", "1000.01", "0005-03-04", "e3", ""),
        ("100", "40", "2020-01-02", "x", "preset"),
        (HUGE, "-" + HUGE, "2020-01-02", "x", ""),
    ]

    def run(amount, outstanding, creation, entity_id, preset):
        loan = global_position.Loan(
            id=None,
            type=global_position.LoanType.MORTGAGE,
            currency="EUR",
            current_installment=Dezimal("500"),
            interest_rate=Dezimal("0.03"),
            loan_amount=Dezimal(amount),
            creation=date.fromisoformat(creation),
            maturity=date(2040, 1, 2),
            principal_outstanding=Dezimal(outstanding),
            principal_paid=Dezimal("1"),
            hash=preset,
        )
        return {
            "principal_paid": decimal_text(loan.principal_paid),
            "hash": loan.compute_hash(entity_id),
        }

    return [
        {
            "loan_amount": amount,
            "principal_outstanding": outstanding,
            "creation": creation,
            "entity_id": entity_id,
            "hash": preset,
            **outcome(lambda args=args: run(*args)),
        }
        for args in inputs
        for amount, outstanding, creation, entity_id, preset in [args]
    ]


def security_cases(build) -> list[dict]:
    investments = [None, "100", "0"]
    prices = [None, "12.5", "0"]
    shares = ["0", "0.000", "8", "3", "-2"]
    inputs = list(itertools.product(investments, prices, shares)) + [
        (None, HUGE, "10"),
        ("1", None, "1E-999999"),
    ]

    def run(investment, price, count):
        detail = build(
            shares=Dezimal(count),
            initial_investment=optional_decimal(investment),
            average_buy_price=optional_decimal(price),
        )
        return {
            "initial_investment": decimal_text(detail.initial_investment),
            "average_buy_price": decimal_text(detail.average_buy_price),
        }

    return [
        {
            "initial_investment": investment,
            "average_buy_price": price,
            "shares": count,
            **outcome(lambda args=args: run(*args)),
        }
        for args in inputs
        for investment, price, count in [args]
    ]


def stock(**fields):
    return global_position.StockDetail(
        id=None,
        name="Stock",
        ticker="STK",
        isin="XX0000000000",
        market_value=Dezimal("100"),
        currency="EUR",
        type=global_position.EquityType.STOCK,
        **fields,
    )


def fund(**fields):
    return global_position.FundDetail(
        id=None,
        name="Fund",
        isin="XX0000000000",
        market=None,
        market_value=Dezimal("100"),
        currency="EUR",
        type=global_position.FundType.MUTUAL_FUND,
        **fields,
    )


def global_position_cases() -> dict:
    return {
        "installment_frequency": [
            {
                "value": frequency.value,
                "payments_per_year": frequency.payments_per_year,
                "flow_frequency": global_position.INSTALLMENT_TO_FLOW_FREQ[
                    frequency
                ].value,
            }
            for frequency in global_position.InstallmentFrequency
        ],
        "compute_loan_hash": loan_hash_cases(),
        "compute_investment_hash": investment_hash_cases(),
        "loan": loan_cases(),
        "stock_detail": security_cases(stock),
        "fund_detail": security_cases(fund),
        "factoring_detail": factoring_cases(),
        "real_estate_cf_detail": real_estate_cf_cases(),
        "deposit": deposit_cases(),
        "crypto_currency_position": crypto_position_cases(),
        "credit_available_amount": credit_cases(),
    }


def sampled(options: dict, count: int, edge: list[dict]) -> list[dict]:
    """The edge cases and a seeded sample of every combination of the options."""
    grid = [
        dict(zip(options, values)) for values in itertools.product(*options.values())
    ]
    return edge + random.Random(SEED).sample(grid, min(count, len(grid)))


def iso(value) -> str | None:
    return None if value is None else value.isoformat()


def factoring_cases() -> list[dict]:
    options = {
        "interest_rate": ["0.1", "0.0725"],
        "start": ["2025-01-10T09:00:00", "2026-11-30T23:00:00-05:00"],
        "maturity": ["2026-01-10", "2026-12-15", "2025-01-01"],
        "late_interest_rate": [None, "0.15"],
        "gross_interest_rate": [None, "0.12"],
        "gross_late_interest_rate": [None, "0.2"],
        "last_invest_date": [None, "2025-02-01T00:00:00"],
        "profitability": [None, "0.5"],
    }
    blank = {name: values[0] for name, values in options.items()}
    edge = [{**blank, "interest_rate": HUGE, "profitability": None}]

    def run(terms):
        detail = global_position.FactoringDetail(
            id=None,
            name="Invoice",
            amount=Dezimal("1000"),
            currency="EUR",
            interest_rate=Dezimal(terms["interest_rate"]),
            start=datetime.fromisoformat(terms["start"]),
            maturity=date.fromisoformat(terms["maturity"]),
            type="FACTORING",
            state="ACTIVE",
            last_invest_date=optional_datetime(terms["last_invest_date"]),
            profitability=optional_decimal(terms["profitability"]),
            late_interest_rate=optional_decimal(terms["late_interest_rate"]),
            gross_interest_rate=optional_decimal(terms["gross_interest_rate"]),
            gross_late_interest_rate=optional_decimal(
                terms["gross_late_interest_rate"]
            ),
        )
        return {
            "gross_interest_rate": decimal_text(detail.gross_interest_rate),
            "gross_late_interest_rate": decimal_text(detail.gross_late_interest_rate),
            "last_invest_date": iso(detail.last_invest_date),
            "profitability": decimal_text(detail.profitability),
        }

    with fixed_today():
        return [
            {**terms, **outcome(lambda terms=terms: run(terms))}
            for terms in sampled(options, 60, edge)
        ]


def real_estate_cf_cases() -> list[dict]:
    options = {
        "interest_rate": ["0.1", "0.0725"],
        "start": ["2025-01-10T09:00:00", "2026-11-30T23:00:00-05:00"],
        "maturity": ["2026-01-10", "2026-12-15", "2025-01-01"],
        "extended_maturity": [None, "2026-12-01", "2027-03-01"],
        "extended_interest_rate": [None, "0.09"],
        "last_invest_date": [None, "2025-02-01T00:00:00"],
        "profitability": [None, "0.5"],
    }
    blank = {name: values[0] for name, values in options.items()}
    edge = [{**blank, "interest_rate": HUGE, "profitability": None}]

    def run(terms):
        detail = global_position.RealEstateCFDetail(
            id=None,
            name="Project",
            amount=Dezimal("1000"),
            pending_amount=Dezimal("0"),
            currency="EUR",
            interest_rate=Dezimal(terms["interest_rate"]),
            start=datetime.fromisoformat(terms["start"]),
            maturity=date.fromisoformat(terms["maturity"]),
            type="LOAN",
            state="ACTIVE",
            last_invest_date=optional_datetime(terms["last_invest_date"]),
            profitability=optional_decimal(terms["profitability"]),
            extended_maturity=optional_date(terms["extended_maturity"]),
            extended_interest_rate=optional_decimal(terms["extended_interest_rate"]),
        )
        return {
            "last_invest_date": iso(detail.last_invest_date),
            "profitability": decimal_text(detail.profitability),
        }

    with fixed_today():
        return [
            {**terms, **outcome(lambda terms=terms: run(terms))}
            for terms in sampled(options, 60, edge)
        ]


def deposit_cases() -> list[dict]:
    options = {
        "amount": ["1000", "2500.55", "0"],
        "interest_rate": ["0.0275", "0.031"],
        "creation": ["2025-03-01T10:00:00", "2026-11-30T23:00:00-05:00"],
        "maturity": ["2026-03-01", "2027-02-28", "2025-01-01"],
        "expected_interests": [None, "12.34"],
    }
    blank = {name: values[0] for name, values in options.items()}
    # Interests beyond 28 digits cannot be rounded to cents.
    edge = [{**blank, "amount": "1E+30", "maturity": "2027-02-28"}]

    def run(terms):
        deposit = global_position.Deposit(
            id=None,
            name="Deposit",
            amount=Dezimal(terms["amount"]),
            currency="EUR",
            interest_rate=Dezimal(terms["interest_rate"]),
            creation=datetime.fromisoformat(terms["creation"]),
            maturity=date.fromisoformat(terms["maturity"]),
            expected_interests=optional_decimal(terms["expected_interests"]),
        )
        return decimal_text(deposit.expected_interests)

    with fixed_today():
        return [
            {**terms, **outcome(lambda terms=terms: run(terms))}
            for terms in sampled(options, 100, edge)
        ]


def crypto_position_cases() -> list[dict]:
    inputs = list(
        itertools.product(
            [None, "100", "0"], [None, "2", "0"], ["0", "5", "-1"], [None, "", "EUR"]
        )
    ) + [(None, HUGE, "10", "EUR")]

    def run(investment, price, amount, currency):
        position = global_position.CryptoCurrencyPosition(
            id=None,
            symbol="BTC",
            amount=Dezimal(amount),
            type=crypto.CryptoCurrencyType.NATIVE,
            initial_investment=optional_decimal(investment),
            average_buy_price=optional_decimal(price),
            investment_currency=currency,
        )
        return {
            "initial_investment": decimal_text(position.initial_investment),
            "average_buy_price": decimal_text(position.average_buy_price),
        }

    return [
        {
            "initial_investment": investment,
            "average_buy_price": price,
            "amount": amount,
            "investment_currency": currency,
            **outcome(lambda args=args: run(*args)),
        }
        for args in inputs
        for investment, price, amount, currency in [args]
    ]


def credit_cases() -> list[dict]:
    inputs = [("1000", "250.5"), ("0", "10"), ("1E+30", "1E-30"), (HUGE, "-" + HUGE)]

    def run(limit, drawn):
        credit = global_position.CreditDetail(
            id=None,
            currency="EUR",
            credit_limit=Dezimal(limit),
            drawn_amount=Dezimal(drawn),
            interest_rate=Dezimal("0.05"),
        )
        return decimal_text(credit.available_amount)

    return [
        {
            "credit_limit": limit,
            "drawn_amount": drawn,
            **outcome(lambda limit=limit, drawn=drawn: run(limit, drawn)),
        }
        for limit, drawn in inputs
    ]


def cases() -> dict:
    groups = {}
    for build in (
        entity_cases,
        constant_cases,
        commodity_cases,
        crypto_cases,
        dezimal_cases,
        profitability_cases,
        global_position_cases,
    ):
        for name, group in build().items():
            if name in groups:
                raise SystemExit(f"case group {name} is defined twice")
            groups[name] = group
    return groups


def render(value) -> str:
    return json.dumps(value, indent=2, ensure_ascii=False) + "\n"


def render_cases(groups: dict) -> str:
    """The case groups with one case per line, which keeps the file short."""

    def line(value) -> str:
        return json.dumps(value, ensure_ascii=False, sort_keys=True)

    entries = []
    for name, group in groups.items():
        if isinstance(group, list) and group:
            body = ",\n".join(f"    {line(case)}" for case in group)
            entries.append(f"  {line(name)}: [\n{body}\n  ]")
        else:
            entries.append(f"  {line(name)}: {line(group)}")
    return "{\n" + ",\n".join(entries) + "\n}\n"


def main():
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    group = parser.add_mutually_exclusive_group(required=True)
    group.add_argument("--output", type=Path, help="write the fixtures to DIRECTORY")
    group.add_argument("--check", type=Path, help="compare the fixtures in DIRECTORY")
    args = parser.parse_args()

    rendered = {ENUMS: render(enums()), CASES: render_cases(cases())}
    if args.output:
        for name, text in rendered.items():
            (args.output / name).write_text(text, encoding="utf-8")
        return 0
    stale = [
        name
        for name, text in rendered.items()
        if not (args.check / name).is_file()
        or (args.check / name).read_text(encoding="utf-8") != text
    ]
    for name in stale:
        print(
            f"{args.check / name} is out of date; regenerate it with --output",
            file=sys.stderr,
        )
    return 1 if stale else 0


if __name__ == "__main__":
    sys.exit(main())
