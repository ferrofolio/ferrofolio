# SPDX-License-Identifier: AGPL-3.0-only
"""Generate the parity fixtures for the status route ported to Rust.

The Python backend is the reference: each scenario starts from a new data
directory, with API sessions, and sends its steps, logins, signups, logouts and status requests,
through the real Quart routes, use cases, session store, session guard, user data
manager and database manager. Every status answer is recorded, and so are the
status code of every other step and the profiles.json it leaves behind. The Rust
test crates/server/tests/parity_status.rs replays the scenarios against the Rust
router, with a fake backend that answers the logins, signups and logouts as Python
did and writes the same profiles.json, and requires the same status answers.

Two tables check the parts of the status that depend on the machine: how Python
writes, completes and resolves paths, on a directory tree with symbolic links, and
whether uvicorn trusts the gateway's proxy headers for a FORWARDED_ALLOW_IPS.

    python tests/parity/generate_status_fixtures.py --output FILE
    python tests/parity/generate_status_fixtures.py --check FILE

--check exits with status 1 when FILE does not match what the Python code
produces now.

Values that differ between machines are placeholders: {root} for the temporary
directory, {version} for the version, which must equal the workspace version in
Cargo.toml, and {platform_type} and {platform_version} for the operating system,
which the Rust tests and the Docker job of CI compare with Python's separately.
"""

import argparse
import asyncio
import itertools
import json
import logging
import os
import sys
import tempfile
import tomllib
import uuid
from datetime import datetime, timezone
from pathlib import Path

from ci_python import require_ci_python

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / "finanze"))

from application.use_cases import register_user as register_module  # noqa: E402
from application.use_cases.authorize_api_access import (  # noqa: E402
    AuthorizeApiAccessImpl,
)
from application.use_cases.get_status import GetStatusImpl  # noqa: E402
from application.use_cases.register_user import RegisterUserImpl  # noqa: E402
from application.use_cases.user_login import UserLoginImpl  # noqa: E402
from application.use_cases.user_logout import UserLogoutImpl  # noqa: E402
from domain.user_login import LoginRequest  # noqa: E402
from infrastructure.api_session.in_memory_api_session_store import (  # noqa: E402
    InMemoryApiSessionStore,
)
from infrastructure.config.server_details_adapter import (  # noqa: E402
    ServerDetailsAdapter,
    resolve_version,
)
from infrastructure.controller.api_session_guard import (  # noqa: E402
    register_api_session_guard,
)
from infrastructure.controller.config import quart  # noqa: E402
from infrastructure.controller.routes.get_status import status  # noqa: E402
from infrastructure.controller.routes.logout import logout  # noqa: E402
from infrastructure.controller.routes.register_user import register_user  # noqa: E402
from infrastructure.controller.routes.user_login import user_login  # noqa: E402
from infrastructure.features.env_feature_flag_adapter import (  # noqa: E402
    EnvFeatureFlagAdapter,
)
from infrastructure.repository.db.client import DBClient  # noqa: E402
from infrastructure.repository.db.manager import DBManager  # noqa: E402
from infrastructure.user_files import user_data_manager as user_data_module  # noqa: E402
from infrastructure.user_files.user_data_manager import UserDataManager  # noqa: E402
from uvicorn.middleware.proxy_headers import ProxyHeadersMiddleware  # noqa: E402
from werkzeug.datastructures import Headers  # noqa: E402

# The environment variables the status depends on; the scenarios set their own.
ENVIRONMENT_PREFIXES = ("FF_",)
ENVIRONMENT_NAMES = (
    "FEATURE_FLAG_URL",
    "FINANZE_PUBLIC_URL",
    "FORWARDED_ALLOW_IPS",
    "MULTI_USER",
)
# uvicorn's default for FORWARDED_ALLOW_IPS.
UVICORN_TRUSTED = "127.0.0.1,::1"
# The internal port of the backend, which the status does not report.
BACKEND_PORT = 49152
NOW = datetime(2026, 11, 18, 12, 0, tzinfo=timezone.utc)
PASSWORD = "Secret-pass1"
# Headers that apply to one connection, which the gateway does not forward
# (crates/server/src/proxy.rs).
HOP_BY_HOP = {
    "connection",
    "keep-alive",
    "proxy-authenticate",
    "proxy-authorization",
    "te",
    "trailer",
    "transfer-encoding",
    "upgrade",
}


class FixedDatetime(datetime):
    """The clock of profiles.json, so that the file is the same on every run."""

    @classmethod
    def now(cls, tz=None):
        return NOW


class Config:
    """The settings port: the logins only connect and disconnect it."""

    async def connect(self, user):
        pass

    async def disconnect(self):
        pass

    async def load_legacy(self):
        return None


class Sheets:
    def connect(self, user):
        pass

    def disconnect(self):
        pass


class Cloud:
    async def connect(self, user):
        pass

    async def disconnect(self):
        pass


def cargo_version() -> str:
    with (ROOT / "Cargo.toml").open("rb") as manifest:
        return tomllib.load(manifest)["workspace"]["package"]["version"]


def build_app(static_dir: Path, scenario: dict, root: Path):
    options = scenario["options"]
    data_dir = Path(options["data_dir"].replace("{root}", str(root)))
    data_manager = UserDataManager(data_dir)
    db_manager = DBManager(DBClient())
    store = InMemoryApiSessionStore() if scenario["api_session"] else None
    args = argparse.Namespace(
        data_dir=data_dir,
        port=BACKEND_PORT,
        public_port=options["port"],
        log_level=options["log_level"],
        log_dir=options["log_dir"].replace("{root}", str(root)),
        log_file_level=options["log_file_level"],
        third_party_log_level=options["third_party_log_level"],
    )
    ports = (Config(), Sheets(), Cloud())
    get_status = GetStatusImpl(
        db_manager,
        data_manager,
        ServerDetailsAdapter(args),
        EnvFeatureFlagAdapter(),
        api_session_port=store,
    )
    login_uc = UserLoginImpl(db_manager, data_manager, *ports, api_session_port=store)
    register_uc = RegisterUserImpl(
        db_manager, data_manager, *ports, api_session_port=store
    )
    logout_uc = UserLogoutImpl(db_manager, *ports, api_session_port=store)

    app = quart(static_dir)
    if store is not None:
        register_api_session_guard(app, AuthorizeApiAccessImpl(store, db_manager))
    app.logger.setLevel(logging.CRITICAL)

    @app.route("/api/v1/status", methods=["GET"])
    async def status_route():
        return await status(get_status)

    @app.route("/api/v1/login", methods=["POST"])
    async def login_route():
        return await user_login(login_uc)

    @app.route("/api/v1/signup", methods=["POST"])
    async def signup_route():
        return await register_user(register_uc)

    @app.route("/api/v1/logout", methods=["POST"])
    async def logout_route():
        return await logout(logout_uc)

    return app, data_dir / "profiles.json", login_uc


def forwarded(headers: list) -> list:
    """The headers as the gateway forwards them: without hop-by-hop headers and
    those that Connection names."""
    listed = {
        name.strip().lower()
        for key, value in headers
        if key.lower() == "connection"
        for name in value.split(",")
    }
    return [
        (key, value)
        for key, value in headers
        if key.lower() not in HOP_BY_HOP and key.lower() not in listed
    ]


async def uvicorn_scheme(headers: list) -> str:
    """The scheme uvicorn gives a request from the gateway, with its own proxy
    header middleware and FORWARDED_ALLOW_IPS as uvicorn's Config reads it."""
    captured = {}

    async def app(scope, receive, send):
        captured["scheme"] = scope["scheme"]

    trusted = os.environ.get("FORWARDED_ALLOW_IPS", UVICORN_TRUSTED)
    # Encoded as Quart's test client encodes them; the request reads the bytes as
    # Latin-1, as a server does.
    scope = {
        "type": "http",
        "scheme": "http",
        "client": ("127.0.0.1", 50000),
        "headers": [
            (key.lower().encode("utf-8"), value.encode("utf-8"))
            for key, value in headers
        ],
    }
    await ProxyHeadersMiddleware(app, trusted_hosts=trusted)(scope, None, None)
    return captured["scheme"]


def placeholders(value, root: Path):
    if isinstance(value, dict):
        return {key: placeholders(item, root) for key, item in value.items()}
    if isinstance(value, list):
        return [placeholders(item, root) for item in value]
    if isinstance(value, str):
        return value.replace(str(root), "{root}")
    return value


def normalise_status(body: dict, root: Path) -> dict:
    server = body["server"]
    if server["version"] != cargo_version():
        raise SystemExit(
            f"finanze/version.py says {server['version']}, Cargo.toml "
            f"{cargo_version()}; the status reports one version"
        )
    server["version"] = "{version}"
    server["platform_type"] = "{platform_type}"
    server["platform_version"] = "{platform_version}"
    return placeholders(body, root)


async def run_scenario(scenario: dict) -> dict:
    saved = {
        key: value
        for key, value in os.environ.items()
        if key in ENVIRONMENT_NAMES or key.startswith(ENVIRONMENT_PREFIXES)
    }
    for key in saved:
        del os.environ[key]
    os.environ.update(scenario.get("environment", {}))
    try:
        with tempfile.TemporaryDirectory(prefix="ferrofolio-status-") as scratch:
            root = Path(scratch).resolve()
            make_tree(root)
            app, profiles, _ = build_app(root / "static", scenario, root)
            scenario["profiles"] = profiles.read_text(encoding="utf-8")
            client = app.test_client()
            tokens = {}
            for step in scenario["steps"]:
                if step["request"] == "restart":
                    # A new process on the same data directory, which logs in with
                    # USERNAME and PASSWORD before it serves (`_init_user`).
                    app, profiles, login_uc = build_app(root / "static", scenario, root)
                    credentials = step["startup_login"]
                    await login_uc.execute(
                        LoginRequest(credentials["username"], credentials["password"])
                    )
                    step["profiles"] = profiles.read_text(encoding="utf-8")
                    client = app.test_client()
                    continue
                await run_step(client, step, tokens, profiles, root)
    finally:
        for key in list(os.environ):
            if key in ENVIRONMENT_NAMES or key.startswith(ENVIRONMENT_PREFIXES):
                del os.environ[key]
        os.environ.update(saved)
    return scenario


async def run_step(client, step: dict, tokens: dict, profiles: Path, root: Path):
    session = step.get("session")
    authorization = []
    if session is not None:
        token = tokens.get(session, f"{session}-unknown-token")
        authorization = [("Authorization", f"Bearer {token}")]
    if step["request"] == "status":
        headers = step.setdefault("headers", [["Host", "localhost"]])
        sent = forwarded([tuple(pair) for pair in headers]) + authorization
        response = await client.get(
            "/api/v1/status",
            headers=Headers(sent),
            scheme=await uvicorn_scheme(sent),
        )
        body = await response.get_json()
        step["response"] = {
            "status": response.status_code,
            "body": normalise_status(body, root),
        }
        return
    path = f"/api/v1/{step['request']}"
    response = await client.post(path, json=step.get("body"), headers=authorization)
    step["response"] = {"status": response.status_code}
    if response.status_code == 200:
        tokens[step["token"]] = (await response.get_json())["token"]
    step["profiles"] = profiles.read_text(encoding="utf-8")


def make_tree(root: Path):
    """The directories and links every scenario and the path table start with."""
    (root / "real" / "sub").mkdir(parents=True)
    (root / "link").symlink_to("real")
    (root / "absolute-link").symlink_to(root / "real")
    (root / "chain").symlink_to("link/sub")
    (root / "loop-a").symlink_to("loop-b")
    (root / "loop-b").symlink_to("loop-a")
    (root / "cwd").mkdir()
    (root / "cwd" / "up").symlink_to("../real")


def options(**changes) -> dict:
    defaults = {
        "data_dir": "{root}/data",
        "port": 7592,
        "log_level": "INFO",
        "log_dir": "{root}/data/logs",
        "log_file_level": None,
        "third_party_log_level": "ERROR",
    }
    defaults.update(changes)
    return defaults


def status_step(session=None, headers=None) -> dict:
    step = {"request": "status"}
    if session is not None:
        step["session"] = session
    if headers is not None:
        step["headers"] = headers
    return step


def login(name, password=PASSWORD, token=None, path="login", **fields) -> dict:
    body = {"username": name, **fields}
    if password is not None:
        body["password"] = password
    step = {"request": path, "body": body}
    if token is not None:
        step["token"] = token
    return step


def signup(name, password=PASSWORD, token=None, **fields) -> dict:
    return login(name, password, token, path="signup", **fields)


def logout_step(session=None) -> dict:
    step = {"request": "logout"}
    if session is not None:
        step["session"] = session
    return step


def scenarios() -> list:
    # Without API sessions the gateway leaves the status to Python, which alone
    # knows then whether the data is unlocked (docs/rust-backend.md, "Status"), so
    # every scenario runs with them.
    return [
        {
            "name": "a new data directory with API sessions",
            "api_session": True,
            "options": options(),
            "steps": [status_step(), status_step("made-up")],
        },
        {
            "name": "signup, logins and logouts with API sessions",
            "api_session": True,
            "options": options(
                data_dir="{root}/link/./sub/../data",
                port=8443,
                log_level="DEBUG",
                log_dir="{root}/link//logs/",
                log_file_level="WARNING",
                third_party_log_level="NONE",
            ),
            "steps": [
                signup("anna", token="first"),
                status_step("first"),
                status_step(),
                status_step("made-up"),
                # A second login while the data is unlocked starts another session.
                login("anna", token="second"),
                status_step("second"),
                login("anna", "wrong-pass1"),
                signup("bert"),
                logout_step(),
                status_step("first"),
                logout_step("first"),
                status_step("first"),
                status_step(),
                login("anna", "wrong-pass1"),
                login("nobody"),
                status_step(),
                login("anna", token="third"),
                status_step("third"),
                status_step("first"),
            ],
        },
        {
            "name": "a guest signs up and registers later, with API sessions",
            "api_session": True,
            "options": options(),
            "steps": [
                signup("gast", password=None, guest=True),
                status_step(),
                # The data is locked, so the session guard lets the logout through,
                # and the logout keeps the guest.
                logout_step(),
                status_step(),
                signup("gast", token="member"),
                status_step("member"),
                status_step(),
                logout_step("member"),
                status_step(),
            ],
        },
        {
            "name": "the automatic login at startup, with API sessions",
            "api_session": True,
            "options": options(),
            "steps": [
                signup("anna", token="first"),
                logout_step("first"),
                {
                    "request": "restart",
                    "startup_login": {"username": "anna", "password": PASSWORD},
                },
                # Unlocked, but not for a request without a session of its own.
                status_step(),
                status_step("first"),
                login("anna", token="again"),
                status_step("again"),
            ],
        },
        {
            "name": "feature flags and a configured public URL",
            "api_session": True,
            "options": options(),
            "environment": {
                "FF_TR_V2_AUTH_API": "OFF",
                "FF_EXTRA": "beta",
                "FINANZE_PUBLIC_URL": " https://finance.example.org// ",
            },
            "steps": [
                status_step(headers=[["Host", "localhost"], ["X-Forwarded-Host", "x"]])
            ],
        },
        {
            "name": "the public URL from the request headers",
            "api_session": True,
            "options": options(),
            "steps": [
                status_step(headers=[["Host", "localhost:7592"]]),
                status_step(
                    headers=[
                        ["Host", "localhost"],
                        ["X-Forwarded-Host", " finance.example.org , proxy.internal"],
                        ["X-Forwarded-Proto", " HTTPS , http"],
                    ]
                ),
                status_step(
                    headers=[["Host", "localhost"], ["X-Forwarded-Proto", "ftp"]]
                ),
                # The first header names no scheme; uvicorn takes the last.
                status_step(
                    headers=[
                        ["Host", "localhost"],
                        ["X-Forwarded-Proto", "wss"],
                        ["X-Forwarded-Proto", " https "],
                    ]
                ),
                status_step(
                    headers=[
                        ["Host", "localhost"],
                        ["X-Forwarded-Proto", "https"],
                        ["X-Forwarded-Proto", "ftp"],
                    ]
                ),
                status_step(
                    headers=[
                        ["Host", "localhost"],
                        ["X-Forwarded-Proto", "ftp"],
                        ["X-Forwarded-Proto", "HTTPS"],
                    ]
                ),
                status_step(headers=[["Host", "h"], ["X-Forwarded-Host", " , x"]]),
                status_step(headers=[["Host", ""]]),
                status_step(headers=[["Host", "bücher.example"]]),
                # The gateway does not forward headers that Connection names.
                status_step(
                    headers=[
                        ["Host", "localhost"],
                        ["Connection", "keep-alive, X-Forwarded-Host"],
                        ["X-Forwarded-Host", "finance.example.org"],
                    ]
                ),
            ],
        },
        {
            "name": "proxy headers from an address uvicorn does not trust",
            "api_session": True,
            "options": options(),
            "environment": {"FORWARDED_ALLOW_IPS": "10.0.0.1"},
            "steps": [
                status_step(
                    headers=[
                        ["Host", "localhost"],
                        ["X-Forwarded-Proto", "ftp"],
                        ["X-Forwarded-Proto", "https"],
                    ]
                ),
                status_step(
                    headers=[["Host", "localhost"], ["X-Forwarded-Proto", "https"]]
                ),
            ],
        },
    ]


PATHS = [
    "/",
    "//",
    "{root}/real",
    "{root}//real/./sub/",
    "{root}/link",
    "{root}/link/sub/../..",
    "{root}/absolute-link/sub",
    "{root}/chain/../x",
    "{root}/loop-a/x",
    "{root}/missing/../real",
    "{root}/link/missing/../sub",
    "",
    ".",
    "..",
    "up/sub",
    "./up//sub/",
    "../link/sub",
    "new/dir/../x",
]

FORWARDED_ALLOW_IPS = [
    None,
    "*",
    "* ",
    "",
    "127.0.0.1",
    " 127.0.0.1 ",
    "10.0.0.1, 127.0.0.1",
    "::1",
    "::ffff:127.0.0.1",
    "localhost",
    "10.0.0.1",
    "127.000.000.001",
    "127.0.0.0/8",
    "127.0.0.0/08",
    "127.0.0.0/255.0.0.0",
    "127.0.0.0/0.255.255.255",
    "127.0.0.1/8",
    "127.0.0.0/33",
    "0.0.0.0/0",
    "128.0.0.0/1",
    "0.0.0.0/1",
]


def path_table() -> list:
    """str(Path(p)), str(Path(p).absolute()) and str(Path(p).resolve()), with the
    working directory {root}/cwd."""
    rows = []
    previous = os.getcwd()
    with tempfile.TemporaryDirectory(prefix="ferrofolio-paths-") as scratch:
        root = Path(scratch).resolve()
        make_tree(root)
        os.chdir(root / "cwd")
        try:
            for raw in PATHS:
                path = Path(raw.replace("{root}", str(root)))
                rows.append(
                    placeholders(
                        {
                            "raw": raw,
                            "str": str(path),
                            "absolute": str(path.absolute()),
                            "resolve": str(path.resolve()),
                        },
                        root,
                    )
                )
        finally:
            os.chdir(previous)
    return rows


def trust_table() -> list:
    from uvicorn.middleware.proxy_headers import _TrustedHosts

    return [
        {
            "value": value,
            "trusted": "127.0.0.1"
            in _TrustedHosts(UVICORN_TRUSTED if value is None else value),
        }
        for value in FORWARDED_ALLOW_IPS
    ]


async def generate() -> dict:
    if resolve_version() != cargo_version():
        raise SystemExit(
            f"finanze/version.py says {resolve_version()}, Cargo.toml "
            f"{cargo_version()}; the status reports one version"
        )
    ids = (uuid.UUID(f"00000000-0000-4000-8000-{n:012d}") for n in itertools.count(1))
    register_module.uuid4 = lambda: next(ids)
    user_data_module.datetime = FixedDatetime
    user_data_module.tzlocal = lambda: timezone.utc
    logging.disable(logging.CRITICAL)
    cases = [await run_scenario(scenario) for scenario in scenarios()]
    names = [scenario["name"] for scenario in cases]
    if len(names) != len(set(names)):
        raise SystemExit("scenario names must be unique")
    return {
        "generated_by": "tests/parity/generate_status_fixtures.py",
        "scenarios": cases,
        "paths": path_table(),
        "forwarded_allow_ips": trust_table(),
    }


def render(fixtures: dict) -> str:
    lines = ["{"]
    lines.append(f'  "generated_by": {json.dumps(fixtures["generated_by"])},')
    for key in ("scenarios", "paths", "forwarded_allow_ips"):
        lines.append(f'  "{key}": [')
        rendered = [
            "    " + json.dumps(entry, sort_keys=True, ensure_ascii=False)
            for entry in fixtures[key]
        ]
        lines.append(",\n".join(rendered))
        lines.append("  ]," if key != "forwarded_allow_ips" else "  ]")
    lines.append("}")
    return "\n".join(lines) + "\n"


def main():
    require_ci_python()
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    group = parser.add_mutually_exclusive_group(required=True)
    group.add_argument("--output", type=Path, help="write the fixtures to this file")
    group.add_argument("--check", type=Path, help="compare the fixtures in this file")
    args = parser.parse_args()

    rendered = render(asyncio.run(generate()))
    if args.output:
        args.output.write_text(rendered, encoding="utf-8")
        return 0
    if args.check.read_text(encoding="utf-8") != rendered:
        print(
            f"{args.check} is out of date; regenerate it with --output",
            file=sys.stderr,
        )
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
