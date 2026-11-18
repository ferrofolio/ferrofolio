# SPDX-License-Identifier: AGPL-3.0-only
"""The public base URL built from a request."""

import pytest
from quart import Quart

from infrastructure.controller.public_url import public_base_url

app = Quart(__name__)


async def _base_url(headers, scheme="http"):
    async with app.test_request_context("/", headers=headers, scheme=scheme):
        return public_base_url()


@pytest.mark.asyncio
async def test_uses_forwarded_host_and_proto(monkeypatch):
    monkeypatch.delenv("FINANZE_PUBLIC_URL", raising=False)
    url = await _base_url(
        {
            "Host": "internal:7592",
            "X-Forwarded-Host": "finance.example.com",
            "X-Forwarded-Proto": "https",
        }
    )
    assert url == "https://finance.example.com"


@pytest.mark.asyncio
async def test_keeps_https_of_a_direct_request(monkeypatch):
    monkeypatch.delenv("FINANZE_PUBLIC_URL", raising=False)
    url = await _base_url({"Host": "finance.example.com"}, scheme="https")
    assert url == "https://finance.example.com"


@pytest.mark.asyncio
async def test_rejects_other_schemes_from_the_request(monkeypatch):
    monkeypatch.delenv("FINANZE_PUBLIC_URL", raising=False)
    url = await _base_url(
        {"Host": "finance.example.com", "X-Forwarded-Proto": "javascript"},
        scheme="javascript",
    )
    assert url == "http://finance.example.com"


@pytest.mark.asyncio
async def test_rejects_other_schemes_of_a_direct_request(monkeypatch):
    monkeypatch.delenv("FINANZE_PUBLIC_URL", raising=False)
    url = await _base_url({"Host": "finance.example.com"}, scheme="javascript")
    assert url == "http://finance.example.com"


@pytest.mark.asyncio
async def test_falls_back_to_the_direct_scheme_for_an_invalid_forwarded_one(
    monkeypatch,
):
    monkeypatch.delenv("FINANZE_PUBLIC_URL", raising=False)
    url = await _base_url(
        {"Host": "finance.example.com", "X-Forwarded-Proto": "javascript"},
        scheme="https",
    )
    assert url == "https://finance.example.com"
