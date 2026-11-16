import logging
from urllib.parse import parse_qsl, urlencode, urlsplit, urlunsplit

from domain.use_cases.complete_external_entity_connection import (
    CompleteExternalEntityConnection,
)
from infrastructure.client.financial.enablebanking.auth_state import (
    InvalidAuthState,
    parse_auth_state,
)
from infrastructure.controller.routes.complete_external_entity_connection import (
    complete_external_entity_link,
    error_connecting_external_entity,
)
from quart import redirect, request

logger = logging.getLogger(__name__)


def _with_query(url: str, params: dict[str, str]) -> str:
    parsed = urlsplit(url)
    query = parse_qsl(parsed.query, keep_blank_values=True)
    query.extend(params.items())
    return urlunsplit(parsed._replace(query=urlencode(query)))


async def relay_external_entity_callback(
    complete_external_entity_connection_uc: CompleteExternalEntityConnection,
):
    """Receive the provider redirect after the user gave consent at the bank.

    The provider is told to redirect here instead of to a third-party relay. The
    client that started the connection is identified by the signed state: if it
    passed a completion URL the browser is sent back there with the result,
    otherwise the connection is completed right away.
    """
    args = request.args
    code = args.get("code")
    error = args.get("error")

    try:
        external_entity_id, completion_url = parse_auth_state(args.get("state"))
    except InvalidAuthState as e:
        logger.warning("Rejected external entity callback: %s", e)
        return error_connecting_external_entity(400)

    if completion_url:
        params = {"state": external_entity_id}
        if code:
            params["code"] = code
        if error:
            params["error"] = error
        return redirect(_with_query(completion_url, params))

    payload = {"external_entity_id": external_entity_id}
    if code:
        payload["code"] = code
    if error:
        payload["error"] = error
    return await complete_external_entity_link(
        complete_external_entity_connection_uc, external_entity_id, payload
    )
