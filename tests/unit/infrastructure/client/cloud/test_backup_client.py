# SPDX-License-Identifier: AGPL-3.0-only
from types import SimpleNamespace
from unittest.mock import AsyncMock, MagicMock

import pytest

from domain.backup import BackupFileType
from domain.exception.exceptions import BackupTransferFailed
from infrastructure.client.cloud.backup.backup_client import BackupClient

AUTH = SimpleNamespace(token=SimpleNamespace(token_type="Bearer", access_token="t"))


def _client() -> BackupClient:
    client = BackupClient(MagicMock())
    client._session = MagicMock()
    client._session.get = AsyncMock()
    client._session.post = AsyncMock()
    return client


@pytest.mark.asyncio
async def test_download_fails_without_a_configured_service(monkeypatch):
    monkeypatch.setattr(BackupClient, "BASE_URL", None)
    client = _client()
    request = SimpleNamespace(types=[BackupFileType.DATA], auth=AUTH)

    with pytest.raises(BackupTransferFailed):
        await client.download(request)

    client._session.get.assert_not_called()


@pytest.mark.asyncio
async def test_info_fails_without_a_configured_service(monkeypatch):
    monkeypatch.setattr(BackupClient, "BASE_URL", None)
    client = _client()

    with pytest.raises(BackupTransferFailed):
        await client.get_info(SimpleNamespace(auth=AUTH))

    client._session.get.assert_not_called()


def test_configured_service_is_used(monkeypatch):
    monkeypatch.setattr(BackupClient, "BASE_URL", "https://backups.example.com")

    assert _client()._url("/v1/backups") == "https://backups.example.com/v1/backups"


@pytest.mark.asyncio
async def test_plain_http_service_is_rejected_before_sending_the_token(monkeypatch):
    monkeypatch.setattr(BackupClient, "BASE_URL", "http://backups.example.com")
    client = _client()

    with pytest.raises(BackupTransferFailed):
        await client.get_info(SimpleNamespace(auth=AUTH))

    client._session.get.assert_not_called()
