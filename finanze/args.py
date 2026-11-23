import argparse
import logging
import os
from datetime import timedelta
from pathlib import Path
from typing import Optional

from data_dir import default_data_dir
from logs import configure_logging


def app_args() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(description="Ferrofolio Server")

    parser.add_argument(
        "--data-dir",
        help="Directory to store database and configuration files (default: "
        "Ferrofolio in the platform's user data directory, into which an existing "
        "Finanze directory there is moved once).",
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
        "--public-port",
        help="Port under which clients reach the server, when it runs behind the "
        "Ferrofolio gateway on another port (default: --port). Used for local "
        "redirect URLs.",
        type=_tcp_port,
        default=None,
    )
    parser.add_argument(
        "--socket-fd",
        help="File descriptor of a listening socket inherited from the Ferrofolio "
        "gateway. The server accepts connections on it instead of binding "
        "--host and --port itself.",
        type=_descriptor,
        default=None,
    )
    parser.add_argument(
        "--log-level",
        help="Set the console logging level (use NONE to disable console logging).",
        choices=["NONE", "DEBUG", "INFO", "WARNING", "ERROR", "CRITICAL"],
        default="INFO",
    )
    parser.add_argument(
        "--log-dir",
        help="Path to log directory (default: <data-dir>/logs).",
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

    default_dir = None
    if args.data_dir is None:
        default_dir = default_data_dir()
        args.data_dir = default_dir.path
    args.data_dir = Path(args.data_dir)
    if args.log_dir is None:
        args.log_dir = args.data_dir / "logs"
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
    if default_dir and default_dir.message:
        log = logging.warning if default_dir.warning else logging.info
        log(default_dir.message)
    logging.info(f"Using data directory: {args.data_dir}")

    args.api_session_remember_ttl = _remember_ttl(
        os.environ.get("FINANZE_API_SESSION_REMEMBER_DAYS")
    )

    return args


def _tcp_port(raw: str) -> int:
    port = int(raw)
    if not 1 <= port <= 65535:
        raise argparse.ArgumentTypeError(f"{raw} is not a TCP port (1-65535)")
    return port


def _descriptor(raw: str) -> int:
    descriptor = int(raw)
    if descriptor < 0:
        raise argparse.ArgumentTypeError(f"{raw} is not a file descriptor")
    return descriptor


def public_port(args: argparse.Namespace) -> int:
    """Port clients use to reach the server, for redirect URLs to this instance."""
    return args.public_port or args.port


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
