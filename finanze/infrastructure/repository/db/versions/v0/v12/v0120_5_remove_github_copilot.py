import json

from domain.data_init import DatasourceInitContext
from infrastructure.repository.db.client import DBCursor
from infrastructure.repository.db.upgrader import DBVersionMigration

PROVIDER = "GITHUB_COPILOT"
SETTINGS_KEYS = ("labeling", "ai", "importing")


def _strip_provider(node) -> tuple[object, bool]:
    """Drop provider entries of the removed integration from a settings value."""
    if isinstance(node, dict):
        changed = False
        stripped = {}
        for key, value in node.items():
            value, value_changed = _strip_provider(value)
            stripped[key] = value
            changed = changed or value_changed
        return stripped, changed

    if isinstance(node, list):
        changed = False
        stripped = []
        for item in node:
            if isinstance(item, dict) and item.get("provider") == PROVIDER:
                changed = True
                continue
            item, item_changed = _strip_provider(item)
            stripped.append(item)
            changed = changed or item_changed
        return stripped, changed

    return node, False


class V01205RemoveGitHubCopilot(DBVersionMigration):
    @property
    def name(self):
        return "v0.12.0:5_remove_github_copilot"

    async def upgrade(self, cursor: DBCursor, context: DatasourceInitContext):
        await cursor.execute(
            "DELETE FROM external_integrations WHERE id = ?", (PROVIDER,)
        )

        await cursor.execute(
            'SELECT "key", value FROM settings WHERE "key" IN (?, ?, ?)',
            SETTINGS_KEYS,
        )
        rows = await cursor.fetchall()
        for key, raw in rows:
            try:
                value = json.loads(raw)
            except (TypeError, ValueError):
                continue
            stripped, changed = _strip_provider(value)
            if not changed:
                continue
            await cursor.execute(
                'UPDATE settings SET value = ? WHERE "key" = ?',
                (json.dumps(stripped), key),
            )
