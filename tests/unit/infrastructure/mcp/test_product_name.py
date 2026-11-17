# SPDX-License-Identifier: AGPL-3.0-only
import re
from pathlib import Path

import pytest

from infrastructure.controller.routes import forward_integration_oauth_callback
from infrastructure.mcp.errors import LOCKED_MESSAGE
from infrastructure.mcp.instructions import READ_ONLY_NOTE, build_instructions

ROOT = Path(__file__).resolve().parents[4]
UPSTREAM_NAME = re.compile(r"\bFinanze\b")


@pytest.mark.parametrize("read_only", [False, True])
def test_mcp_instructions_name_ferrofolio(read_only):
    text = build_instructions(read_only=read_only)

    assert text.startswith("Ferrofolio is the user's self-hosted")
    assert not UPSTREAM_NAME.search(text)


def test_mcp_messages_name_ferrofolio():
    assert "Ferrofolio" in READ_ONLY_NOTE
    assert "Ferrofolio" in LOCKED_MESSAGE


def test_callback_pages_name_ferrofolio():
    page = forward_integration_oauth_callback.EXPIRED_PAGE

    assert "<title>Ferrofolio</title>" in page
    assert not UPSTREAM_NAME.search(page)


@pytest.mark.parametrize(
    "relative",
    [
        "finanze/infrastructure/mcp",
        "finanze/infrastructure/controller/routes/oauth_callback.py",
    ],
)
def test_user_facing_backend_text_does_not_name_upstream(relative):
    path = ROOT / relative
    files = [path] if path.is_file() else sorted(path.rglob("*.py"))

    offending = [
        f"{file.relative_to(ROOT)}:{number}"
        for file in files
        for number, line in enumerate(file.read_text().splitlines(), start=1)
        if UPSTREAM_NAME.search(line)
    ]

    assert offending == []
