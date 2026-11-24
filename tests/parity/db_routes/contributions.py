# SPDX-License-Identifier: AGPL-3.0-only
"""The periodic contributions: GET /api/v1/contributions.

The contributions database holds contributions of every frequency, active and
inactive, starting before and after today, ending before and after the next date,
from real and virtual sources, for entities that are enabled and disabled and for
one linked to an entity account. They are written by the Python repository. The
invalid database holds a contribution in a spelling pydantic reads but the writer
does not produce: Rust passes that request on.

The route takes today's date from the clock; the recording fixes it at TODAY,
which the Rust test sets as its clock. `functions` records
`_next_contribution_date` for many contributions and days.
"""

import json
import sys
from datetime import date, timedelta
from pathlib import Path
from uuid import UUID

# The generator imports this slice before the harness, which sets up the path for
# the backend, so set it up here as well.
sys.path.insert(0, str(Path(__file__).resolve().parents[3] / "finanze"))

import application.use_cases.get_contributions as get_contributions_module  # noqa: E402
from application.use_cases.get_contributions import (
    GetContributionsImpl,
    _next_contribution_date,
)
from domain.auto_contributions import (
    AutoContributions,
    ContributionFrequency,
    ContributionTargetSubtype,
    ContributionTargetType,
    PeriodicContribution,
)
from domain.dezimal import Dezimal
from domain.entity import EntityOrigin, EntityType
from domain.fetch_record import DataSource
from infrastructure.controller.config import FJSONProvider
from infrastructure.controller.routes.contributions import contributions
from infrastructure.repository.auto_contributions.auto_contributions_repository import (
    AutoContributionsSQLRepository,
)
from infrastructure.repository.entity.entity_repository import EntitySQLRepository

from db_routes.harness import EMPTY, Scenario, Slice, case

PATH = "/api/v1/contributions"
TODAY = date(2026, 11, 18)


class _Today:
    """The day `date.today()` answers in the use case."""

    value = TODAY


class FixedDate(date):
    @classmethod
    def today(cls):
        return _Today.value


get_contributions_module.date = FixedDate

E1 = "a0000000-0000-4000-8000-000000000001"
E2 = "a0000000-0000-4000-8000-000000000002"
E3 = "a0000000-0000-4000-8000-000000000003"
E4 = "a0000000-0000-4000-8000-000000000004"
E5 = "a0000000-0000-4000-8000-000000000005"
EA1 = "b0000000-0000-4000-8000-000000000001"
EA2 = "b0000000-0000-4000-8000-000000000002"
CREATED = "2026-02-01T10:00:00+01:00"

ENTITIES = [
    (E1, "Bank", EntityType.FINANCIAL_INSTITUTION, EntityOrigin.NATIVE, None),
    (E2, "Manual", EntityType.FINANCIAL_INSTITUTION, EntityOrigin.MANUAL, "tag"),
    (E3, "Exchange", EntityType.CRYPTO_EXCHANGE, EntityOrigin.NATIVE, None),
    (E4, "Sheets", EntityType.FINANCIAL_INSTITUTION, EntityOrigin.MANUAL, None),
    (E5, "Wallet", EntityType.CRYPTO_WALLET, EntityOrigin.NATIVE, None),
]


def _contribution(
    index,
    entity_id,
    frequency,
    since,
    *,
    until=None,
    active=True,
    source=DataSource.REAL,
    target_type=ContributionTargetType.FUND,
    target_subtype=None,
    alias=None,
    target_name=None,
    amount="100.005",
    entity_account_id=None,
):
    return PeriodicContribution(
        id=UUID(f"c1000000-0000-4000-8000-{index:012d}"),
        alias=alias,
        target=f"target-{index}",
        target_name=target_name or f"Target {index}",
        target_type=target_type,
        target_subtype=target_subtype,
        amount=Dezimal(amount),
        currency="EUR",
        since=since,
        until=until,
        frequency=frequency,
        active=active,
        source=source,
        entity_account_id=UUID(entity_account_id) if entity_account_id else None,
    )


WRITTEN = [
    (
        E1,
        DataSource.REAL,
        [
            _contribution(
                1, E1, ContributionFrequency.MONTHLY, date(2026, 1, 31), alias="Rent"
            ),
            _contribution(2, E1, ContributionFrequency.WEEKLY, date(2026, 11, 17)),
            _contribution(
                3,
                E1,
                ContributionFrequency.BIWEEKLY,
                date(2020, 2, 29),
                until=date(2026, 11, 20),
            ),
            _contribution(
                4,
                E1,
                ContributionFrequency.YEARLY,
                date(2027, 3, 1),
                until=date(2026, 1, 1),
            ),
            _contribution(5, E1, ContributionFrequency.MONTHLY, TODAY, active=False),
            _contribution(
                6,
                E1,
                ContributionFrequency.BIMONTHLY,
                date(2026, 1, 1),
                source=DataSource.MANUAL,
            ),
            _contribution(
                7,
                E1,
                ContributionFrequency.QUARTERLY,
                date(2025, 12, 31),
                target_type=ContributionTargetType.STOCK_ETF,
                target_subtype=ContributionTargetSubtype.ETF,
                target_name="Ü ñ €",
                amount="12345678901234.56789",
            ),
            _contribution(8, E1, ContributionFrequency.SEMIANNUAL, date(1, 1, 1)),
        ],
    ),
    (
        E2,
        DataSource.MANUAL,
        [_contribution(9, E2, ContributionFrequency.MONTHLY, date(2026, 1, 1))],
    ),
    (
        E3,
        DataSource.REAL,
        [
            _contribution(
                10,
                E3,
                ContributionFrequency.EVERY_FOUR_MONTHS,
                date(2026, 2, 1),
                entity_account_id=EA2,
            )
        ],
    ),
    (
        E4,
        DataSource.SHEETS,
        [_contribution(11, E4, ContributionFrequency.YEARLY, date(2024, 2, 29))],
    ),
    (
        E5,
        DataSource.REAL,
        [_contribution(12, E5, ContributionFrequency.WEEKLY, date(2026, 11, 18))],
    ),
]


async def fill(client):
    async with client.tx() as cursor:
        for entity_id, name, entity_type, origin, icon in ENTITIES:
            await cursor.execute(
                "INSERT INTO entities (id, name, natural_id, type, origin, icon_url) "
                "VALUES (?, ?, ?, ?, ?, ?)",
                (entity_id, name, name.lower(), entity_type.value, origin.value, icon),
            )
        for account_id, entity_id in ((EA1, E1), (EA2, E3)):
            await cursor.execute(
                "INSERT INTO entity_accounts (id, name, entity_id, created_at, deleted_at) "
                "VALUES (?, ?, ?, ?, NULL)",
                (account_id, account_id, entity_id, CREATED),
            )
        # E4 is manual, but its latest import keeps it from being disabled.
        await cursor.execute(
            "INSERT INTO virtual_data_imports (id, import_id, source, date, feature, entity_id) "
            "VALUES (?, ?, ?, ?, ?, ?)",
            (
                "d0000000-0000-4000-8000-000000000001",
                "import-1",
                "MANUAL",
                "2026-11-01T00:00:00",
                "POSITION",
                E4,
            ),
        )
    repository = AutoContributionsSQLRepository(client)
    for entity_id, source, periodic in WRITTEN:
        await repository.save(entity_id, AutoContributions(periodic=periodic), source)


# pydantic reads a text flag and a date in forms the writer never stores.
INVALID_ROW = (
    "c2000000-0000-4000-8000-000000000001",
    E1,
    "target",
    "FUND",
    None,
    None,
    "Target",
    "10",
    "EUR",
    "20260101",
    None,
    "MONTHLY",
    "true",
    1,
    "REAL",
    CREATED,
    None,
)


async def fill_invalid(client):
    async with client.tx() as cursor:
        await cursor.execute(
            "INSERT INTO entities (id, name, natural_id, type, origin, icon_url) "
            "VALUES (?, ?, ?, ?, ?, ?)",
            (
                E1,
                "Bank",
                None,
                EntityType.FINANCIAL_INSTITUTION.value,
                EntityOrigin.NATIVE.value,
                None,
            ),
        )
        await cursor.execute(
            "INSERT INTO entity_accounts (id, name, entity_id, created_at, deleted_at) "
            "VALUES (?, ?, ?, ?, NULL)",
            (EA1, EA1, E1, CREATED),
        )
        await cursor.execute(
            "INSERT INTO periodic_contributions (id, entity_id, target, target_type, "
            "target_subtype, alias, target_name, amount, currency, since, until, "
            "frequency, active, is_real, source, created_at, entity_account_id) "
            "VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
            INVALID_ROW,
        )


# Every table a scenario writes is compared row by row. `created_at` is taken
# from the clock when the repository saves a contribution, so its value differs
# between runs and the dump keeps a marker for it.
TABLES = {
    "entities": (),
    "entity_accounts": (),
    "periodic_contributions": ("created_at",),
    "virtual_data_imports": (),
}
OWN_ENTITIES = {entity[0] for entity in ENTITIES}


def _normalize(connection, table, columns, rows):
    """The migrations seed entities with new ids; compare only this scenario's."""
    if table != "entities":
        return rows
    index = columns.index("id")
    return [row for row in rows if row[index] in OWN_ENTITIES]


CONTRIBUTIONS = Scenario("contributions", fill, TABLES, _normalize)
INVALID = Scenario("contributions_invalid", fill_invalid, TABLES, _normalize)


def register(app, client):
    use_case = GetContributionsImpl(
        AutoContributionsSQLRepository(client), EntitySQLRepository(client)
    )

    @app.route(PATH, methods=["GET"])
    async def contributions_route():
        _Today.value = TODAY
        return await contributions(use_case)


def cases():
    def get(name, database, **options):
        return case(name, database, PATH, **options)

    return [
        get("no contributions", "empty"),
        get("contributions", "contributions"),
        get("a query is ignored", "contributions", query="stats=true"),
        get("one entity", "contributions", query=f"entity={E1}"),
        get("two entities", "contributions", query=f"entity={E2}&entity={E3}"),
        get(
            "an entity id Python rejects",
            "contributions",
            query="entity=not-a-uuid",
            compare="forwarded",
            passed_on=True,
        ),
        get("real contributions", "contributions", query="real=true"),
        get("virtual contributions", "contributions", query="real=FALSE"),
        get(
            "a flag neither true nor false is ignored",
            "contributions",
            query="real=maybe",
        ),
        get(
            "a row only Python reads",
            "contributions_invalid",
            compare="forwarded",
            passed_on=True,
        ),
        get(
            "no session while unlocked",
            "contributions",
            session="none",
            compare="forwarded",
        ),
        get(
            "unknown session while unlocked",
            "contributions",
            session="unknown",
            compare="forwarded",
        ),
        get(
            "locked",
            "contributions",
            session="none",
            unlocked=False,
            compare="forwarded",
        ),
        get(
            "locked with an ended session",
            "contributions",
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
]
STARTS = [
    date(1, 1, 1),
    date(2024, 1, 31),
    date(2024, 2, 29),
    date(2026, 11, 17),
    date(2026, 11, 18),
    date(2026, 11, 19),
    date(9999, 12, 31),
]


def _next_dates():
    entries = []
    for today in DAYS:
        _Today.value = today
        for frequency in ContributionFrequency:
            for since in STARTS:
                contribution = _contribution(0, E1, frequency, since)
                try:
                    plain = _next_contribution_date(contribution)
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
                    contribution.until = until
                    for active in (True, False):
                        contribution.active = active
                        result = _next_contribution_date(contribution)
                        entries.append(
                            [
                                frequency.value,
                                since.isoformat(),
                                until.isoformat() if until else None,
                                active,
                                today.isoformat(),
                                {"ok": result.isoformat() if result else None},
                            ]
                        )
    _Today.value = TODAY
    return entries


def functions():
    return {"next_date": _next_dates()}


SLICE = Slice(
    "contributions",
    (EMPTY, CONTRIBUTIONS, INVALID),
    register=register,
    cases=cases,
    functions=functions,
    context={"today": TODAY.isoformat()},
)
