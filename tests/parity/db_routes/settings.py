# SPDX-License-Identifier: AGPL-3.0-only
"""The settings: GET /api/v1/settings.

The settings database holds one row per section, each showing another way a stored
section is read: valid, partial, invalid, unreadable or in the shape of an older
version, with timestamps in several forms. The route cases select sections with
`key`, so that each selection shows how its rows decide the answer and its
lastUpdate. `functions` records the section mapper and the timestamp parsing for
many more stored values than one row per section allows.
"""

import hashlib
import json
from dataclasses import asdict

from infrastructure.config import base_config
from infrastructure.config.file_legacy_config_source import FileLegacyConfigSource
from infrastructure.controller.routes.get_settings import get_settings
from infrastructure.repository.settings.queries import SettingsQueries
from infrastructure.repository.settings.settings_mapper import (
    _json_safe,
    _resolve_sections,
    deserialize_section,
    parse_section,
    parse_timestamp,
)
from infrastructure.repository.settings.settings_repository import SettingsRepository
from application.use_cases.get_settings import GetSettingsImpl

from db_routes.harness import EMPTY, Scenario, Slice, case

PATH = "/api/v1/settings"

# Python answers with the time it started when no stored section has a timestamp
# (BASE_CONFIG.lastUpdate). The recording fixes it; the Rust test sets the same.
DEFAULT_LAST_UPDATE = "2026-01-02T03:04:05.678901+01:00"

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


def register(app, client):
    base_config.BASE_CONFIG.lastUpdate = DEFAULT_LAST_UPDATE
    get_settings_uc = GetSettingsImpl(
        SettingsRepository(client, FileLegacyConfigSource())
    )

    @app.route(PATH, methods=["GET"])
    async def settings_route():
        return await get_settings(get_settings_uc)


def cases():
    def get(name, database, query=None, **options):
        return case(name, database, PATH, query=query, **options)

    return [
        get("no stored settings", "empty"),
        get("no stored settings, one section", "empty", "key=assets"),
        get("every section", "settings"),
        get("partial section with an unknown field", "settings", "key=general"),
        get("local time in a gap", "settings", "key=data"),
        get("full export", "settings", "key=export"),
        get("same instant, first row wins", "settings", "key=general&key=export"),
        get("invalid section, timestamp counts", "settings", "key=importing"),
        get("unreadable section, integer timestamp", "settings", "key=assets"),
        get("section of an older version", "settings", "key=labeling"),
        get("no row", "settings", "key=ai"),
        get("no usable timestamp", "settings", "key=assets&key=labeling&key=ai"),
        get("order of keys", "settings", "key=ai&key=data&key=general"),
        get("repeated key", "settings", "key=general&key=general"),
        get("escaped key", "settings", "key=%67eneral"),
        get("empty pairs", "settings", "&&key=general&&"),
        get("other parameter", "settings", "other=1"),
        get("key in upper case", "settings", "KEY=general"),
        get("unknown section", "settings", "key=general&key=bogus"),
        get("first unknown section", "settings", "key=bogus&key=other"),
        get("section in upper case", "settings", "key=GENERAL"),
        get("empty key", "settings", "key="),
        get("key without value", "settings", "key"),
        get("plus is a space", "settings", "key=gen+eral"),
        get("quote in key", "settings", "key=it%27s"),
        get("both quotes in key", "settings", "key=%22it%27s%22"),
        get("control character in key", "settings", "key=a%00b%0A"),
        get("non-ASCII key", "settings", "key=%C3%A9t%C3%A9"),
        get("invalid UTF-8 in key", "settings", "key=%FFx"),
        get("semicolon is no separator", "settings", "key=general;key=data"),
        get("equals sign in value", "settings", "key==general"),
        # Rust passes these on to Python, which answers as recorded.
        get(
            "no session while unlocked", "settings", session="none", compare="forwarded"
        ),
        get(
            "unknown session while unlocked",
            "settings",
            session="unknown",
            compare="forwarded",
        ),
        get(
            "unknown session and unknown section",
            "settings",
            "key=bogus",
            session="unknown",
            compare="forwarded",
        ),
        get("locked", "settings", session="none", unlocked=False, compare="forwarded"),
        get(
            "locked with an ended session",
            "settings",
            session="unknown",
            unlocked=False,
            compare="forwarded",
        ),
        get(
            "locked and unknown section",
            "settings",
            "key=bogus",
            session="none",
            unlocked=False,
            compare="forwarded",
        ),
    ]


# Stored sections the mapper reads: (section, stored text). Python writes JSON
# with json.dumps; older versions, imports and hand edits can leave anything.
SECTIONS = (
    [
        ("general", "{}"),
        ("general", '{"defaultCurrency": "USD"}'),
        (
            "general",
            '{"defaultCurrency": "usd", "defaultCommodityWeightUnit": "TROY_OUNCE", '
            '"editMode": "DRAFT"}',
        ),
        ("general", '{"defaultCommodityWeightUnit": "KILO"}'),
        ("general", '{"defaultCurrency": 978}'),
        ("general", '{"defaultCurrency": true}'),
        ("general", '{"editMode": "draft"}'),
        ("general", '{"editMode": null}'),
        ("general", '{"unknown": [1, {"a": null}], "defaultCurrency": "GBP"}'),
        ("general", '{"defaultCurrency": "A", "defaultCurrency": "B"}'),
        ("general", ' \n{"defaultCurrency": "CHF"}\t'),
        ("general", '{"defaultCurrency": "\\u00e9\\ud83d\\ude00 \\"x\\""}'),
        ("general", '{"defaultCurrency": NaN}'),
        ("general", "null"),
        ("general", "[]"),
        ("general", '"general"'),
        ("general", "5"),
        ("general", "{not json"),
        ("general", ""),
        ("general", '{"defaultCurrency": "USD"} x'),
        ("data", "{}"),
        ("data", '{"autoRefresh": {}}'),
        (
            "data",
            '{"autoRefresh": {"mode": "OFF", "max_outdated": "WEEK", '
            '"entities": [{"id": "a"}, {"id": "b", "name": "x"}]}}',
        ),
        ("data", '{"autoRefresh": {"mode": "off"}}'),
        ("data", '{"autoRefresh": {"max_outdated": "MONTH"}}'),
        ("data", '{"autoRefresh": {"entities": [{"id": 5}]}}'),
        ("data", '{"autoRefresh": {"entities": [{}]}}'),
        ("data", '{"autoRefresh": {"entities": {"id": "a"}}}'),
        ("data", '{"autoRefresh": null}'),
        ("data", '{"autoRefresh": []}'),
        ("export", "{}"),
        ("export", '{"sheets": null}'),
        ("export", '{"sheets": {}}'),
        ("export", json.dumps(EXPORT)),
        ("export", '{"sheets": {"globals": {}}}'),
        ("export", '{"sheets": {"globals": {"spreadsheetId": "s", "dateFormat": 5}}}'),
        ("export", '{"sheets": {"position": [{"data": ["a"]}]}}'),
        ("export", '{"sheets": {"position": "A"}}'),
        ("export", '{"sheets": {"transactions": null}}'),
        ("export", '{"sheets": {"position": [{"range": "A", "data": "accounts"}]}}'),
        (
            "export",
            '{"sheets": {"position": [{"range": "A", '
            '"filters": [{"field": "f", "values": 5}]}]}}',
        ),
        (
            "export",
            '{"sheets": {"position": [{"range": "A", '
            '"filters": [{"field": "f", "values": ["a", 1]}]}]}}',
        ),
        (
            "export",
            '{"sheets": {"historic": [{"range": "A", "filters": [], "lastUpdate": "yes"}]}}',
        ),
        ("export", '{"sheets": {"historic": [{"range": "A", "lastUpdate": 2}]}}'),
        (
            "export",
            '{"sheets": {"position": [{"range": "A", '
            '"template": {"id": "t", "params": []}}]}}',
        ),
        (
            "export",
            '{"sheets": {"position": [{"range": "A", "template": {"id": "t", '
            '"params": {"b": 1, "a": {"d": [1, 2.50, -0.0, 1e300, '
            '12345678901234567890123, "\\u00fc"]}}}}]}}',
        ),
        ("importing", "{}"),
        (
            "importing",
            '{"sheets": {"position": null, "transactions": [{"range": "T", '
            '"data": "transactions", "template": {"id": "x", "params": null}}]}}',
        ),
        ("importing", '{"sheets": {"position": [{"range": "P"}]}}'),
        ("importing", '{"sheets": {"position": [{"range": "P", "data": ["x"]}]}}'),
        (
            "importing",
            '{"ai": {"providers": [{"provider": "anthropic", "model": "claude"}], '
            '"instructions": "Be brief"}}',
        ),
        ("importing", '{"ai": null}'),
        ("importing", '{"ai": {"instructions": 5}}'),
        (
            "importing",
            json.dumps(
                {
                    "ai": {
                        "providers": [
                            {"provider": "p", "model": str(n)} for n in range(6)
                        ]
                    }
                }
            ),
        ),
        ("assets", "{}"),
        ("assets", '{"crypto": {}}'),
        (
            "assets",
            '{"crypto": {"stablecoins": ["USDT", "EURC"], "hideUnknownTokens": "yes"}}',
        ),
        ("assets", '{"crypto": {"hideUnknownTokens": 1.0}}'),
        ("assets", '{"crypto": {"hideUnknownTokens": 0}}'),
        ("assets", '{"crypto": {"hideUnknownTokens": "Off"}}'),
        ("assets", '{"crypto": {"hideUnknownTokens": "TRUE"}}'),
        ("assets", '{"crypto": {"hideUnknownTokens": 2}}'),
        ("assets", '{"crypto": {"hideUnknownTokens": 0.5}}'),
        ("assets", '{"crypto": {"hideUnknownTokens": " true"}}'),
        ("assets", '{"crypto": {"hideUnknownTokens": ""}}'),
        ("assets", '{"crypto": {"hideUnknownTokens": null}}'),
        ("assets", '{"crypto": {"stablecoins": "USDT"}}'),
        ("assets", '{"crypto": {"stablecoins": [1]}}'),
        ("assets", '{"crypto": null}'),
        ("assets", '{"crypto": {}, "accounts": null}'),
        ("assets", '{"crypto": {}, "accounts": {}}'),
    ]
    + [
        (
            "assets",
            json.dumps({"crypto": {}, "accounts": {"exclusions": exclusions}}),
        )
        for exclusions in [
            [{"type": "IBAN", "value": "de89 3704 0044 0532 0130 00"}],
            [{"type": "IBAN", "value": "DE89 3704　0044\x1c0532"}],
            [{"type": "IBAN", "value": "ıbanſ"}],
            [{"type": "IBAN", "value": "ßx"}],
            [{"type": "IBAN", "value": "ＤE89"}],
            [{"type": "IBAN", "value": "ǆ"}],
            [{"type": "IBAN", "value": "a" * 34}],
            [{"type": "IBAN", "value": "a" * 35}],
            [{"type": "IBAN", "value": "ß" * 17}],
            [{"type": "IBAN", "value": "ß" * 18}],
            [{"type": "IBAN", "value": ""}],
            [{"type": "IBAN", "value": " \t"}],
            [{"type": "IBAN", "value": "DE-89"}],
            [{"type": "IBAN", "value": 5}],
            [{"type": "iban", "value": "DE1"}],
            [{"value": "DE1"}],
            [{"type": "IBAN", "value": "DE1"}, {"type": "IBAN", "value": "de 1"}],
            [{"type": "IBAN", "value": "DE1"}, {"type": "IBAN", "value": "DE2"}],
        ]
    ]
    + [
        ("labeling", "{}"),
        ("labeling", '{"external": {}}'),
        ("labeling", '{"external": {"provider": "openai", "model": "gpt"}}'),
        (
            "labeling",
            '{"external": {"provider": "openrouter", "model": "m", "upstreamProvider": " groq "}}',
        ),
        ("labeling", '{"external": {"provider": "openai"}}'),
        ("labeling", '{"external": {"provider": "", "model": "m"}}'),
        ("labeling", '{"external": {"provider": 0, "model": "m"}}'),
        ("labeling", '{"external": {"provider": 5, "model": "m"}}'),
        ("labeling", '{"external": {"provider": [1], "model": {"a": 1}}}'),
        (
            "labeling",
            '{"external": {"provider": "x", "model": "y", '
            '"providers": [{"provider": "a", "model": "b"}]}}',
        ),
        ("labeling", '{"external": "x"}'),
        ("labeling", '{"external": null}'),
        ("labeling", '{"provider": "x", "model": "y"}'),
        (
            "labeling",
            '{"external": {"minConfidence": "75", "maxPerRun": 10.0, "autoRun": "no", '
            '"enabled": "t", "instructions": "x", '
            '"examples": {"enabled": 1, "count": " 5 ", "origins": ["EXTERNAL"]}}}',
        ),
        ("labeling", '{"external": {"minConfidence": 7.5}}'),
        ("labeling", '{"external": {"minConfidence": "1_000"}}'),
        ("labeling", '{"external": {"minConfidence": "5.0"}}'),
        ("labeling", '{"external": {"minConfidence": "5.5"}}'),
        ("labeling", '{"external": {"minConfidence": true}}'),
        ("labeling", '{"external": {"minConfidence": "-3"}}'),
        ("labeling", '{"external": {"minConfidence": "0x10"}}'),
        ("labeling", '{"external": {"minConfidence": 1e20}}'),
        ("labeling", '{"external": {"minConfidence": 9223372036854775807}}'),
        ("labeling", '{"external": {"minConfidence": null}}'),
        ("labeling", '{"external": {"examples": {"origins": ["manual"]}}}'),
        ("labeling", '{"external": {"examples": {"origins": "RULE"}}}'),
        ("labeling", '{"external": {"examples": {"origins": []}}}'),
        (
            "labeling",
            '{"external": {"providers": [{"provider": "a", "model": "m"}, '
            '{"provider": "a", "model": "m", "upstreamProvider": ""}]}}',
        ),
        (
            "labeling",
            '{"external": {"providers": [{"provider": "a", "model": "m", "upstreamProvider": ""}]}}',
        ),
        ("labeling", '{"external": {"providers": [{"provider": " ", "model": "m"}]}}'),
        ("ai", "{}"),
        ("ai", '{"providers": []}'),
        (
            "ai",
            '{"providers": [{"provider": " anthropic ", "model": " claude ", '
            '"upstreamProvider": null}]}',
        ),
        (
            "ai",
            json.dumps(
                {"providers": [{"provider": "p", "model": str(n)} for n in range(5)]}
            ),
        ),
        (
            "ai",
            json.dumps(
                {"providers": [{"provider": "p", "model": str(n)} for n in range(6)]}
            ),
        ),
        (
            "ai",
            '{"providers": [{"provider": "a", "model": "m"}, {"provider": "a", "model": "m"}]}',
        ),
        (
            "ai",
            '{"providers": [{"provider": " a", "model": "m"}, {"provider": "a ", "model": "m"}]}',
        ),
        (
            "ai",
            '{"providers": [{"provider": "a", "model": "m", "upstreamProvider": "x"}, '
            '{"provider": "a", "model": "m", "upstreamProvider": "y"}]}',
        ),
        (
            "ai",
            '{"providers": [{"provider": "a", "model": "m", "upstreamProvider": 5}]}',
        ),
        ("ai", '{"providers": [{"provider": "a"}]}'),
        ("ai", '{"providers": null}'),
        ("ai", json.dumps({"providers": [{"provider": "　a\x1c\x85", "model": "m"}]})),
        ("ai", json.dumps({"providers": [{"provider": "a" * 200, "model": "m"}]})),
        ("ai", json.dumps({"providers": [{"provider": "a" * 201, "model": "m"}]})),
        (
            "ai",
            json.dumps({"providers": [{"provider": "\U0001f600" * 200, "model": "m"}]}),
        ),
    ]
)

# Stored values the parsing of Python's json module and serde_json read
# differently; Rust deliberately reads them as unreadable, see
# docs/rust-backend.md.
SECTION_DEVIATIONS = {
    ("general", '{"defaultCurrency": "USD", "unknown": NaN}'): (
        "Python's json module reads NaN, which is not JSON, in a field the section "
        "ignores; serde_json cannot read the section, which keeps its defaults"
    ),
    ("general", '{"defaultCurrency": "\\ud800"}'): (
        "Python keeps a lone surrogate escape in a string; serde_json cannot read "
        "the section, which keeps its defaults"
    ),
    ("labeling", '{"external": {"maxPerRun": 123456789012345678901234567890}}'): (
        "Python keeps integers beyond 64 bits; Rust cannot read the section, which "
        "keeps its defaults"
    ),
}

TIMESTAMPS = [
    "2026-03-01T10:00:00+01:00",
    "2026-03-01T09:00:00Z",
    "2026-03-01 10:00:00",
    "  2026-03-01T10:00:00+01:00\n",
    "　2026-03-01T10:00:00\x1c",
    "2026-03-01",
    "20260301",
    "20260301T1000",
    "2026-03-01T10",
    "2026-03-01T1000",
    "2026-03-01T100000",
    "2026-03-01T10:00:00.5",
    "2026-03-01T10:00:00,25+05:30",
    "2026-03-01T10:00:00.123456789-03:00",
    "2026-03-01T10:00:00.1234567890123",
    "2026-03-01T10:00:00.000001",
    "2026-03-01T10:00:00.000000+00:00",
    "2026-03-01T10:00:00+0530",
    "2026-03-01T10:00:00+05",
    "2026-03-01T10:00:00+05:30:15",
    "2026-03-01T10:00:00+05:30:15.5",
    "2026-03-01T10:00:00+053015.25",
    "2026-03-01T10:00:00-00:00",
    "2026-03-01T10:00:00+00:00:00.000001",
    "2026-03-01T10:00:00-23:59:59.999999",
    "2026-03-01T10:00:00+23:59",
    "2026-03-01T10:00:00+24:00",
    "2026-03-01T10:00:00+1",
    "2026-03-01T10:00:00+123",
    "2026-03-01T10:00:00+12:3",
    "2026-03-01T10:00:00-",
    "2026-03-01T10:00:00+01:00Z",
    "2026-03-01T10:00:00Z0",
    "2026-03-01T10:00Z+01:00",
    "2026-03-01T10:00z",
    "2026-03-01T10:00:00 +01:00",
    "2026-03-01T10:00:00x+01:00",
    "2026-03-01T10:00x+01:00",
    "2026-03-01T1000005",
    "2026-03-01T10:00:00+05:99",
    "2026-03-00T24:00",
    "2026-W09-7T10:00",
    "2026-W09",
    "2026W097",
    "2026W09",
    "2026W0971000",
    "2026-W09-7",
    "2026-W097",
    "2026W09-7",
    "2026-W53-1",
    "2025-W53-1",
    "2024-W53",
    "2026-W00",
    "2026-W09-8",
    "2026-060",
    "2026-03-01X10:00",
    "2026-03-01é10:00",
    "2026-03-01\U0001f60010:00",
    "2026-03-01t10:00",
    "2026-03-0110:00",
    "2026-03-01T24:00",
    "2026-12-31T24:00:00",
    "2026-02-28T24:00",
    "2026-02-29T24:00",
    "2026-13-01T24:00",
    "2026-03-01T24:00:01",
    "2026-03-01T23:59:60",
    "2026-03-01T25:00",
    "2026-02-30",
    "2026-00-01",
    "0000-01-01",
    "2026-03-29T02:30:00",
    "2026-03-29T03:30:00",
    "2026-10-25T02:30:00",
    "2026-10-25T01:59:59.999999",
    "2026-10-25T03:00:00",
    "0001-01-01T00:00:00",
    "0001-01-01T00:59:59",
    "0001-01-01T01:00:00",
    "0001-01-01T00:00:00+01:00",
    "9999-12-31T22:59:59",
    "9999-12-31T23:00:00",
    "9999-12-31T23:59:59-23:59",
    "9999-12-31T24:00",
    "1970-01-01T00:00:00",
    "2026-3-1",
    "2026-03-01T10:0",
    "2026-03-01T1:00",
    "2026-03-01T10:00:00.",
    "2026-03-01T10:00:00.5a",
    "2026-03-01T10:00:",
    "2026-03-01T10:0000",
    "2026-03-01T1000:00",
    "2026-03-01T",
    "2026-03-01T1",
    "２０２６-03-01",
    "2026-03-01T10:00:00²",
    "2026-0301",
    "202603-01",
    "",
    "   ",
    "garbage",
    "2026-03",
    "2026-W1",
    20260301,
    1.5,
    None,
]

# Several stored timestamps at once: the latest wins, the first of equal instants.
LAST_UPDATES = [
    [],
    ["garbage"],
    [20260101, None, "x"],
    ["2026-03-01T10:00:00+01:00", "2026-03-01T09:00:00Z"],
    ["2026-03-01T09:00:00Z", "2026-03-01T10:00:00+01:00"],
    ["2026-03-01T10:00:00", "2026-03-01T09:00:00+00:00"],
    ["2026-03-01T10:00:00+01:00", "2026-03-01T10:00:00+00:00", "garbage"],
    ["9999-12-31T23:59:59-23:59", "0001-01-01T00:00:00+23:59"],
    ["2026-03-01T10:00:00.000001+00:00", "2026-03-01T10:00:00+00:00"],
]


def _section(key, text):
    """What the mapper reads from a stored section, or `None` for its defaults."""
    try:
        raw = deserialize_section(text)
    except Exception:
        raw = None
    parsed = None
    if raw is not None:
        try:
            parsed = parse_section(key, raw)
        except Exception:
            parsed = None
    resolved = _resolve_sections({} if raw is None else {key: raw}, [key])[key]
    return {
        "valid": parsed is not None,
        "value": _json_safe(asdict(resolved)),
    }


def _timestamp(value):
    try:
        parsed = parse_timestamp(value)
    except Exception as e:
        return {"stored": value, "error": str(e)}
    return {"stored": value, "parsed": None if parsed is None else parsed.isoformat()}


def _last_update(values):
    repository = SettingsRepository(None, FileLegacyConfigSource())
    rows = {f"row{index}": ("{}", value) for index, value in enumerate(values)}
    try:
        return {"stored": values, "last_update": repository._last_update_value(rows)}
    except Exception as e:
        return {"stored": values, "error": str(e)}


def functions():
    base_config.BASE_CONFIG.lastUpdate = DEFAULT_LAST_UPDATE
    sections = [
        {"section": key, "stored": text, **_section(key, text)}
        for key, text in SECTIONS
    ]
    for (key, text), reason in SECTION_DEVIATIONS.items():
        python = _section(key, text)
        digest = hashlib.sha256(
            json.dumps(python, sort_keys=True).encode("utf-8")
        ).hexdigest()
        defaults = _section(key, "null")
        sections.append(
            {
                "section": key,
                "stored": text,
                "python_sha256": digest,
                "deviation": {"reason": reason, **defaults},
            }
        )
    return {
        "sections": sections,
        "timestamps": [_timestamp(value) for value in TIMESTAMPS],
        "last_updates": [_last_update(values) for values in LAST_UPDATES],
    }


SLICE = Slice(
    "settings",
    (EMPTY, SETTINGS),
    register=register,
    cases=cases,
    context={"default_last_update": DEFAULT_LAST_UPDATE},
    functions=functions,
)
