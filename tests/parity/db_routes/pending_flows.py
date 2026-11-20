# SPDX-License-Identifier: AGPL-3.0-only
"""The pending flows: GET /api/v1/flows/pending.

The empty database has no flows. The pending flows database holds flows of both
types and every status, in two currencies, with and without a date, a category
(also an empty one) and a time of the last status change (aware with several
offsets, with microseconds, naive), equal amounts and equal dates, so that the
stable sort shows, written by the repository, and a few in spellings Python reads
but does not write. The invalid database holds a flow whose time only pydantic's
coercion reads; the huge database holds active amounts whose sum a float cannot
hold. Rust passes the requests on that need either.

The cases cover the filters, both sort fields and orders, paging with Python's
slice rules, the statistics and categories, and every way the query fails.
`functions` records `_map_row_to_pending_flow` for rows in the forms the writer
stores and in many it does not, and `args.get(name, type=int)` for many texts.
"""

import json
from datetime import date, datetime, timedelta, timezone
from uuid import UUID

from application.use_cases.query_pending_flows import QueryPendingFlowsImpl
from domain.dezimal import Dezimal
from domain.earnings_expenses import FlowStatus, FlowType, PendingFlow
from infrastructure.controller.config import FJSONProvider
from infrastructure.controller.routes.get_pending_flows import (
    _serialize_flow,
    get_pending_flows,
)
from infrastructure.repository.earnings_expenses.pending_flow_repository import (
    PendingFlowRepository,
    _map_row_to_pending_flow,
)
from werkzeug.datastructures import MultiDict

from db_routes.harness import EMPTY, Scenario, Slice, case

PATH = "/api/v1/flows/pending"

CET = timezone(timedelta(hours=1))
IST = timezone(timedelta(hours=5, minutes=30))
WEST = timezone(timedelta(hours=-3, minutes=-30))


def _flow(
    index,
    name,
    amount,
    *,
    currency="EUR",
    flow_type=FlowType.EXPENSE,
    category=None,
    status=FlowStatus.ACTIVE,
    on=None,
    icon=None,
    changed=None,
):
    return PendingFlow(
        id=UUID(f"f4000000-0000-4000-8000-{index:012d}"),
        name=name,
        amount=Dezimal(amount),
        currency=currency,
        flow_type=flow_type,
        category=category,
        status=status,
        date=on,
        icon=icon,
        status_changed_at=changed,
    )


EARNING = FlowType.EARNING
ACTIVE, DISABLED, COMPLETED = (
    FlowStatus.ACTIVE,
    FlowStatus.DISABLED,
    FlowStatus.COMPLETED,
)

# Written by PendingFlowRepository.save.
WRITTEN = [
    _flow(
        1,
        "Car tax",
        "250",
        category="Car",
        on=date(2026, 12, 15),
        icon="car",
        changed=datetime(2026, 11, 1, 9, 30, tzinfo=CET),
    ),
    # The same amount as the car tax, written later: it stays after it in
    # both orders.
    _flow(2, "Insurance", "250.00", category="Car", on=date(2026, 12, 15)),
    _flow(
        3,
        "Bonus",
        "1500.5",
        flow_type=EARNING,
        category="Work",
        on=date(2027, 3, 1),
        changed=datetime(2026, 11, 2, 23, 59, 59, 123456, tzinfo=timezone.utc),
    ),
    _flow(4, "Refund", "42.10", currency="USD", flow_type=EARNING, category="Shop"),
    _flow(
        5,
        "Dentist",
        "80",
        category="Health",
        status=COMPLETED,
        on=date(2026, 10, 1),
        changed=datetime(2026, 10, 1, 12, 0, 0, 1, tzinfo=IST),
    ),
    _flow(
        6,
        "Gym",
        "35.99",
        category="Health",
        status=DISABLED,
        changed=datetime(2026, 9, 1, 8, 0),
    ),
    _flow(7, "Gift", "0.01", category="", on=date(1, 1, 1)),
    _flow(8, "Ü ñ €", "12345678901234.56789", currency="USD", on=date(9999, 12, 31)),
    _flow(9, "Repair", "-20", category="Car", on=date(2026, 12, 15)),
    _flow(
        10,
        "Dividend",
        "0",
        flow_type=EARNING,
        category="Work",
        status=COMPLETED,
        changed=datetime(2026, 11, 3, 7, 15, tzinfo=WEST),
    ),
    _flow(11, "Old fee", "5", category="Bank", status=DISABLED, on=date(2025, 1, 31)),
    _flow(12, "Health check", "120", category="Health", on=date(2027, 1, 10)),
]

# Rows in spellings Python reads but its writer does not produce: the id and the
# amount. Columns: id, name, amount, currency, flow_type, category, status,
# status_changed_at, date, icon.
OTHER_ROWS = [
    (
        "F4000000-0000-4000-8000-000000000020",
        "Upper-case id",
        " 1_000.5 ",
        "EUR",
        "EXPENSE",
        "Work",
        "ACTIVE",
        None,
        "2026-12-01",
        "",
    ),
    (
        "{f4000000-0000-4000-8000-000000000021}",
        "Braced id",
        "-0",
        "USD",
        "EARNING",
        None,
        "ACTIVE",
        "2026-11-18T10:00:00+00:00",
        None,
        None,
    ),
]

# pydantic reads this time with `Z`; Rust leaves it to Python.
UNREADABLE_ROWS = [
    (
        "f5000000-0000-4000-8000-000000000001",
        "Coerced",
        "10",
        "EUR",
        "EXPENSE",
        None,
        "ACTIVE",
        "2026-11-18 10:00:00Z",
        None,
        None,
    ),
]

# Each a float, but not their sum.
HUGE = [
    _flow(30, "Huge", "1E+308"),
    _flow(31, "Huger", "1.5E+308", category="Big"),
]

INSERT = (
    "INSERT INTO pending_flows (id, name, amount, currency, flow_type, category, "
    "status, status_changed_at, date, icon) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)"
)


async def fill(client):
    repository = PendingFlowRepository(client)
    for flow in WRITTEN:
        await repository.save(flow)
    async with client.tx() as cursor:
        for row in OTHER_ROWS:
            await cursor.execute(INSERT, row)


async def fill_invalid(client):
    async with client.tx() as cursor:
        for row in UNREADABLE_ROWS:
            await cursor.execute(INSERT, row)


async def fill_huge(client):
    repository = PendingFlowRepository(client)
    for flow in HUGE:
        await repository.save(flow)


TABLES = {"pending_flows": ()}
FLOWS = Scenario("pending_flows", fill, TABLES)
INVALID = Scenario("pending_flows_invalid", fill_invalid, TABLES)
HUGE_SUMS = Scenario("pending_flows_huge", fill_huge, TABLES)


def register(app, client):
    query_pending_flows_uc = QueryPendingFlowsImpl(PendingFlowRepository(client))

    @app.route(PATH, methods=["GET"])
    async def pending_flows_route():
        return await get_pending_flows(query_pending_flows_uc)


# Queries Rust answers like Python, by name.
QUERIES = {
    "the default order": None,
    "an unknown argument": "enabled=false",
    "active": "status=ACTIVE",
    "two statuses": "status=COMPLETED&status=DISABLED",
    "a status twice": "status=ACTIVE&status=ACTIVE",
    "earnings": "flow_type=EARNING",
    "expenses, the second type ignored": "flow_type=EXPENSE&flow_type=EARNING",
    "an empty type": "flow_type=",
    "an empty first type": "flow_type=&flow_type=x",
    "a category": "category=Car",
    "two categories": "category=Car&category=Health",
    "the empty category": "category=",
    "an unknown category": "category=car",
    "a category with spaces and escapes": "category=Car+%26+more&category=%C3%9C",
    "by amount ascending": "order=asc",
    "by amount descending": "sort_by=amount&order=desc",
    "by date": "sort_by=date",
    "by date ascending": "sort_by=date&order=asc",
    "an empty sort field and order": "sort_by=&order=",
    "the first page": "page=1&limit=3",
    "the second page": "page=2&limit=3",
    "the last page": "page=5&limit=3",
    "after the last page": "page=6&limit=3",
    "far after the last page": "page=9223372036854775807&limit=9223372036854775807",
    "page zero": "page=0&limit=3",
    "a negative page": "page=-1&limit=3",
    "the most negative page": "page=-9223372036854775808&limit=2",
    "a negative limit": "page=2&limit=-3",
    "a zero limit": "page=1&limit=0",
    "a page without a limit": "page=2",
    "a limit without a page": "limit=2",
    "a page that is no integer": "page=x&limit=2",
    "a page with a fraction": "page=2.0&limit=2",
    "a page in spaces and with underscores": "page=+%201_0%20&limit=1",
    "a page in other digits": "page=%D9%A2&limit=%E0%A5%A9",
    "a page with a separator int keeps": "page=%1C2%1F&limit=2",
    "an empty page": "page=&limit=2",
    "the first page value counts": "page=2&page=1&limit=3",
    "statistics": "stats=true",
    "statistics in capitals": "stats=TRUE&categories=True",
    "statistics only for true": "stats=1&categories=yes",
    "statistics of categories": "stats=true&category=Car&category=",
    "statistics of an empty category": "stats=true&category=",
    "statistics with filters that do not apply to them": (
        "stats=true&status=COMPLETED&flow_type=EARNING"
    ),
    "categories": "categories=true",
    "categories with a page": "categories=true&page=1&limit=1&status=ACTIVE",
    "everything": (
        "status=ACTIVE&status=COMPLETED&flow_type=EXPENSE&category=Car"
        "&category=Health&sort_by=date&order=asc&page=1&limit=2&stats=true"
        "&categories=true"
    ),
}

# Queries Python rejects with 400.
INVALID_QUERIES = {
    "an unknown status": "status=x",
    "a status in lower case": "status=active",
    "an empty status": "status=",
    "an unknown type": "flow_type=income",
    "an unknown sort field": "sort_by=name",
    "an unknown order": "order=DESC",
    "the status fails first": "order=x&sort_by=y&flow_type=z&status=ACTIVE&status=w",
    "the type fails before the sort field": "sort_by=y&flow_type=z",
    "a quote in the value": "flow_type=it%27s",
    "both quotes in the value": "flow_type=%22it%27s%22",
    "a non-ASCII value": "status=%C3%A9t%C3%A9",
    "bytes that are not UTF-8": "status=%FF%FE",
    "a control character": "sort_by=%07",
    "an invalid value with a page Rust leaves to Python": (
        "order=x&page=99999999999999999999"
    ),
}

# Queries with a page or limit beyond i64, which Rust passes on.
PASSED_ON_QUERIES = {
    "a page beyond i64": "page=9223372036854775808&limit=1",
    "a limit beyond i64": "page=1&limit=-9223372036854775809",
    "a page with more digits than int reads": "page=" + "1" * 4301,
}


def cases():
    def get(name, database, **options):
        return case(name, database, PATH, **options)

    entries = [
        get("no flows", "empty"),
        get("no flows with statistics", "empty", query="stats=true&categories=true"),
    ]
    entries += [
        get(name, "pending_flows", query=query) for name, query in QUERIES.items()
    ]
    entries += [
        get(name, "pending_flows", query=query, compare="status_code")
        for name, query in INVALID_QUERIES.items()
    ]
    entries += [
        get(name, "pending_flows", query=query, compare="forwarded", passed_on=True)
        for name, query in PASSED_ON_QUERIES.items()
    ]
    entries += [
        get(
            "a time only Python reads",
            "pending_flows_invalid",
            compare="forwarded",
            passed_on=True,
        ),
        get("huge amounts", "pending_flows_huge"),
        get(
            "huge amounts with statistics",
            "pending_flows_huge",
            query="stats=true",
            compare="forwarded",
            passed_on=True,
        ),
        get(
            "huge amounts with statistics of one category",
            "pending_flows_huge",
            query="stats=true&category=Big",
        ),
        # Rust passes these on to Python, which answers as recorded.
        get(
            "no session while unlocked",
            "pending_flows",
            session="none",
            compare="forwarded",
        ),
        get(
            "unknown session while unlocked",
            "pending_flows",
            session="unknown",
            compare="forwarded",
        ),
        get(
            "locked",
            "pending_flows",
            session="none",
            unlocked=False,
            compare="forwarded",
        ),
        get(
            "locked with an ended session",
            "pending_flows",
            session="unknown",
            unlocked=False,
            compare="forwarded",
        ),
    ]
    return entries


class _App:
    """What FJSONProvider needs of an app: nothing but a weak reference."""


_PROVIDER = FJSONProvider(_App())


def _json(value):
    """value as jsonify answers it, or None when that is not JSON."""
    answer = json.loads(_PROVIDER.dumps(value))
    try:
        json.dumps(answer, allow_nan=False)
    except ValueError:
        return None
    return answer


def _row(**changes):
    row = {
        "id": "f6000000-0000-4000-8000-000000000001",
        "name": "Row",
        "amount": "12.50",
        "currency": "EUR",
        "flow_type": "EXPENSE",
        "category": None,
        "status": "ACTIVE",
        "status_changed_at": None,
        "date": None,
        "icon": None,
    }
    row.update(changes)
    return row


# Rows as the writer stores them.
WRITTEN_ROWS = [
    _row(),
    _row(
        name="Ü ñ €",
        amount="850.5",
        category="Housing",
        status="COMPLETED",
        status_changed_at="2026-11-20T14:36:09.123456+01:00",
        date="2026-12-31",
        icon="house",
    ),
    _row(status="DISABLED", status_changed_at="2026-11-20T14:36:09"),
    _row(status_changed_at="2026-11-20T14:36:09.000001-03:30"),
    _row(status_changed_at="0001-01-01T00:00:00+05:45"),
    _row(status_changed_at="9999-12-31T23:59:59.999999-23:59"),
    _row(flow_type="EARNING", amount="1E+3", date="0001-01-01"),
    _row(amount="12345678901234567890.123456789", date="9999-12-31"),
]

# Forms the writer does not produce that Rust reads exactly as Python does.
OTHER_FORM_ROWS = [
    _row(id="f6000000-0000-4000-8000-000000000001".replace("-", "").upper()),
    _row(id="{f6000000-0000-4000-8000-000000000001}"),
    _row(id="urn:uuid:f6000000-0000-4000-8000-000000000001"),
    _row(amount=" 1_000.5 "),
    _row(amount="-0"),
    _row(amount="0e-500"),
    _row(category="", icon=""),
]

# Forms Python reads, coerces or rejects that Rust leaves to Python.
LEFT_TO_PYTHON_ROWS = [
    _row(status="active"),
    _row(status="ENABLED"),
    _row(status=None),
    _row(status=1),
    _row(status_changed_at="2026-11-20 14:36:09+01:00"),
    _row(status_changed_at="2026-11-20T14:36:09Z"),
    _row(status_changed_at="2026-11-20T14:36:09.000000"),
    _row(status_changed_at="2026-11-20T14:36:09.5"),
    _row(status_changed_at="2026-11-20T14:36:09-00:00"),
    _row(status_changed_at="2026-11-20T14:36:09+01:00:30"),
    _row(status_changed_at="2026-11-20T14:36"),
    _row(status_changed_at="2026-11-20"),
    _row(status_changed_at="1700000000"),
    _row(status_changed_at=1700000000),
    _row(status_changed_at=""),
    _row(status_changed_at="2026-11-20T24:00:00"),
    _row(date="20261201"),
    _row(date=20261201),
    _row(date=""),
    _row(date="2026-02-30"),
    _row(date="2026-12-01T00:00:00"),
    _row(amount="NaN"),
    _row(amount="Infinity"),
    _row(amount="1e400"),
    _row(amount=""),
    _row(amount=5),
    _row(amount=1.5),
    _row(flow_type="expense"),
    _row(id="x"),
    _row(id=None),
    _row(name=None),
    _row(name=5),
    _row(currency=None),
    _row(category=5),
    _row(icon=5),
]


def _mapped(row):
    """_map_row_to_pending_flow(row) as the route answers the flow, or the error."""
    try:
        flow = _map_row_to_pending_flow(row)
        answer = _json(_serialize_flow(flow))
    except Exception as error:  # noqa: BLE001 - every failure is recorded
        return {"error": type(error).__name__}
    if answer is None:
        return {"error": "the answer is not JSON"}
    return {"ok": answer}


# Texts for `args.get(name, type=int)`.
INT_TEXTS = [
    "0",
    "7",
    "-7",
    "+7",
    "007",
    "-0",
    " 12 ",
    "\t12\n",
    "\x1c12\x1f",
    "　" + "12" + "\x85",
    "​12",
    "1_000",
    "1__000",
    "_1",
    "1_",
    "+_1",
    "- 1",
    "--1",
    "1 2",
    "1.0",
    "1e3",
    "0x10",
    "١٢",
    "१२३",
    "1٢",
    "−1",
    "",
    " ",
    "+",
    "9223372036854775807",
    "-9223372036854775808",
    "9223372036854775808",
    "-9223372036854775809",
    "99999999999999999999999999999999999999999",
    "1" * 4300,
    "1" * 4301,
]


def _int(text):
    value = MultiDict([("page", text)]).get("page", type=int)
    # Too long for JSON readers that keep integers in 64 bits; the Rust test only
    # needs to know that it lies outside them.
    if value is not None and not -(2**63) <= value < 2**63:
        return "out of range"
    return value


def functions():
    return {
        "rows": [
            [row, _mapped(row), kind]
            for kind, rows in (
                ("written", WRITTEN_ROWS),
                ("other", OTHER_FORM_ROWS),
                ("python", LEFT_TO_PYTHON_ROWS),
            )
            for row in rows
        ],
        "ints": [[text, _int(text)] for text in INT_TEXTS],
    }


SLICE = Slice(
    "pending_flows",
    (EMPTY, FLOWS, INVALID, HUGE_SUMS),
    register=register,
    cases=cases,
    functions=functions,
)
