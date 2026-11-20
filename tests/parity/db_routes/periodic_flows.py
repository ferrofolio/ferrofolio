# SPDX-License-Identifier: AGPL-3.0-only
"""The periodic flows: GET /api/v1/flows/periodic.

The empty database has no flows. The flows database holds flows of every
frequency, enabled and disabled, starting before and after today, ending before
and after their next date, with and without a label, a maximum amount and a link
to a real estate, written by the repository, and a few in spellings Python reads
but does not write. The invalid database holds a flow whose flag only pydantic's
coercion reads: Rust passes the request on.

The route takes today's date from the clock; the recording fixes it at TODAY,
which the Rust test sets as its clock. `functions` records `get_next_date` for
many flows and days, and `_map_row_to_periodic_flow` for rows in the forms the
writer stores and in many it does not.
"""

import json
from datetime import date, timedelta
from uuid import UUID

import application.use_cases.get_periodic_flows as get_periodic_flows_module
from application.use_cases.get_periodic_flows import (
    GetPeriodicFlowsImpl,
    get_next_date,
)
from domain.dezimal import Dezimal
from domain.earnings_expenses import FlowFrequency, FlowType, PeriodicFlow
from infrastructure.controller.config import FJSONProvider
from infrastructure.controller.routes.get_periodic_flows import get_periodic_flows
from infrastructure.repository.earnings_expenses.periodic_flow_repository import (
    PeriodicFlowRepository,
    _map_row_to_periodic_flow,
)

from db_routes.harness import EMPTY, Scenario, Slice, case

PATH = "/api/v1/flows/periodic"
TODAY = date(2026, 11, 18)


class _Today:
    """The day `date.today()` answers in the use case."""

    value = TODAY


class FixedDate(date):
    @classmethod
    def today(cls):
        return _Today.value


# The use case reads the day through its module's `date`.
get_periodic_flows_module.date = FixedDate

LABEL = "5b1d1a52-6e0b-4a54-8f7c-0a6c2e9d7b10"
# A label stored in a spelling Python reads but does not write, which a flow
# links to in the same spelling, as the foreign key requires.
BRACED_LABEL = "{1F0A9C3E-2B4D-4E6F-8A9B-0C1D2E3F4A5B}"
REAL_ESTATE = "c0000000-0000-4000-8000-000000000001"
CREATED = "2026-02-01T10:00:00+01:00"


def _flow(
    index,
    frequency,
    since,
    *,
    until=None,
    enabled=True,
    flow_type=FlowType.EXPENSE,
    amount="12.50",
    max_amount=None,
    label_id=None,
    category=None,
    icon=None,
    name=None,
):
    return PeriodicFlow(
        id=UUID(f"f1000000-0000-4000-8000-{index:012d}"),
        name=name or f"{frequency.value.title()} flow",
        amount=Dezimal(amount),
        currency="EUR",
        flow_type=flow_type,
        frequency=frequency,
        category=category,
        enabled=enabled,
        since=since,
        until=until,
        icon=icon,
        max_amount=Dezimal(max_amount) if max_amount else None,
        label_id=UUID(label_id) if label_id else None,
    )


# Written by PeriodicFlowRepository.save; TODAY is 2026-11-18.
WRITTEN = [
    # 31 January plus one month is 28 February, and the day stays the 28th.
    _flow(
        1,
        FlowFrequency.MONTHLY,
        date(2025, 1, 31),
        category="Housing",
        icon="house",
        label_id=LABEL,
        max_amount="900",
        amount="850.5",
    ),
    _flow(2, FlowFrequency.WEEKLY, date(2026, 1, 5), flow_type=FlowType.EARNING),
    # The next date, 2026-11-23, lies after the end.
    _flow(3, FlowFrequency.BIWEEKLY, date(2025, 12, 1), until=date(2026, 11, 20)),
    _flow(4, FlowFrequency.SEMIMONTHLY, date(2026, 11, 3), until=date(2026, 12, 3)),
    _flow(5, FlowFrequency.DAILY, date(2020, 2, 29), amount="0.01"),
    _flow(6, FlowFrequency.QUARTERLY, date(2024, 8, 31), max_amount="1E+3"),
    # Starts after today: next due on its start, although it ended before.
    _flow(7, FlowFrequency.YEARLY, date(2027, 3, 1), until=date(2026, 1, 1)),
    _flow(8, FlowFrequency.EVERY_TWO_MONTHS, date(2026, 1, 18), enabled=False),
    _flow(9, FlowFrequency.EVERY_FOUR_MONTHS, date(2026, 7, 18)),
    _flow(10, FlowFrequency.SEMIANNUALLY, date(2024, 5, 31), name="Ü ñ €"),
    # Starts today; the yearly step from 29 February.
    _flow(11, FlowFrequency.MONTHLY, TODAY),
    _flow(12, FlowFrequency.YEARLY, date(2024, 2, 29), amount="12345678901234.56789"),
    _flow(13, FlowFrequency.WEEKLY, date(1, 1, 1), flow_type=FlowType.EARNING),
]

# Flows linked to the real estate: flow index, subtype, loan hash.
LINKS = [(6, "LOAN", "loan-hash-1"), (9, "RENT", None)]

# Rows in spellings Python reads but its writer does not produce: the id, the
# amounts and the label id. Columns: id, name, amount, currency, flow_type,
# frequency, category, enabled, since, until, icon, max_amount, label_id.
OTHER_ROWS = [
    (
        "F1000000-0000-4000-8000-000000000020",
        "Upper-case id",
        " 1_000.5 ",
        "USD",
        "EARNING",
        "MONTHLY",
        None,
        True,
        "2026-10-31",
        None,
        None,
        "",
        BRACED_LABEL,
    ),
    (
        "{f1000000-0000-4000-8000-000000000021}",
        "Braced id",
        "-0",
        "EUR",
        "EXPENSE",
        "QUARTERLY",
        "",
        False,
        "2026-11-18",
        "2026-11-18",
        "",
        "0E-8",
        None,
    ),
]

# pydantic reads the text "true" as True; Rust leaves such a flag to Python.
UNREADABLE_ROWS = [
    (
        "f2000000-0000-4000-8000-000000000001",
        "Coerced",
        "10",
        "EUR",
        "EXPENSE",
        "MONTHLY",
        None,
        "true",
        "2026-01-01",
        None,
        None,
        None,
        None,
    ),
]

INSERT = (
    "INSERT INTO periodic_flows (id, name, amount, currency, flow_type, frequency, "
    "category, enabled, since, until, icon, max_amount, label_id) "
    "VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)"
)


async def fill(client):
    async with client.tx() as cursor:
        for label_id, name in ((LABEL, "Home"), (BRACED_LABEL, "Braced")):
            await cursor.execute(
                "INSERT INTO labels (id, key, name, description, color, icon, "
                "category, subcategory, created_at) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
                (
                    label_id,
                    None,
                    name,
                    None,
                    "#123456",
                    "tag",
                    "EXPENSE",
                    "NEED",
                    CREATED,
                ),
            )
        await cursor.execute(
            "INSERT INTO real_estate (id, name, currency, is_residence, is_rented, "
            "purchase_date, purchase_price, purchase_expenses, estimated_market_value, "
            "valuations, created_at) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
            (
                REAL_ESTATE,
                "Flat",
                "EUR",
                False,
                True,
                "2020-01-01",
                "200000",
                "[]",
                "250000",
                "[]",
                CREATED,
            ),
        )
    repository = PeriodicFlowRepository(client)
    for flow in WRITTEN:
        await repository.save(flow)
    async with client.tx() as cursor:
        for row in OTHER_ROWS:
            await cursor.execute(INSERT, row)
        for index, subtype, loan_hash in LINKS:
            await cursor.execute(
                "INSERT INTO real_estate_flows (real_estate_id, periodic_flow_id, "
                "flow_subtype, description, payload, extra_reference) "
                "VALUES (?, ?, ?, ?, ?, ?)",
                (
                    REAL_ESTATE,
                    str(WRITTEN[index - 1].id),
                    subtype,
                    f"{subtype} description",
                    "{}",
                    loan_hash,
                ),
            )


async def fill_invalid(client):
    async with client.tx() as cursor:
        for row in UNREADABLE_ROWS:
            await cursor.execute(INSERT, row)


TABLES = {"periodic_flows": (), "real_estate_flows": (), "real_estate": ()}

INVALID = Scenario("flows_invalid", fill_invalid, {"periodic_flows": ()})


def _seeded_labels(connection, table, columns, rows):
    """The migrations seed labels with new ids and creation times in every
    database; only the scenario's own label is compared row by row."""
    if table != "labels":
        return rows
    index = columns.index("id")
    return [row for row in rows if row[index] in (LABEL, BRACED_LABEL)]


FLOWS = Scenario("flows", fill, {**TABLES, "labels": ()}, _seeded_labels)


def register(app, client):
    get_periodic_flows_uc = GetPeriodicFlowsImpl(PeriodicFlowRepository(client))

    @app.route(PATH, methods=["GET"])
    async def periodic_flows_route():
        _Today.value = TODAY
        return await get_periodic_flows(get_periodic_flows_uc)


def cases():
    def get(name, database, **options):
        return case(name, database, PATH, **options)

    return [
        get("no flows", "empty"),
        get("flows", "flows"),
        get("a query is ignored", "flows", query="enabled=false"),
        get(
            "a flag only Python reads",
            "flows_invalid",
            compare="forwarded",
            passed_on=True,
        ),
        # Rust passes these on to Python, which answers as recorded.
        get("no session while unlocked", "flows", session="none", compare="forwarded"),
        get(
            "unknown session while unlocked",
            "flows",
            session="unknown",
            compare="forwarded",
        ),
        get("locked", "flows", session="none", unlocked=False, compare="forwarded"),
        get(
            "locked with an ended session",
            "flows",
            session="unknown",
            unlocked=False,
            compare="forwarded",
        ),
    ]


class _App:
    """What FJSONProvider needs of an app: nothing but a weak reference."""


_APP = _App()
_PROVIDER = FJSONProvider(_APP)


def _json(value):
    """value as jsonify answers it, or None when that is not JSON."""
    answer = json.loads(_PROVIDER.dumps(value))
    try:
        json.dumps(answer, allow_nan=False)
    except ValueError:
        return None
    return answer


# Days and starts around month ends, leap days and the ends of the range.
DAYS = [
    date(2026, 11, 18),
    date(2026, 12, 31),
    date(2028, 2, 29),
    date(2027, 1, 30),
    date(9999, 12, 31),
]
STARTS = [
    date(1, 1, 1),
    date(2024, 1, 31),
    date(2024, 2, 29),
    date(2025, 8, 30),
    date(2026, 11, 17),
    date(2026, 11, 18),
    date(2026, 11, 19),
    date(2026, 12, 31),
    date(9999, 12, 31),
]


def _next_dates():
    entries = []
    for today in DAYS:
        _Today.value = today
        for frequency in FlowFrequency:
            for since in STARTS:
                flow = _flow(0, frequency, since)
                try:
                    plain = get_next_date(flow)
                except Exception as error:  # noqa: BLE001 - every failure is recorded
                    entries.append(
                        [
                            frequency.value,
                            since.isoformat(),
                            None,
                            True,
                            today.isoformat(),
                            {"error": type(error).__name__},
                        ]
                    )
                    continue
                untils = [None]
                if plain is not None:
                    untils += [plain, plain - timedelta(days=1)]
                for until in untils:
                    flow.until = until
                    for enabled in (True, False):
                        flow.enabled = enabled
                        result = get_next_date(flow)
                        entries.append(
                            [
                                frequency.value,
                                since.isoformat(),
                                until.isoformat() if until else None,
                                enabled,
                                today.isoformat(),
                                {"ok": result.isoformat() if result else None},
                            ]
                        )
    _Today.value = TODAY
    return entries


def _row(**changes):
    row = {
        "id": "f3000000-0000-4000-8000-000000000001",
        "name": "Row",
        "amount": "12.50",
        "currency": "EUR",
        "flow_type": "EXPENSE",
        "frequency": "MONTHLY",
        "category": None,
        "enabled": 1,
        "since": "2026-01-31",
        "until": None,
        "icon": None,
        "max_amount": None,
        "label_id": None,
        "linked": 0,
        "linked_loan_hash": None,
        "re_flow_subtype": None,
    }
    row.update(changes)
    return row


# Rows as the writer and the query store them.
WRITTEN_ROWS = [
    _row(),
    _row(
        name="Ü ñ €",
        amount="850.5",
        category="Housing",
        enabled=0,
        until="2027-12-31",
        icon="house",
        max_amount="1E+3",
        label_id=LABEL,
        linked=1,
        linked_loan_hash="hash",
        re_flow_subtype="LOAN",
    ),
    _row(linked=1, re_flow_subtype="RENT", amount="12345678901234567890.123456789"),
    _row(
        flow_type="EARNING", frequency="DAILY", since="0001-01-01", until="9999-12-31"
    ),
]

# Forms the writer does not produce that Rust reads exactly as Python does.
OTHER_FORM_ROWS = [
    _row(id="f3000000-0000-4000-8000-000000000001".replace("-", "").upper()),
    _row(id="{f3000000-0000-4000-8000-000000000001}"),
    _row(id="urn:uuid:f3000000-0000-4000-8000-000000000001"),
    _row(label_id=""),
    _row(label_id=LABEL.upper()),
    _row(max_amount=""),
    _row(amount=" 1_000.5 "),
    _row(amount="-0"),
    _row(amount="0e-500"),
    _row(category="", icon=""),
]

# Forms Python reads, coerces or rejects that Rust leaves to Python.
LEFT_TO_PYTHON_ROWS = [
    _row(enabled="true"),
    _row(enabled="1"),
    _row(enabled=2),
    _row(enabled=1.0),
    _row(enabled=None),
    _row(since="20260101"),
    _row(since=20260101),
    _row(since="2026-02-30"),
    _row(since="2026-01-01T00:00:00"),
    _row(since="0000-01-01"),
    _row(since=None),
    _row(until=""),
    _row(until="2026-1-1"),
    _row(amount="NaN"),
    _row(amount="Infinity"),
    _row(amount="1e400"),
    _row(amount=""),
    _row(amount=5),
    _row(amount=1.5),
    _row(max_amount="x"),
    _row(max_amount=5),
    _row(flow_type="expense"),
    _row(frequency="monthly"),
    _row(frequency="BIMONTHLY"),
    _row(id="x"),
    _row(id=None),
    _row(label_id="x"),
    _row(name=None),
    _row(name=5),
    _row(currency=None),
    _row(category=5),
    _row(icon=5),
    _row(re_flow_subtype=5),
    _row(re_flow_subtype="LOAN", linked_loan_hash=5),
    _row(linked=None),
    _row(linked="1"),
]


def _mapped(row):
    """_map_row_to_periodic_flow(row) as the route answers the flow, without the
    next date, which the use case adds; or the error."""
    try:
        flow = _map_row_to_periodic_flow(row)
    except Exception as error:  # noqa: BLE001 - every failure is recorded
        return {"error": type(error).__name__}
    answer = _json(
        {
            "id": str(flow.id),
            "name": flow.name,
            "amount": flow.amount,
            "currency": flow.currency,
            "flow_type": flow.flow_type.value,
            "frequency": flow.frequency.value,
            "category": flow.category,
            "label_id": str(flow.label_id) if flow.label_id else None,
            "enabled": flow.enabled,
            "since": flow.since.isoformat() if flow.since else None,
            "until": flow.until.isoformat() if flow.until else None,
            "linked": flow.linked,
            "real_estate_flow": {
                "flow_subtype": flow.real_estate_flow.flow_subtype,
                "linked_loan_hash": flow.real_estate_flow.linked_loan_hash,
            }
            if flow.real_estate_flow
            else None,
            "next_date": None,
            "max_amount": flow.max_amount,
            "icon": flow.icon,
        }
    )
    if answer is None:
        return {"error": "the answer is not JSON"}
    return {"ok": answer}


def functions():
    return {
        "next_date": _next_dates(),
        "rows": [
            [row, _mapped(row), kind]
            for kind, rows in (
                ("written", WRITTEN_ROWS),
                ("other", OTHER_FORM_ROWS),
                ("python", LEFT_TO_PYTHON_ROWS),
            )
            for row in rows
        ],
    }


SLICE = Slice(
    "periodic_flows",
    (EMPTY, FLOWS, INVALID),
    register=register,
    cases=cases,
    functions=functions,
    context={"today": TODAY.isoformat()},
)
