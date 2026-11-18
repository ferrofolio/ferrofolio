# SPDX-License-Identifier: AGPL-3.0-only
"""The routes served by Rust are still declared by the Python backend.

The Rust server forwards every route it does not serve to Python and keeps the
Python implementation of a ported route as the reference for its parity tests
(docs/rust-backend.md). crates/server/ported_routes.json lists the ported routes;
a Rust test checks it against the routes Rust mounts.
"""

import inspect
import json
from pathlib import Path
from unittest.mock import MagicMock

import pytest
from quart import Quart

from infrastructure.controller.controllers import register_routes

PORTED_ROUTES = (
    Path(__file__).resolve().parents[4] / "crates" / "server" / "ported_routes.json"
)


async def _declared_routes():
    app = Quart(__name__)
    parameters = list(inspect.signature(register_routes).parameters)[1:]
    await register_routes(app, **{name: MagicMock() for name in parameters})
    return {
        (method, rule.rule)
        for rule in app.url_map.iter_rules()
        for method in rule.methods
    }


@pytest.mark.asyncio
async def test_python_declares_every_ported_route():
    ported = json.loads(PORTED_ROUTES.read_text(encoding="utf-8"))
    assert ported, "ported_routes.json lists no routes"
    declared = await _declared_routes()
    missing = [
        route for route in ported if (route["method"], route["path"]) not in declared
    ]
    assert missing == []
