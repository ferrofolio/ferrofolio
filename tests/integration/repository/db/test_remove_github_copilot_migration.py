import json
import sqlite3

import pytest
import pytest_asyncio

from infrastructure.repository.db.client import DBClient
from infrastructure.repository.db.versions.v0.v12.v0120_5_remove_github_copilot import (
    V01205RemoveGitHubCopilot,
)

UPDATED_AT = "2026-10-01T10:00:00+02:00"
COPILOT_ENTRY = {"provider": "GITHUB_COPILOT", "model": "gpt-5-mini"}
OPENROUTER_ENTRY = {"provider": "OPENROUTER", "model": "openai/gpt-5-mini"}


@pytest_asyncio.fixture
async def db():
    conn = sqlite3.connect(":memory:")
    conn.row_factory = sqlite3.Row
    conn.execute("CREATE TABLE sys_config (key TEXT PRIMARY KEY, value TEXT)")
    conn.execute(
        "CREATE TABLE external_integrations ("
        "id VARCHAR(36) NOT NULL PRIMARY KEY, "
        "name VARCHAR(48) NOT NULL, "
        "type VARCHAR(32) NOT NULL, "
        "status VARCHAR(32) NOT NULL)"
    )
    conn.execute(
        'CREATE TABLE settings ("key" VARCHAR(48) NOT NULL PRIMARY KEY, '
        "value TEXT NOT NULL, updated_at TEXT NOT NULL)"
    )
    conn.execute(
        "INSERT INTO external_integrations (id, name, type, status) VALUES "
        "('GITHUB_COPILOT', 'GitHub Copilot', 'AI_PROVIDER', 'ON'), "
        "('OPENAI', 'OpenAI', 'AI_PROVIDER', 'OFF')"
    )
    conn.commit()
    client = DBClient(conn)
    yield client, conn
    conn.close()


def _insert(conn, key: str, value):
    conn.execute(
        'INSERT INTO settings ("key", value, updated_at) VALUES (?, ?, ?)',
        (key, json.dumps(value), UPDATED_AT),
    )
    conn.commit()


async def _migrate(client):
    async with client.tx(skip_last_update=True) as cursor:
        await V01205RemoveGitHubCopilot().upgrade(cursor, None)


def _settings(conn):
    rows = {}
    for row in conn.execute('SELECT "key", value, updated_at FROM settings'):
        raw = row["value"]
        try:
            value = json.loads(raw)
        except (TypeError, ValueError):
            value = raw
        rows[row["key"]] = (value, row["updated_at"])
    return rows


def _integrations(conn):
    return [row["id"] for row in conn.execute("SELECT id FROM external_integrations")]


class TestRemoveGitHubCopilotMigration:
    @pytest.mark.asyncio
    async def test_removes_the_integration_row_only(self, db):
        client, conn = db

        await _migrate(client)

        assert _integrations(conn) == ["OPENAI"]

    @pytest.mark.asyncio
    async def test_drops_provider_entries_and_keeps_the_rest(self, db):
        client, conn = db
        _insert(
            conn,
            "labeling",
            {
                "external": {
                    "enabled": True,
                    "providers": [COPILOT_ENTRY, OPENROUTER_ENTRY],
                }
            },
        )
        _insert(conn, "ai", {"providers": [COPILOT_ENTRY]})
        _insert(
            conn,
            "importing",
            {"ai": {"providers": [COPILOT_ENTRY, OPENROUTER_ENTRY], "instructions": "x"}},
        )

        await _migrate(client)

        rows = _settings(conn)
        assert rows["labeling"] == (
            {"external": {"enabled": True, "providers": [OPENROUTER_ENTRY]}},
            UPDATED_AT,
        )
        assert rows["ai"] == ({"providers": []}, UPDATED_AT)
        assert rows["importing"] == (
            {"ai": {"providers": [OPENROUTER_ENTRY], "instructions": "x"}},
            UPDATED_AT,
        )

    @pytest.mark.asyncio
    async def test_settings_without_the_provider_stay_untouched(self, db):
        client, conn = db
        labeling = {"external": {"enabled": False, "providers": [OPENROUTER_ENTRY]}}
        general = {"currency": "EUR"}
        _insert(conn, "labeling", labeling)
        _insert(conn, "general", general)

        await _migrate(client)

        assert _settings(conn) == {
            "labeling": (labeling, UPDATED_AT),
            "general": (general, UPDATED_AT),
        }

    @pytest.mark.asyncio
    async def test_broken_settings_value_is_kept(self, db):
        client, conn = db
        conn.execute(
            'INSERT INTO settings ("key", value, updated_at) VALUES (?, ?, ?)',
            ("ai", "not json", UPDATED_AT),
        )
        conn.commit()

        await _migrate(client)

        assert _settings(conn) == {"ai": ("not json", UPDATED_AT)}
