# SPDX-License-Identifier: AGPL-3.0-only
"""Creates the fixture databases and records the routes that read them.

A scenario is a database the Python backend creates with every migration of this
version, filled with fixed data through its own repositories or queries. A slice
registers the real Quart routes it records, with the real repositories and use
cases, behind the production session guard, and lists its cases. Every case runs
against the committed file of its scenario, which Rust opens for the same case.

SQLCipher salts every database, so a file differs on each run. check() therefore
creates every scenario anew and requires the same dump from it as from the
committed file, records every case again against the committed files, and
requires the same fixtures.
"""

import hashlib
import json
import logging
import os
import shutil
import sys
import tempfile
import time
import uuid
from collections.abc import Awaitable, Callable
from dataclasses import dataclass, field
from pathlib import Path
from typing import Any, Optional

# Local times in the database become aware with the server's time zone. The
# recording uses a POSIX rule, which Rust reads without a time zone database, with
# daylight saving time, so that gaps and folds are covered. It is set before the
# backend is imported, as the backend reads the time zone once.
TIME_ZONE = "CET-1CEST,M3.5.0,M10.5.0/3"
os.environ["TZ"] = TIME_ZONE
time.tzset()
# Invalid stored data is recorded on purpose; the backend's warnings are noise.
logging.disable(logging.WARNING)

ROOT = Path(__file__).resolve().parents[3]
sys.path.insert(0, str(ROOT / "finanze"))

from application.use_cases.authorize_api_access import (  # noqa: E402
    AuthorizeApiAccessImpl,
)
from domain.data_init import DatasourceInitParams  # noqa: E402
from domain.user import User  # noqa: E402
from infrastructure.api_session.in_memory_api_session_store import (  # noqa: E402
    InMemoryApiSessionStore,
)
from infrastructure.controller.api_session_guard import (  # noqa: E402
    register_api_session_guard,
)
from infrastructure.controller.config import quart  # noqa: E402
from infrastructure.repository.db.client import DBClient  # noqa: E402
from infrastructure.repository.db.manager import DB_NAME, DBManager  # noqa: E402
from infrastructure.repository.db.version_registry import versions  # noqa: E402

GENERATED_BY = "tests/parity/generate_db_route_fixtures.py"
USERNAME = "parity"
# The password of the fixture databases, not a secret. The quote and the non-ASCII
# letter check that both implementations hand it to SQLCipher the same way.
PASSWORD = "parity pass'word ü"
# The token of a session Python does not know, as after a logout.
UNKNOWN_TOKEN = "parity-unknown-session-token"
DATABASES = "databases.json"
SESSIONS = ("valid", "unknown", "none")


@dataclass(frozen=True)
class Scenario:
    """A fixture database. `fill` writes its data; `tables` names the tables it
    writes to, each with the columns whose values differ between runs, such as a
    time the backend takes from the clock. The dump records these tables row by
    row and every other table, which holds what the migrations seed, by its number
    of rows."""

    name: str
    fill: Optional[Callable[[DBClient], Awaitable[None]]] = None
    tables: dict[str, tuple[str, ...]] = field(default_factory=dict)
    # Rewrites a table's rows before the dump compares them, given the open
    # connection, the table name, its columns and its rows: for values that differ
    # between runs only in some rows, such as the ids the migrations generate,
    # which a fixed value then replaces while the scenario's own stay as they are.
    normalize: Optional[Callable[[Any, str, list[str], list[list]], list[list]]] = None


@dataclass(frozen=True)
class Slice:
    """The routes of one slice of the port, recorded into `<name>.json`.

    `register` adds the routes to the app with the real repositories and use
    cases on the given database client. `context` holds values the Rust test
    needs to answer like the recording, `functions` records functions behind the
    routes for inputs that one database row per case could not cover."""

    name: str
    scenarios: tuple[Scenario, ...]
    register: Optional[Callable[[Any, DBClient], None]] = None
    cases: Callable[[], list[dict]] = list
    context: dict = field(default_factory=dict)
    functions: Callable[[], dict] = dict


EMPTY = Scenario("empty")


def case(
    name,
    database,
    path,
    *,
    query=None,
    method="GET",
    session="valid",
    unlocked=True,
    compare="full",
    deviation=None,
):
    """A request against a scenario's database and how Rust's answer is compared.

    session is "valid" (the token of the login), "unknown" (a token no session
    has, as after a logout) or "none" (no Authorization header). unlocked=False
    sends the request while the data is locked, which ends every session. query
    is the raw query string.

    compare and deviation work as in the calculation fixtures, with one more
    mode: "forwarded", for a request Rust passes on to Python, which then gives
    the recorded answer. Rust answers from its own database only with a valid
    session on unlocked data (docs/rust-backend.md, "Routes that read the
    database"); every other case must be "forwarded"."""
    if session not in SESSIONS:
        raise ValueError(f"{name}: unknown session {session!r}")
    if session == "valid" and not unlocked:
        raise ValueError(f"{name}: a session exists only while the data is unlocked")
    if (session != "valid" or not unlocked) != (compare == "forwarded"):
        raise ValueError(
            f"{name}: Rust passes exactly the requests without a valid session or "
            "on locked data on to Python, which compare as forwarded"
        )
    entry = {
        "name": name,
        "database": database,
        "method": method,
        "path": path,
        "compare": compare,
    }
    if query is not None:
        entry["query"] = query
    if session != "valid":
        entry["session"] = session
    if not unlocked:
        entry["unlocked"] = False
    if deviation is not None:
        entry["deviation"] = deviation
    return entry


def open_database(path: Path):
    """A connection to a fixture database, set up as the backend sets up its own."""
    connection = DBManager._base_connect(path)
    DBManager._unlock_and_setup(connection, PASSWORD)
    return connection


async def create(directory: Path, scenario: Scenario) -> Path:
    """Creates the scenario's database in `directory` with every migration and its
    data, and closes it."""
    user = User(id=uuid.uuid4(), username=USERNAME, path=directory, last_login=None)
    client = DBClient()
    manager = DBManager(client)
    await manager.initialize(DatasourceInitParams.build(user, PASSWORD))
    try:
        if scenario.fill is not None:
            await scenario.fill(client)
    finally:
        await manager.lock()
    return directory / DB_NAME


def _value(value):
    if isinstance(value, bytes):
        return {"blob": value.hex()}
    return value


def dump(path: Path, scenario: Scenario) -> dict:
    """The migrations of a database and the contents of its tables (see Scenario)."""
    with tempfile.TemporaryDirectory() as scratch:
        # Opening a database adds WAL files next to it; the fixture stays clean.
        copy = Path(scratch) / DB_NAME
        shutil.copyfile(path, copy)
        connection = open_database(copy)
        try:
            migrations = [
                {"version": version, "name": name}
                for version, name in connection.execute(
                    "SELECT version, name FROM migrations ORDER BY version"
                )
            ]
            names = [
                row[0]
                for row in connection.execute(
                    "SELECT name FROM sqlite_master WHERE type = ? ORDER BY name",
                    ("table",),
                )
                if not row[0].startswith("sqlite_")
            ]
            tables = {}
            for name in names:
                # A table name cannot be a bound parameter; it comes from
                # sqlite_master and is quoted as an identifier.
                identifier = '"' + name.replace('"', '""') + '"'
                if name not in scenario.tables:
                    (count,) = connection.execute(
                        f"SELECT count(*) FROM {identifier}"
                    ).fetchone()
                    tables[name] = {"count": count}
                    continue
                cursor = connection.execute(f"SELECT * FROM {identifier}")
                columns = [description[0] for description in cursor.description]
                volatile = scenario.tables[name]
                rows = [list(row) for row in cursor.fetchall()]
                if scenario.normalize is not None:
                    rows = scenario.normalize(connection, name, columns, rows)
                rows = [
                    [
                        "<volatile>" if column in volatile else _value(value)
                        for column, value in zip(columns, row)
                    ]
                    for row in rows
                ]
                rows.sort(key=lambda row: json.dumps(row, sort_keys=True))
                tables[name] = {"columns": columns, "rows": rows}
        finally:
            connection.close()
    unknown = set(scenario.tables) - set(tables)
    if unknown:
        raise SystemExit(f"{scenario.name}: no tables {sorted(unknown)}")
    return {"migrations": migrations, "tables": tables}


def _dump_or_none(path: Path, scenario: Scenario) -> Optional[dict]:
    """The dump of a committed database, or None when it cannot hold the scenario,
    for example because a migration since has added a table the scenario fills."""
    try:
        return dump(path, scenario)
    except SystemExit:
        return None


class DataState:
    """What the session guard asks the datasource: whether the data is unlocked."""

    unlocked = False


async def _send(http, entry, token):
    headers = {}
    session = entry.get("session", "valid")
    if session == "valid":
        headers["Authorization"] = f"Bearer {token}"
    elif session == "unknown":
        headers["Authorization"] = f"Bearer {UNKNOWN_TOKEN}"
    path = entry["path"]
    if "query" in entry:
        path = f"{path}?{entry['query']}"
    response = await http.open(path, method=entry["method"], headers=headers)
    data = await response.get_data()
    try:
        body = json.loads(data)
    except ValueError:
        body = None
    return {"status": response.status_code, "body": body}, data


async def record(slice_: Slice, directory: Path) -> dict:
    """Sends every case of the slice through its routes against the committed
    databases in `directory` and returns the fixtures."""
    cases = slice_.cases()
    names = [entry["name"] for entry in cases]
    if len(names) != len(set(names)):
        raise SystemExit(f"{slice_.name}: case names must be unique")
    known = {scenario.name for scenario in slice_.scenarios}
    with tempfile.TemporaryDirectory() as scratch:
        static = Path(scratch) / "static"
        static.mkdir()
        client = DBClient()
        state = DataState()
        sessions = InMemoryApiSessionStore()
        token = sessions.create()
        app = quart(static)
        register_api_session_guard(app, AuthorizeApiAccessImpl(sessions, state))
        if slice_.register is not None:
            slice_.register(app, client)
        # Some cases provoke errors on purpose; their tracebacks are noise.
        app.logger.setLevel(logging.CRITICAL)
        http = app.test_client()
        connections = {}
        try:
            for entry in cases:
                database = entry["database"]
                if database not in known:
                    raise SystemExit(f"{entry['name']}: unknown database {database}")
                if database not in connections:
                    copy = Path(scratch) / f"{database}.db"
                    shutil.copyfile(directory / f"{database}.db", copy)
                    connections[database] = open_database(copy)
                state.unlocked = entry.get("unlocked", True)
                client.set_connection(connections[database] if state.unlocked else None)
                entry["response"], raw = await _send(http, entry, token)
                if entry["compare"] == "status_code" and "code" not in (
                    entry["response"]["body"] or {}
                ):
                    raise SystemExit(f"{entry['name']}: Python's answer has no code")
                if entry["compare"] == "deviation":
                    # Rust only has to match the deviation; a digest of Python's
                    # body still makes --check notice when Python's answer changes.
                    body = entry["response"].pop("body")
                    digest_input = (
                        raw
                        if body is None
                        else json.dumps(body, sort_keys=True).encode("utf-8")
                    )
                    entry["response"]["body_sha256"] = hashlib.sha256(
                        digest_input
                    ).hexdigest()
        finally:
            client.set_connection(None)
            for connection in connections.values():
                connection.close()
    return {
        "generated_by": GENERATED_BY,
        "time_zone": TIME_ZONE,
        "context": slice_.context,
        "functions": slice_.functions(),
        "cases": cases,
    }


def render_slice(fixtures: dict) -> str:
    lines = ["{"]
    for key in ("generated_by", "time_zone", "context", "functions"):
        value = json.dumps(fixtures[key], sort_keys=True, ensure_ascii=False)
        lines.append(f'  "{key}": {value},')
    lines.append('  "cases": [')
    lines.append(
        ",\n".join(
            "    " + json.dumps(entry, sort_keys=True, ensure_ascii=False)
            for entry in fixtures["cases"]
        )
    )
    lines.append("  ]")
    lines.append("}")
    return "\n".join(lines) + "\n"


def scenarios(slices: list[Slice]) -> dict[str, Scenario]:
    """Every scenario of the slices by name; slices share a scenario by using the
    same object."""
    found: dict[str, Scenario] = {}
    for slice_ in slices:
        for scenario in slice_.scenarios:
            if found.setdefault(scenario.name, scenario) is not scenario:
                raise SystemExit(f"two scenarios are named {scenario.name}")
    return dict(sorted(found.items()))


def render_databases(directory: Path, all_scenarios: dict[str, Scenario]) -> str:
    databases = {
        name: {"file": f"{name}.db", **dump(directory / f"{name}.db", scenario)}
        for name, scenario in all_scenarios.items()
    }
    fixtures = {"generated_by": GENERATED_BY, "databases": databases}
    return json.dumps(fixtures, indent=1, sort_keys=True, ensure_ascii=False) + "\n"


async def output(slices: list[Slice], directory: Path) -> None:
    """Creates every database and records every slice into `directory`."""
    directory.mkdir(parents=True, exist_ok=True)
    all_scenarios = scenarios(slices)
    for name, scenario in all_scenarios.items():
        committed = directory / f"{name}.db"
        with tempfile.TemporaryDirectory() as scratch:
            created = await create(Path(scratch), scenario)
            # SQLCipher salts every file, so a database that still holds what its
            # scenario defines is kept rather than replaced by an equal one.
            if committed.exists() and _dump_or_none(committed, scenario) == dump(
                created, scenario
            ):
                continue
            shutil.copyfile(created, committed)
    (directory / DATABASES).write_text(
        render_databases(directory, all_scenarios), encoding="utf-8"
    )
    for slice_ in slices:
        if slice_.register is None:
            continue
        (directory / f"{slice_.name}.json").write_text(
            render_slice(await record(slice_, directory)), encoding="utf-8"
        )


async def check(slices: list[Slice], directory: Path) -> list[str]:
    """What differs between the committed fixtures in `directory` and the ones the
    Python backend produces now."""
    problems = []
    all_scenarios = scenarios(slices)
    expected_migrations = [migration.name for migration in versions]
    for name, scenario in all_scenarios.items():
        committed = dump(directory / f"{name}.db", scenario)
        with tempfile.TemporaryDirectory() as scratch:
            fresh = dump(await create(Path(scratch), scenario), scenario)
        if [m["name"] for m in committed["migrations"]] != expected_migrations:
            problems.append(f"{name}.db lacks migrations of this version")
        elif committed != fresh:
            problems.append(f"{name}.db does not hold the data its scenario defines")
    rendered = render_databases(directory, all_scenarios)
    if (directory / DATABASES).read_text(encoding="utf-8") != rendered:
        problems.append(f"{DATABASES} does not match the databases")
    for slice_ in slices:
        if slice_.register is None:
            continue
        path = directory / f"{slice_.name}.json"
        rendered = render_slice(await record(slice_, directory))
        if path.read_text(encoding="utf-8") != rendered:
            problems.append(f"{path.name} does not match what Python answers now")
    return problems
