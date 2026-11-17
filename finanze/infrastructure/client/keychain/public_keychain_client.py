import logging
import os
from datetime import datetime

from typing import Optional

from aiocache import cached, Cache
from dateutil.tz import tzlocal

from application.ports.public_keychain_fetcher_port import PublicKeychainFetcherPort
from domain.public_keychain import PublicKeyEntry
from infrastructure.client.http.http_session import get_http_session

KEYCHAIN_URL_ENV = "PUBLIC_KEYCHAIN_URL"
CACHE_TTL = 21600


class PublicKeychainClient(PublicKeychainFetcherPort):
    def __init__(self, url: Optional[str] = None):
        self._log = logging.getLogger(__name__)
        self._session = None
        # Opt-in; without a URL the keys already stored in the database are used
        self._url = url if url is not None else os.getenv(KEYCHAIN_URL_ENV) or None

    def _get_session(self):
        if not self._session:
            self._session = get_http_session()
        return self._session

    @cached(cache=Cache.MEMORY, ttl=CACHE_TTL)
    async def fetch(self) -> list[PublicKeyEntry]:
        if not self._url:
            return []
        try:
            response = await self._get_session().get(self._url, timeout=2)
            data = await response.json()
        except Exception as e:
            self._log.error(
                "Failed to fetch public keychain (%s): %r", type(e).__name__, e
            )
            return []

        version = data.get("version", 0)
        algo = data.get("algo", 0)
        entries_map = data.get("entries", {})
        now = datetime.now(tzlocal())

        return [
            PublicKeyEntry(key=k, value=v, algo=algo, version=version, updated_at=now)
            for k, v in entries_map.items()
        ]
