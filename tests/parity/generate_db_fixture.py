# SPDX-License-Identifier: AGPL-3.0-only
"""Generate the encrypted database the Rust database layer is tested against.

    python tests/parity/generate_db_fixture.py --output crates/infra-db/tests/fixtures
    python tests/parity/generate_db_fixture.py --check crates/infra-db/tests/fixtures

--output creates data.db with the Python backend's DBManager: a new SQLCipher
database with every migration of this version applied, encrypted with PASSWORD, and
records the applied migrations in migrations.json. SQLCipher salts every database,
so the file differs on each run; --check therefore opens it the way the backend
does and exits with status 1 when its migrations differ from migrations.json or
from the migrations of this version. Regenerate the fixture after adding a migration.
"""

import argparse
import asyncio
import json
import shutil
import sys
import tempfile
import uuid
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / "finanze"))

from domain.data_init import DatasourceInitParams  # noqa: E402
from domain.user import User  # noqa: E402
from infrastructure.repository.db.client import DBClient  # noqa: E402
from infrastructure.repository.db.manager import DB_NAME, DBManager  # noqa: E402
from infrastructure.repository.db.version_registry import versions  # noqa: E402

# The password of a test fixture, not a secret. The quote and the non-ASCII letter
# check that both implementations hand the password to SQLCipher the same way.
PASSWORD = "fixture pass'word ü"
MIGRATIONS = "migrations.json"


def read_migrations(database: Path) -> list[dict]:
    """The applied migrations, read with the backend's own connection setup."""
    connection = DBManager._base_connect(database)
    try:
        DBManager._unlock_and_setup(connection, PASSWORD)
        rows = connection.execute(
            "SELECT version, name FROM migrations ORDER BY version"
        ).fetchall()
    finally:
        connection.close()
    return [{"version": version, "name": name} for version, name in rows]


async def create(directory: Path) -> None:
    user = User(id=uuid.uuid4(), username="fixture", path=directory, last_login=None)
    manager = DBManager(DBClient())
    await manager.initialize(DatasourceInitParams.build(user, PASSWORD))
    await manager.lock()


def expected_names() -> list[str]:
    return [migration.name for migration in versions]


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    group = parser.add_mutually_exclusive_group(required=True)
    group.add_argument("--output", type=Path, help="directory to write the fixture to")
    group.add_argument("--check", type=Path, help="directory of the fixture to check")
    args = parser.parse_args()

    if args.output:
        with tempfile.TemporaryDirectory() as scratch:
            asyncio.run(create(Path(scratch)))
            migrations = read_migrations(Path(scratch) / DB_NAME)
            args.output.mkdir(parents=True, exist_ok=True)
            shutil.copyfile(Path(scratch) / DB_NAME, args.output / DB_NAME)
        (args.output / MIGRATIONS).write_text(
            json.dumps(migrations, indent=2) + "\n", encoding="utf-8"
        )
        return 0

    recorded = json.loads((args.check / MIGRATIONS).read_text(encoding="utf-8"))
    with tempfile.TemporaryDirectory() as scratch:
        # Opening the database adds WAL files next to it; keep the fixture clean.
        copy = Path(scratch) / DB_NAME
        shutil.copyfile(args.check / DB_NAME, copy)
        migrations = read_migrations(copy)
    problems = []
    if migrations != recorded:
        problems.append(f"{DB_NAME} does not match {MIGRATIONS}")
    if [migration["name"] for migration in migrations] != expected_names():
        problems.append("the fixture lacks migrations of this version")
    if problems:
        print(
            "; ".join(problems)
            + ". Regenerate with: python tests/parity/generate_db_fixture.py "
            f"--output {args.check}",
            file=sys.stderr,
        )
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
