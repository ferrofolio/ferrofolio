# SPDX-License-Identifier: AGPL-3.0-only
"""The settings: the data GET /api/v1/settings reads.

The settings database holds one row per section, each showing another way a stored
section is read: valid, partial, invalid, unreadable or in the shape of an older
version, with timestamps in several forms.
"""

import json

from infrastructure.repository.settings.queries import SettingsQueries

from db_routes.harness import EMPTY, Scenario, Slice

EXPORT = {
    "sheets": {
        "globals": {
            "spreadsheetId": "sheet-global",
            "datetimeFormat": "%d/%m/%Y %H:%M:%S",
            "dateFormat": None,
        },
        "position": [
            {
                "range": "Positions!A1",
                "data": ["accounts", "stocks"],
                "filters": [
                    {"field": "type", "values": "STOCK"},
                    {"field": "entity", "values": ["MyInvestor", "Trade Republic"]},
                ],
                "template": {
                    "id": "template-1",
                    "params": {
                        "z": 1,
                        "a": [1.5, "text", None, True],
                        "ü": {"n": 1e16},
                    },
                },
                "lastUpdate": True,
            }
        ],
        "contributions": [
            {"range": "Contributions!A1", "spreadsheetId": "other", "data": []}
        ],
        "transactions": [],
        "historic": [
            {"range": "Historic!B2", "data": ["historic"], "datetimeFormat": "%Y"}
        ],
    }
}

# key, stored value (a section as Python writes it, or the raw text), updated_at.
ROWS = [
    (
        "general",
        json.dumps({"defaultCurrency": "USD", "editMode": "DRAFT", "removed": True}),
        "2026-03-01T10:00:00+01:00",
    ),
    (
        "data",
        json.dumps(
            {
                "autoRefresh": {
                    "mode": "OFF",
                    "max_outdated": "DAY",
                    "entities": [{"id": "entity-1"}, {"id": "entity-2", "name": "x"}],
                }
            }
        ),
        # A local time in the gap of the change to summer time.
        "2026-03-29 02:30:00",
    ),
    # The same instant as the timestamp of general.
    ("export", json.dumps(EXPORT), "2026-03-01T09:00:00Z"),
    (
        "importing",
        json.dumps({"sheets": {"position": "Positions!A1"}, "ai": {"providers": []}}),
        "2026-06-01T12:00:00.5+02:00",
    ),
    # Not JSON; a timestamp SQLite stores as an integer, which is ignored.
    ("assets", '{"crypto": {"stablecoins": ["USDT"]},', 20260101),
    (
        "labeling",
        json.dumps(
            {
                "external": {
                    "enabled": True,
                    "provider": " openai ",
                    "model": "gpt-5",
                    "upstreamProvider": None,
                    "minConfidence": "80",
                    "examples": {"origins": ["RULE"]},
                }
            }
        ),
        "not a timestamp",
    ),
]


async def fill(client):
    async with client.tx() as cursor:
        for key, value, updated_at in ROWS:
            await cursor.execute(SettingsQueries.UPSERT, (key, value, updated_at))


SETTINGS = Scenario("settings", fill, {"settings": ()})


SLICE = Slice("settings", (EMPTY, SETTINGS))
