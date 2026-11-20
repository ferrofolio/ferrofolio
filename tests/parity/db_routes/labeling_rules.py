# SPDX-License-Identifier: AGPL-3.0-only
"""The labeling rules: GET /api/v1/labeling/rules.

The empty database holds the rules the migrations seed. The labels database (see
labels.py) adds rules of both kinds, enabled and disabled, with and without
labels, with every condition the writer stores, ids in spellings Python reads but
does not write, and conditions in forms an older writer or a hand-edited database
may hold that Rust still reads exactly as Python does. The invalid database holds
a rule whose conditions only Python's coercions read: Rust passes the request on.

`functions` records how Python reads stored conditions, for every form the
writer produces and for many it does not: Rust must read the first exactly as
Python does, and may only pass on, never answer, where Python fails.
"""

import json
from datetime import date
from uuid import UUID

from application.use_cases.get_labeling_rules import GetLabelingRulesImpl
from domain.dezimal import Dezimal
from domain.labeling import (
    LabelingRuleConditions,
    TextCondition,
    TextMatchField,
    TextMatchOperator,
)
from domain.transactions import TxType
from infrastructure.controller.config import FJSONProvider
from infrastructure.controller.routes.get_labeling_rules import get_labeling_rules
from infrastructure.repository.labeling.labeling_rule_repository import (
    LabelingRuleRepository,
    deserialize_conditions,
    serialize_conditions,
)

from db_routes.harness import EMPTY, Slice, case
from db_routes.labels import ENTITY, INVALID, LABELS, LABELS_DB

PATH = "/api/v1/labeling/rules"

# Not IBANs of any account: the country code XX does not exist.
IBANS = ["XX00TEST0000000001", "XX00TEST0000000002"]

ZOO, ALPHA, UPPER, BRACES, CUSTOM = (label[0] for label in LABELS)

# Conditions as serialize_conditions writes them.
WRITTEN = [
    [
        LabelingRuleConditions(
            types=[TxType.OUTFLOW, TxType.FEE],
            min_amount=Dezimal("1.50"),
            max_amount=Dezimal("250"),
            currency="EUR",
            text=TextCondition(
                values=["Rewe", "Café Ü"],
                operator=TextMatchOperator.CONTAINS,
                field=TextMatchField.ANY,
            ),
        ),
        LabelingRuleConditions(
            entities=[UUID(ENTITY)],
            from_date=date(2026, 1, 1),
            to_date=date(2026, 12, 31),
            day_from=25,
            day_to=5,
        ),
    ],
    [LabelingRuleConditions(ibans=IBANS, max_days=2, min_amount=Dezimal("0.01"))],
    [
        LabelingRuleConditions(
            text=TextCondition(
                values=[r"^amzn\s+mktp", '"quoted" \\ back'],
                operator=TextMatchOperator.REGEX,
                field=TextMatchField.COUNTERPARTY,
            ),
            max_amount=Dezimal("12345678901234567890.123456789"),
        )
    ],
    [LabelingRuleConditions(currency="usd", from_date=date(1, 1, 1))],
    [
        LabelingRuleConditions(
            types=[TxType.INTEREST],
            text=TextCondition(
                values=["x"],
                operator=TextMatchOperator.STARTS_WITH,
                field=TextMatchField.NAME,
            ),
        ),
        LabelingRuleConditions(
            text=TextCondition(values=["y"], operator=TextMatchOperator.EQUALS),
            to_date=date(9999, 12, 31),
            min_amount=Dezimal("0"),
        ),
    ],
]

# Forms the writer does not produce that Rust reads exactly as Python does.
OTHER_FORMS = [
    # Falsy values, an unknown key, keys in another order, no operator or field.
    '[{"types": [], "entities": null, "from_date": "", "ibans": [], "legacy": 1,'
    ' "text": {"values": ["a"]}, "currency": null}, {}]',
    # The text of an empty list, surrounding whitespace, amounts in other
    # spellings.
    "",
    " [{}] ",
    '[{"min_amount": "1E+2", "max_amount": " 1_000.5 "}, {"text": {}}]',
    '[{"min_amount": "-0", "max_amount": "0.1", "day_from": -3, "max_days": 0}]',
    '[{"min_amount": "0e0", "max_amount": "0e-500"}]',
    '[{"entities": ["{E0000000-0000-0000-0000-000000000001}",'
    ' "urn:uuid:e0000000000000000000000000000002"]}]',
]

CREATED = "2026-03-01T09:00:00+01:00"

# id as stored, name, enabled, kind, conditions, created_at, label ids as stored.
RULES = [
    (
        "a1000000-0000-4000-8000-000000000001",
        "Groceries",
        True,
        "MATCH",
        serialize_conditions(WRITTEN[0]),
        CREATED,
        [ZOO, ALPHA],
    ),
    (
        "a1000000-0000-4000-8000-000000000002",
        None,
        False,
        "TRANSFER",
        serialize_conditions(WRITTEN[1]),
        "2026-02-01T09:00:00+01:00",
        # Linked in upper case, read in its canonical form.
        [UPPER.upper()],
    ),
    (
        # The same creation time as the first rule: the id orders them.
        "A1000000-0000-4000-8000-000000000000",
        "Ü ñ €",
        True,
        "MATCH",
        serialize_conditions(WRITTEN[2]),
        CREATED,
        [BRACES, CUSTOM, ZOO],
    ),
    (
        "a1000000-0000-4000-8000-000000000004",
        "No labels",
        True,
        "MATCH",
        serialize_conditions(WRITTEN[3]),
        "2026-03-02T09:00:00+01:00",
        [],
    ),
    (
        "{a1000000-0000-4000-8000-000000000005}",
        "Other forms",
        True,
        "MATCH",
        OTHER_FORMS[0],
        "2026-03-03T09:00:00+01:00",
        [ALPHA],
    ),
    (
        "a1000000-0000-4000-8000-000000000006",
        "Empty conditions",
        True,
        "MATCH",
        "",
        "2026-03-04T09:00:00+01:00",
        [],
    ),
    (
        "a1000000-0000-4000-8000-000000000007",
        "Two groups",
        True,
        "MATCH",
        serialize_conditions(WRITTEN[4]),
        "2026-03-05T09:00:00+01:00",
        [CUSTOM],
    ),
]

# pydantic reads "3" as the day 3; Rust leaves such a value to Python.
UNREADABLE_RULES = [
    (
        "b1000000-0000-4000-8000-000000000001",
        "Coerced",
        True,
        "MATCH",
        '[{"day_from": "3"}]',
        CREATED,
        [],
    ),
]


def register(app, client):
    get_labeling_rules_uc = GetLabelingRulesImpl(LabelingRuleRepository(client))

    @app.route(PATH, methods=["GET"])
    async def labeling_rules_route():
        return await get_labeling_rules(get_labeling_rules_uc)


def cases():
    def get(name, database, **options):
        return case(name, database, PATH, **options)

    return [
        get("seeded rules", "empty"),
        get("rules", "labels"),
        get("a query is ignored", "labels", query="enabled_only=true"),
        get(
            "conditions only Python reads",
            "labels_invalid",
            compare="forwarded",
            passed_on=True,
        ),
        # Rust passes these on to Python, which answers as recorded.
        get("no session while unlocked", "labels", session="none", compare="forwarded"),
        get(
            "unknown session while unlocked",
            "labels",
            session="unknown",
            compare="forwarded",
        ),
        get("locked", "labels", session="none", unlocked=False, compare="forwarded"),
        get(
            "locked with an ended session",
            "labels",
            session="unknown",
            unlocked=False,
            compare="forwarded",
        ),
    ]


# Forms Python reads, coerces or rejects that Rust leaves to Python.
LEFT_TO_PYTHON = [
    "{}",
    "[1]",
    "[[]]",
    "null",
    "NaN",
    '[{"day_from": 3.0}]',
    '[{"day_from": "3"}]',
    '[{"day_from": true}]',
    '[{"day_from": 99999999999999999999}]',
    '[{"day_from": {"$serde_json::private::Number": "3"}}]',
    '[{"day_from": {"\\u0024serde_json::private::Number": "3"}}]',
    '[{"currency": {"$serde_json::private::Number": "3"}}]',
    '[{"min_amount": 1}]',
    '[{"min_amount": 1.5}]',
    '[{"min_amount": "NaN"}]',
    '[{"min_amount": "Infinity"}]',
    '[{"min_amount": "1e400"}]',
    '[{"min_amount": "0e1000000000000000000"}]',
    '[{"min_amount": "0e999999999999999999"}]',
    '[{"min_amount": ""}]',
    '[{"min_amount": "1.5.0"}]',
    '[{"min_amount": "' + "1" * 40 + '"}]',
    '[{"types": ["inflow"]}]',
    '[{"types": "INFLOW"}]',
    '[{"types": [null]}]',
    '[{"entities": ["x"]}]',
    '[{"entities": [1]}]',
    '[{"entities": "e0000000000000000000000000000001"}]',
    '[{"from_date": "20260101"}]',
    '[{"from_date": "2026-W01-1"}]',
    '[{"from_date": "0000-01-01"}]',
    '[{"from_date": "2026-02-30"}]',
    '[{"from_date": "２０２６-01-01"}]',
    '[{"from_date": 0}]',
    '[{"to_date": "2026-01-01T00:00"}]',
    '[{"text": {"operator": "REGEX"}}]',
    '[{"text": {"values": "abc"}}]',
    '[{"text": {"values": [1]}}]',
    '[{"text": {"values": [], "operator": null}}]',
    '[{"text": {"values": [], "field": "any"}}]',
    '[{"text": "abc"}]',
    '[{"text": ["values"]}]',
    '[{"currency": 1}]',
    '[{"ibans": [1]}]',
    '[{"ibans": "XX00"}]',
    '[{"max_days": "2"}]',
]


class _App:
    """What FJSONProvider needs of an app: nothing but a weak reference."""


# The provider holds only a weak reference to its app.
_APP = _App()
_PROVIDER = FJSONProvider(_APP)


def _read(raw):
    """deserialize_conditions(raw) as jsonify answers it, or the error."""
    try:
        groups = deserialize_conditions(raw)
    except Exception as error:  # noqa: BLE001 - every failure is recorded
        return {"error": type(error).__name__}
    answer = json.loads(_PROVIDER.dumps(groups))
    try:
        json.dumps(answer, allow_nan=False)
    except ValueError:
        return {"error": "the answer is not JSON"}
    return {"ok": answer}


def functions():
    written = [serialize_conditions(groups) for groups in WRITTEN]
    return {
        "conditions": [
            [raw, _read(raw), kind]
            for kind, raws in (
                ("written", written),
                ("other", OTHER_FORMS),
                ("python", LEFT_TO_PYTHON),
            )
            for raw in raws
        ],
    }


SLICE = Slice(
    "labeling_rules",
    (EMPTY, LABELS_DB, INVALID),
    register=register,
    cases=cases,
    functions=functions,
)
