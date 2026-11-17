# SPDX-License-Identifier: AGPL-3.0-only
import json
from datetime import datetime
from types import SimpleNamespace
from uuid import UUID
from unittest.mock import AsyncMock, MagicMock

import pytest
import pytest_asyncio

from domain.backup import BackupFileType, BackupTransferPiece
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
@pytest.mark.parametrize(
    "base_url", ["http://backups.example.com", "https://[backups.example.com"]
)
async def test_non_https_service_is_rejected_before_sending_the_token(
    monkeypatch, base_url
):
    monkeypatch.setattr(BackupClient, "BASE_URL", base_url)
    client = _client()

    with pytest.raises(BackupTransferFailed):
        await client.get_info(SimpleNamespace(auth=AUTH))

    client._session.get.assert_not_called()


INVALID_JSON = json.JSONDecodeError("Expecting value", "{", 1)
PIECE_INFO = {
    "id": "00000000-0000-0000-0000-000000000001",
    "protocol": 1,
    "date": "2025-01-01T00:00:00",
    "type": "DATA",
    "size": 3,
}


@pytest_asyncio.fixture(autouse=True)
async def _clear_info_cache():
    # get_info caches its result under a fixed key, which would leak between tests
    await BackupClient.get_info.cache.clear()
    yield
    await BackupClient.get_info.cache.clear()


def _configured_client(monkeypatch, payload=None, error=None) -> BackupClient:
    monkeypatch.setattr(BackupClient, "BASE_URL", "https://backups.example.com")
    response = MagicMock()
    response.status = 200
    response.json = AsyncMock(return_value=payload, side_effect=error)
    client = _client()
    client._session.get.return_value = response
    client._session.post.return_value = response
    return client


def _upload_request():
    piece = BackupTransferPiece(
        id=UUID("00000000-0000-0000-0000-000000000001"),
        type=BackupFileType.DATA,
        size=1,
        date=datetime(2025, 1, 1),
        protocol=1,
        payload=b"x",
    )
    return SimpleNamespace(pieces=SimpleNamespace(pieces=[piece]), auth=AUTH)


@pytest.mark.asyncio
@pytest.mark.parametrize(
    "payload, error",
    [(None, INVALID_JSON), ([], None), ({}, None), ({"pieces": []}, None)],
)
async def test_info_rejects_a_malformed_response(monkeypatch, payload, error):
    client = _configured_client(monkeypatch, payload, error)

    with pytest.raises(BackupTransferFailed):
        await client.get_info(SimpleNamespace(auth=AUTH))


@pytest.mark.asyncio
async def test_info_accepts_an_empty_pieces_object(monkeypatch):
    client = _configured_client(monkeypatch, {"pieces": {}})

    info = await client.get_info(SimpleNamespace(auth=AUTH))

    assert info.pieces == {}


@pytest.mark.asyncio
@pytest.mark.parametrize("backup_type", ["DATA", "CONFIG"])
@pytest.mark.parametrize(
    "malformed_piece",
    [["not", "a", "dict"], {**PIECE_INFO, "id": 123}, {"protocol": 1}],
)
async def test_info_rejects_a_malformed_piece(
    monkeypatch, backup_type, malformed_piece
):
    # Reporting a malformed piece as absent would let an upload skip the
    # conflict check and overwrite the remote backup
    pieces = {"DATA": PIECE_INFO, "CONFIG": PIECE_INFO, backup_type: malformed_piece}
    client = _configured_client(monkeypatch, {"pieces": pieces})

    with pytest.raises(BackupTransferFailed):
        await client.get_info(SimpleNamespace(auth=AUTH))


@pytest.mark.asyncio
async def test_info_ignores_an_unknown_backup_type(monkeypatch):
    client = _configured_client(
        monkeypatch, {"pieces": {"DATA": PIECE_INFO, "UNKNOWN": ["not", "a", "dict"]}}
    )

    info = await client.get_info(SimpleNamespace(auth=AUTH))

    assert list(info.pieces) == [BackupFileType.DATA]
    assert info.pieces[BackupFileType.DATA].size == 3


@pytest.mark.asyncio
@pytest.mark.parametrize(
    "payload, error",
    [(None, INVALID_JSON), ([], None), ({}, None), ({"pieces": []}, None)],
)
async def test_download_rejects_a_malformed_response(monkeypatch, payload, error):
    client = _configured_client(monkeypatch, payload, error)
    request = SimpleNamespace(types=[BackupFileType.DATA], auth=AUTH)

    with pytest.raises(BackupTransferFailed):
        await client.download(request)

    client._file_transfer_strategy.download.assert_not_called()


@pytest.mark.asyncio
@pytest.mark.parametrize(
    "payload, error",
    [
        (None, INVALID_JSON),
        ([], None),
        ({}, None),
        ({"uploads": {}}, None),
        ({"uploads": ["DATA"]}, None),
        ({"uploads": [{"url": "https://storage.example.com"}]}, None),
    ],
)
async def test_upload_rejects_a_malformed_response(monkeypatch, payload, error):
    client = _configured_client(monkeypatch, payload, error)

    with pytest.raises(BackupTransferFailed):
        await client.upload(_upload_request())

    client._file_transfer_strategy.upload.assert_not_called()


@pytest.mark.asyncio
@pytest.mark.parametrize(
    "url",
    [
        "http://storage.example.com/piece",
        "ftp://storage.example.com/piece",
        "https://[storage.example.com/piece",
        None,
    ],
)
async def test_upload_refuses_a_non_https_upload_url(monkeypatch, url):
    client = _configured_client(
        monkeypatch, {"uploads": [{"type": "DATA", "url": url, "method": "PUT"}]}
    )

    with pytest.raises(BackupTransferFailed):
        await client.upload(_upload_request())

    client._file_transfer_strategy.upload.assert_not_called()


@pytest.mark.asyncio
async def test_upload_sends_the_piece_to_an_https_upload_url(monkeypatch):
    client = _configured_client(
        monkeypatch,
        {
            "uploads": [
                {
                    "type": "DATA",
                    "url": "https://storage.example.com/piece",
                    "method": "PUT",
                }
            ]
        },
    )
    client._file_transfer_strategy.upload = AsyncMock()

    result = await client.upload(_upload_request())

    assert [p.type for p in result.pieces] == [BackupFileType.DATA]
    client._file_transfer_strategy.upload.assert_awaited_once()
    assert (
        client._file_transfer_strategy.upload.await_args.kwargs["url"]
        == "https://storage.example.com/piece"
    )
