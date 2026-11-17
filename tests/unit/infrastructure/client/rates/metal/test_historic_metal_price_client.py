from datetime import date
from unittest.mock import AsyncMock

import pytest

from domain.commodity import CommodityType, WeightUnit
from domain.dezimal import Dezimal
from domain.exchange_rate import HistoricMetalRates
from infrastructure.client.rates.metal.historic_metal_price_client import (
    HistoricMetalPriceClient,
    HistoricRatesParseError,
)


METAL_RATES_URL = "https://data.example.com/rates/metals"


@pytest.fixture(autouse=True)
def _configured_source(monkeypatch):
    monkeypatch.setattr(HistoricMetalPriceClient, "BASE_URL", METAL_RATES_URL)


def _make_response(payload, ok=True, text=""):
    response = AsyncMock()
    response.ok = ok
    response.json = AsyncMock(return_value=payload)
    response.text = AsyncMock(return_value=text)
    return response


def _build_client(payload, ok=True) -> HistoricMetalPriceClient:
    client = HistoricMetalPriceClient()
    client._session = AsyncMock()
    client._session.get = AsyncMock(return_value=_make_response(payload, ok=ok))
    return client


class TestHistoricMetalPriceClientParsing:
    def test_build_parses_sparse_series(self):
        client = HistoricMetalPriceClient()
        raw = {
            "2025-01-03": {"EUR": "102", "USD": "112"},
            "2025-01-01": {"EUR": "100", "USD": "110"},
        }

        rates = client._build(raw)

        assert isinstance(rates, HistoricMetalRates)
        assert rates.unit == WeightUnit.TROY_OUNCE
        assert rates.days == (date(2025, 1, 1), date(2025, 1, 3))
        assert rates.prices["EUR"] == (Dezimal("100"), Dezimal("102"))
        assert rates.prices["USD"] == (Dezimal("110"), Dezimal("112"))

    def test_build_returns_none_for_empty(self):
        client = HistoricMetalPriceClient()
        assert client._build({}) is None

    def test_price_at_uses_previous_known_day(self):
        client = HistoricMetalPriceClient()
        rates = client._build(
            {
                "2025-01-01": {"EUR": "100"},
                "2025-01-03": {"EUR": "102"},
            }
        )

        # Before range start -> first known
        assert rates.price_at(date(2024, 12, 31), "EUR") == Dezimal("100")
        assert rates.price_at(date(2025, 1, 1), "EUR") == Dezimal("100")
        # Missing day -> closest previous known
        assert rates.price_at(date(2025, 1, 2), "EUR") == Dezimal("100")
        assert rates.price_at(date(2025, 1, 3), "EUR") == Dezimal("102")
        # After range end -> last known
        assert rates.price_at(date(2025, 6, 1), "EUR") == Dezimal("102")
        # Unknown currency
        assert rates.price_at(date(2025, 1, 1), "GBP") is None

    @pytest.mark.parametrize(
        "raw",
        [
            {"bad-date": {"EUR": "100"}},
            {"2025-01-01": {"EUR": "not-a-number"}},
            {"2025-01-01": ["EUR", "100"]},
            ["2025-01-01"],
        ],
    )
    def test_build_rejects_malformed_payload(self, raw):
        client = HistoricMetalPriceClient()

        with pytest.raises(HistoricRatesParseError):
            client._build(raw)


class TestHistoricMetalPriceClientFetch:
    @pytest.mark.asyncio
    async def test_returns_rates_and_caches(self):
        payload = {
            "2025-01-02": {"EUR": "102", "USD": "112"},
            "2025-01-01": {"EUR": "100", "USD": "110"},
        }
        client = _build_client(payload)

        result = await client.get_partial_historic_rates(CommodityType.GOLD)

        assert isinstance(result, HistoricMetalRates)
        assert result.days[0] == date(2025, 1, 1)
        assert result.price_at(date(2025, 1, 2), "USD") == Dezimal("112")
        client._session.get.assert_awaited_once()
        called_url = client._session.get.await_args.args[0]
        assert called_url.endswith("/xau.json")

        # Second call must hit the cache, no extra HTTP request.
        await client.get_partial_historic_rates(CommodityType.GOLD)
        client._session.get.assert_awaited_once()

    @pytest.mark.asyncio
    async def test_uses_correct_symbol(self):
        client = _build_client({"2025-01-01": {"EUR": "100"}})

        await client.get_partial_historic_rates(CommodityType.SILVER)

        assert client._session.get.await_args.args[0].endswith("/xag.json")

    @pytest.mark.asyncio
    async def test_returns_none_on_failed_response(self):
        client = _build_client({}, ok=False)

        result = await client.get_partial_historic_rates(CommodityType.PLATINUM)

        assert result is None

    @pytest.mark.asyncio
    async def test_returns_none_and_skips_cache_on_malformed_payload(self):
        client = _build_client({"bad-date": {"EUR": "100"}})

        assert await client.get_partial_historic_rates(CommodityType.GOLD) is None
        assert await client.get_partial_historic_rates(CommodityType.GOLD) is None

        assert client._session.get.await_count == 2

    @pytest.mark.asyncio
    async def test_raises_for_unsupported_commodity(self):
        client = HistoricMetalPriceClient()

        class _Fake:
            value = "BRONZE"

        with pytest.raises(ValueError):
            await client.get_partial_historic_rates(_Fake())


class TestWithoutConfiguredSource:
    @pytest.mark.asyncio
    async def test_makes_no_request(self, monkeypatch):
        monkeypatch.setattr(HistoricMetalPriceClient, "BASE_URL", None)
        client = _build_client({"2025-01-01": {"EUR": "100"}})

        assert await client.get_partial_historic_rates(CommodityType.GOLD) is None

        client._session.get.assert_not_called()
