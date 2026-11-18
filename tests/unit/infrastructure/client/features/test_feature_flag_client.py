# SPDX-License-Identifier: AGPL-3.0-only
from unittest.mock import AsyncMock, MagicMock

import pytest

from infrastructure.client.features.feature_flag_client import FeatureFlagClient


@pytest.mark.asyncio
async def test_makes_no_request_without_a_configured_url(monkeypatch):
    monkeypatch.delenv("FEATURE_FLAG_URL", raising=False)
    client = FeatureFlagClient(operative_system=None)
    client._session = MagicMock()
    client._session.get = AsyncMock()

    await client.load()

    client._session.get.assert_not_called()
    assert client.get_all() == {}


@pytest.mark.asyncio
async def test_loads_flags_from_a_configured_url(monkeypatch):
    monkeypatch.setenv("FEATURE_FLAG_URL", "https://flags.example.com")
    client = FeatureFlagClient(operative_system=None)
    response = MagicMock()
    response.json = AsyncMock(
        return_value={"features": {"CLOUD": {"ON": {"target": [{"type": "ALL"}]}}}}
    )
    client._session = MagicMock()
    client._session.get = AsyncMock(return_value=response)

    await client.load()

    client._session.get.assert_called_once_with("https://flags.example.com", timeout=4)
    assert client.get_value("CLOUD") == "ON"
