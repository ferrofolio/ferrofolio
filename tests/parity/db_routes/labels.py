# SPDX-License-Identifier: AGPL-3.0-only
"""The labels: GET /api/v1/labels.

The empty database holds the labels the migrations seed. The labels database adds
labels without a key, ordered by creation and name, ids in spellings Python reads
but does not write, and transactions that carry labels, so that the usage counts
differ per label. The invalid database holds a label whose id Python cannot read.
`functions` records `UUID()` and the label enums for many more stored values than
rows can show.
"""

import uuid

from application.use_cases.get_labels import GetLabelsImpl
from domain.labeling import LabelCategory, LabelSubcategory
from infrastructure.controller.routes.get_labels import get_labels
from infrastructure.repository.labeling.label_repository import LabelRepository

from db_routes.harness import EMPTY, Scenario, Slice, case

PATH = "/api/v1/labels"
ENTITY = "e0000000-0000-0000-0000-000000000001"
CREATED = "2026-02-01T10:00:00+01:00"

# id as stored, key, name, category, subcategory, created_at.
LABELS = [
    ("5b1d1a52-6e0b-4a54-8f7c-0a6c2e9d7b10", None, "Zoo", "EXPENSE", "WANTS", CREATED),
    ("1f0a9c3e-2b4d-4e6f-8a9b-0c1d2e3f4a5b", None, "Alpha", "EXPENSE", "NEED", CREATED),
    # Python reads an id in upper case and without hyphens, and answers it in its
    # canonical form.
    ("7C9E6679742540DE944BE07FC1F90AE7", None, "Upper", "INCOME", "NONE", CREATED),
    (
        "{0a1b2c3d-4e5f-4a6b-8c7d-9e0f1a2b3c4d}",
        None,
        "Braces",
        "EXCLUDED",
        "SAVINGS",
        "2026-01-01T00:00:00+01:00",
    ),
    (
        "2c3d4e5f-6a7b-4c8d-9e0f-1a2b3c4d5e6f",
        "custom_key",
        "Ü ñ €",
        "EXPENSE",
        "NEED",
        CREATED,
    ),
]

# Label keys or names, with the number of transactions carrying each. The label
# whose id is stored in upper case gets one more below.
USAGE = [("groceries", 3), ("internal_transfer", 1), ("Zoo", 2)]


async def _insert_labels(cursor, labels):
    for label_id, key, name, category, subcategory, created_at in labels:
        await cursor.execute(
            "INSERT INTO labels (id, key, name, description, color, icon, category, "
            "subcategory, created_at) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
            (
                label_id,
                key,
                name,
                f"{name} description" if name else None,
                "#123456",
                "tag",
                category,
                subcategory,
                created_at,
            ),
        )


async def _carry(cursor, label_id, count, prefix):
    for index in range(count):
        tx_id = f"{prefix}{index:04d}-0000-4000-8000-000000000000"
        await cursor.execute(
            "INSERT INTO account_transactions (id, ref, name, amount, currency, type, "
            "date, entity_id, is_real, created_at, labels_locked) "
            "VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
            (
                tx_id,
                f"ref-{prefix}-{index}",
                "Transaction",
                "-10.00",
                "EUR",
                "OUTFLOW",
                "2026-02-01T00:00:00",
                ENTITY,
                True,
                CREATED,
                False,
            ),
        )
        await cursor.execute(
            "INSERT INTO account_transaction_labels (tx_id, label_id, origin, created_at) "
            "VALUES (?, ?, ?, ?)",
            (tx_id, label_id, "MANUAL", CREATED),
        )


async def fill(client):
    async with client.tx() as cursor:
        await _insert_labels(cursor, LABELS)
        for index, (which, count) in enumerate(USAGE):
            await cursor.execute(
                "SELECT id FROM labels WHERE key = ? OR name = ?", (which, which)
            )
            (label_id,) = await cursor.fetchone()
            await _carry(cursor, label_id, count, f"{index:04x}")
        await _carry(cursor, LABELS[2][0], 1, "00aa")


async def fill_invalid(client):
    async with client.tx() as cursor:
        await _insert_labels(
            cursor,
            [
                ("not-a-uuid", "broken", None, "EXPENSE", "NEED", CREATED),
                (
                    "3d4e5f6a-7b8c-4d9e-8f0a-1b2c3d4e5f6a",
                    None,
                    "Wrong case",
                    "expense",
                    "NEED",
                    CREATED,
                ),
            ],
        )


# The seeded labels get a new id and creation time in every database.
SEEDED = ("id", "created_at")
LABELS_DB = Scenario(
    "labels",
    fill,
    {
        "labels": SEEDED,
        "account_transactions": (),
        "account_transaction_labels": ("label_id",),
    },
)
INVALID = Scenario("labels_invalid", fill_invalid, {"labels": SEEDED})


def register(app, client):
    get_labels_uc = GetLabelsImpl(LabelRepository(client))

    @app.route(PATH, methods=["GET"])
    async def labels_route():
        return await get_labels(get_labels_uc)


def cases():
    def get(name, database, **options):
        return case(name, database, PATH, **options)

    return [
        get("seeded labels", "empty"),
        get("labels with usage", "labels"),
        get("a query is ignored", "labels", query="key=groceries"),
        get("an id Python cannot read", "labels_invalid"),
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


UUIDS = [
    "0d3e7c1a-5b2f-4c8e-9a1b-2c3d4e5f6a7b",
    "0D3E7C1A5B2F4C8E9A1B2C3D4E5F6A7B",
    "{0d3e7c1a-5b2f-4c8e-9a1b-2c3d4e5f6a7b}",
    "{{0d3e7c1a5b2f4c8e9a1b2c3d4e5f6a7b}}}",
    "urn:uuid:0d3e7c1a-5b2f-4c8e-9a1b-2c3d4e5f6a7b",
    "uuid:0d3e7c1a5b2f4c8e9a1b2c3d4e5f6a7b",
    "0d3eurn:7c1a5b2f4c8e9a1b2c3d4e5f6a7b",
    "-0d3e-7c1a-5b2f-4c8e-9a1b-2c3d-4e5f-6a7b-",
    "+" + "a" * 31,
    " " + "a" * 31,
    "a" * 31 + " ",
    "　" + "a" * 31,
    "\t" + "a" * 31,
    "\x1c" + "a" * 31,
    "0x" + "a" * 30,
    "0X_" + "a" * 29,
    "0x__" + "a" * 28,
    "+0x" + "a" * 29,
    "a_" * 16,
    "_" + "a" * 31,
    "a" * 15 + "_" + "a" * 16,
    "a" * 15 + "__" + "a" * 15,
    "٣" * 32,
    "１" * 32,
    "ａ" * 32,
    "a" * 31 + "g",
    "a" * 31 + "\x00",
    "a" * 33,
    "a" * 31,
    "",
    "not-a-uuid",
    "'" * 32,
    "f" * 32,
    "0" * 32,
]

ENUM_VALUES = ["INCOME", "EXPENSE", "EXCLUDED", "NEED", "WANTS", "SAVINGS", "NONE"] + [
    "income",
    " NEED",
    "",
    "it's",
    "é",
    "\x07",
]


def _outcome(function, value):
    try:
        result = function(value)
    except ValueError as error:
        return {"error": str(error)}
    return {"ok": result}


def functions():
    return {
        "uuid": [
            [value, _outcome(lambda text: str(uuid.UUID(text)), value)]
            for value in UUIDS
        ],
        "category": [
            [value, _outcome(lambda text: LabelCategory(text).value, value)]
            for value in ENUM_VALUES
        ],
        "subcategory": [
            [value, _outcome(lambda text: LabelSubcategory(text).value, value)]
            for value in ENUM_VALUES
        ],
    }


SLICE = Slice(
    "labels",
    (EMPTY, LABELS_DB, INVALID),
    register=register,
    cases=cases,
    functions=functions,
)
