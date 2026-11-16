from types import SimpleNamespace
from unittest.mock import AsyncMock
from urllib.parse import parse_qs, urlsplit
from uuid import uuid4

import pytest
import pytest_asyncio
from domain.exception.exceptions import ExternalEntityLinkError, ExternalEntityNotFound
from domain.external_entity import (
    ExternalEntityConnectionResult,
    ExternalEntitySetupResponseCode,
)
from domain.platform import OS
from domain.status import BackendDetails, BackendOptions, GlobalStatus, LoginStatusCode
from infrastructure.client.financial.enablebanking.auth_state import build_auth_state
from infrastructure.controller.config import quart
from infrastructure.controller.routes.connect_external_entity import (
    connect_external_entity,
)
from infrastructure.controller.routes.get_status import status as get_status
from infrastructure.controller.routes.relay_external_entity_callback import (
    relay_external_entity_callback,
)

CALLBACK_URL = "/api/v1/entities/external/callback"
CONNECT_URL = "/api/v1/entities/external"
STATUS_URL = "/api/v1/status"
ENTITY_ID = str(uuid4())
COMPLETION_URL = "https://my-host.example/app/#/entities"
BODY = {"institution_id": "ES:Bank", "provider": "ENABLE_BANKING"}


@pytest_asyncio.fixture
async def ctx(tmp_path):
    complete_uc = AsyncMock()
    connect_uc = AsyncMock()
    connect_uc.execute.return_value = ExternalEntityConnectionResult(
        ExternalEntitySetupResponseCode.CONTINUE_WITH_LINK,
        link="https://auth.example/start",
        id=uuid4(),
    )

    status_uc = AsyncMock()
    status_uc.execute.return_value = GlobalStatus(
        status=LoginStatusCode.LOCKED,
        server=BackendDetails(
            version="1.0.0", platform_type=OS.LINUX, options=BackendOptions()
        ),
        features={},
    )

    static_dir = tmp_path / "static"
    static_dir.mkdir()
    app = quart(static_dir)

    @app.route(CALLBACK_URL, methods=["GET"])
    async def callback_route():
        return await relay_external_entity_callback(complete_uc)

    @app.route(CONNECT_URL, methods=["POST"])
    async def connect_route():
        return await connect_external_entity(connect_uc)

    @app.route(STATUS_URL, methods=["GET"])
    async def status_route():
        return await get_status(status_uc)

    yield SimpleNamespace(
        http=app.test_client(),
        complete_uc=complete_uc,
        connect_uc=connect_uc,
        status_uc=status_uc,
    )


class TestRelayToClient:
    @pytest.mark.asyncio
    async def test_redirects_to_completion_url_with_code_and_entity_id(self, ctx):
        state = build_auth_state(ENTITY_ID, COMPLETION_URL)

        response = await ctx.http.get(
            CALLBACK_URL, query_string={"code": "code-1", "state": state}
        )

        assert response.status_code == 302
        location = urlsplit(response.headers["Location"])
        assert (location.scheme, location.netloc, location.path) == (
            "https",
            "my-host.example",
            "/app/",
        )
        assert location.fragment == "/entities"
        assert parse_qs(location.query) == {"code": ["code-1"], "state": [ENTITY_ID]}
        ctx.complete_uc.execute.assert_not_awaited()

    @pytest.mark.asyncio
    async def test_keeps_existing_query_of_completion_url(self, ctx):
        state = build_auth_state(ENTITY_ID, "https://my-host.example/?lang=en")

        response = await ctx.http.get(
            CALLBACK_URL, query_string={"code": "code-1", "state": state}
        )

        query = parse_qs(urlsplit(response.headers["Location"]).query)
        assert query == {"lang": ["en"], "code": ["code-1"], "state": [ENTITY_ID]}

    @pytest.mark.asyncio
    async def test_forwards_provider_error(self, ctx):
        state = build_auth_state(ENTITY_ID, COMPLETION_URL)

        response = await ctx.http.get(
            CALLBACK_URL, query_string={"error": "access_denied", "state": state}
        )

        assert response.status_code == 302
        query = parse_qs(urlsplit(response.headers["Location"]).query)
        assert query == {"error": ["access_denied"], "state": [ENTITY_ID]}


class TestCompleteWithoutClient:
    @pytest.mark.asyncio
    async def test_completes_the_link_directly(self, ctx):
        state = build_auth_state(ENTITY_ID, None)

        response = await ctx.http.get(
            CALLBACK_URL, query_string={"code": "code-1", "state": state}
        )

        assert response.status_code == 200
        request = ctx.complete_uc.execute.await_args.args[0]
        assert request.external_entity_id == ENTITY_ID
        assert request.payload == {"external_entity_id": ENTITY_ID, "code": "code-1"}

    @pytest.mark.asyncio
    async def test_reports_unknown_entity(self, ctx):
        ctx.complete_uc.execute.side_effect = ExternalEntityNotFound()
        state = build_auth_state(ENTITY_ID, None)

        response = await ctx.http.get(
            CALLBACK_URL, query_string={"code": "code-1", "state": state}
        )

        assert response.status_code == 404

    @pytest.mark.asyncio
    async def test_reports_link_failure(self, ctx):
        ctx.complete_uc.execute.side_effect = ExternalEntityLinkError(details="nope")
        state = build_auth_state(ENTITY_ID, None)

        response = await ctx.http.get(
            CALLBACK_URL, query_string={"code": "code-1", "state": state}
        )

        assert response.status_code == 500


class TestRejectsForeignState:
    @pytest.mark.asyncio
    @pytest.mark.parametrize(
        "state",
        [None, ENTITY_ID, f"{ENTITY_ID}~aHR0cHM6Ly9hdHRhY2tlci5leGFtcGxlLw"],
    )
    async def test_rejects_state_not_issued_by_this_server(self, ctx, state):
        query = {"code": "code-1"}
        if state is not None:
            query["state"] = state

        response = await ctx.http.get(CALLBACK_URL, query_string=query)

        assert response.status_code == 400
        assert "Location" not in response.headers
        ctx.complete_uc.execute.assert_not_awaited()

    @pytest.mark.asyncio
    async def test_does_not_redirect_to_a_swapped_completion_url(self, ctx):
        genuine = build_auth_state(ENTITY_ID, COMPLETION_URL)
        forged = build_auth_state(ENTITY_ID, "https://attacker.example/")
        entity_id, _, signature = genuine.split("~")
        forged_url = forged.split("~")[1]

        response = await ctx.http.get(
            CALLBACK_URL,
            query_string={
                "code": "c",
                "state": f"{entity_id}~{forged_url}~{signature}",
            },
        )

        assert response.status_code == 400
        assert "Location" not in response.headers


class TestRedirectHost:
    @pytest.mark.asyncio
    async def test_uses_the_request_host(self, ctx):
        await ctx.http.post(CONNECT_URL, json=BODY, headers={"Host": "my-host:7592"})

        request = ctx.connect_uc.execute.await_args.args[0]
        assert request.redirect_host == "http://my-host:7592"

    @pytest.mark.asyncio
    async def test_honours_reverse_proxy_headers(self, ctx):
        await ctx.http.post(
            CONNECT_URL,
            json=BODY,
            headers={
                "Host": "internal:7592",
                "X-Forwarded-Proto": "https",
                "X-Forwarded-Host": "finance.example.com",
            },
        )

        request = ctx.connect_uc.execute.await_args.args[0]
        assert request.redirect_host == "https://finance.example.com"

    @pytest.mark.asyncio
    async def test_ignores_unknown_forwarded_scheme(self, ctx):
        await ctx.http.post(
            CONNECT_URL,
            json=BODY,
            headers={"Host": "my-host:7592", "X-Forwarded-Proto": "javascript"},
        )

        request = ctx.connect_uc.execute.await_args.args[0]
        assert request.redirect_host == "http://my-host:7592"

    @pytest.mark.asyncio
    async def test_configured_public_url_takes_precedence(self, ctx, monkeypatch):
        monkeypatch.setenv("FINANZE_PUBLIC_URL", "https://finance.example.com/")

        await ctx.http.post(CONNECT_URL, json=BODY, headers={"Host": "internal:7592"})

        request = ctx.connect_uc.execute.await_args.args[0]
        assert request.redirect_host == "https://finance.example.com"


class TestStatusPublicUrl:
    @pytest.mark.asyncio
    async def test_reports_the_address_of_the_current_request(self, ctx):
        response = await ctx.http.get(STATUS_URL, headers={"Host": "my-host:7592"})
        payload = await response.get_json()

        assert payload["server"]["publicUrl"] == "http://my-host:7592"

    @pytest.mark.asyncio
    async def test_reports_the_configured_public_url(self, ctx, monkeypatch):
        monkeypatch.setenv("FINANZE_PUBLIC_URL", "https://finance.example.com/")

        response = await ctx.http.get(STATUS_URL, headers={"Host": "internal:7592"})
        payload = await response.get_json()

        assert payload["server"]["publicUrl"] == "https://finance.example.com"
