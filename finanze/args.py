import argparse
import logging
import os
import shlex
import subprocess
from datetime import timedelta
from pathlib import Path
from typing import Optional

from data_dir import DataDirMigration, default_data_dir, migrate_legacy_data_dir
from logs import configure_logging


def app_args() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(description="Ferrofolio Server")

    parser.add_argument(
        "--data-dir",
        help="Directory to store database and configuration files (default: <default-app-data-dir>).",
        type=str,
        default=None,
    )
    parser.add_argument(
        "--host",
        help="Host interface on which the API server will listen (default: 127.0.0.1).",
        type=str,
        default=os.environ.get("FINANZE_HOST", "127.0.0.1"),
    )
    parser.add_argument(
        "--port",
        help="Port on which the API server will run.",
        type=int,
        default=7592,
    )
    parser.add_argument(
        "--log-level",
        help="Set the console logging level (use NONE to disable console logging).",
        choices=["NONE", "DEBUG", "INFO", "WARNING", "ERROR", "CRITICAL"],
        default="INFO",
    )
    parser.add_argument(
        "--log-dir",
        help="Path to log directory (default: <default-app-data-dir>/logs).",
        type=str,
        default=None,
    )
    parser.add_argument(
        "--log-file-level",
        help="Set the file logging level (default: same as --log-level; if console is NONE then INFO; use NONE to disable file logging).",
        choices=["NONE", "DEBUG", "INFO", "WARNING", "ERROR", "CRITICAL"],
        default=None,
    )
    parser.add_argument(
        "--third-party-log-level",
        help="Logging level for third-party libraries (waitress, urllib3, requests...). Use NONE to disable.",
        choices=["NONE", "DEBUG", "INFO", "WARNING", "ERROR", "CRITICAL"],
        default="ERROR",
    )
    parser.add_argument(
        "--llama-server-path",
        help="Path to the llama.cpp server executable used to run local AI models.",
        type=str,
        default=os.environ.get("FINANZE_LLAMA_SERVER_PATH"),
    )
    return parser


def parse_args() -> argparse.Namespace:
    parser = app_args()
    args = parser.parse_args()

    migration = None
    if args.data_dir is None:
        migration = migrate_legacy_data_dir(default_data_dir())
        args.data_dir = migration.data_dir
    else:
        args.data_dir = Path(args.data_dir)
    if args.log_dir is None:
        # Logs follow the default data directory, also while it is the legacy one
        base = migration.data_dir if migration is not None else default_data_dir()
        args.log_dir = base / "logs"
    os.makedirs(args.data_dir, exist_ok=True)
    args.llama_server_path = (
        Path(args.llama_server_path) if args.llama_server_path else None
    )

    args.logged_username = os.environ.get("USERNAME")
    args.logged_password = os.environ.get("PASSWORD")
    args.api_session = os.environ.get(
        "FINANZE_API_SESSION", "true"
    ).strip().lower() not in ("0", "false", "no", "off")

    configure_logging(args)
    if migration is not None:
        _log_migration(migration)
    logging.info(f"Using data directory: {args.data_dir}")

    args.api_session_remember_ttl = _remember_ttl(
        os.environ.get("FINANZE_API_SESSION_REMEMBER_DAYS")
    )

    return args


def _log_migration(migration: DataDirMigration) -> None:
    if migration.moved_from is not None:
        logging.info(
            f"Moved data of a previous installation from {migration.moved_from}"
        )
    elif migration.conflict is not None:
        logging.warning(
            f"Found data of a previous installation in {migration.conflict}, but "
            f"{migration.data_dir} is not empty. Leaving both untouched; pass "
            f"--data-dir {_shell_quote(migration.conflict)} to keep using the previous data."
        )
    elif migration.error is not None:
        logging.error(
            f"Could not move the data of a previous installation "
            f"({migration.error}); using {migration.data_dir} for now."
        )


def _shell_quote(path: Path) -> str:
    """Quote a path so it can be pasted as a single command line argument."""
    if os.name == "nt":
        return subprocess.list2cmdline([str(path)])
    return shlex.quote(str(path))


def _remember_ttl(raw: Optional[str]) -> Optional[timedelta]:
    if not raw or not raw.strip():
        return None
    try:
        days = int(raw.strip())
    except ValueError:
        days = 0
    if days <= 0:
        logging.warning(
            f"Ignoring invalid FINANZE_API_SESSION_REMEMBER_DAYS '{raw}', remembered sessions won't expire"
        )
        return None
    return timedelta(days=days)
