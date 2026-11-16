import base64
import hashlib
import hmac
import secrets
from typing import Optional
from urllib.parse import urlsplit

EXTERNAL_COMPLETE_DEEPLINK = "finanze://external/complete"
_SEPARATOR = "~"
_SIGNATURE_BYTES = 16

# The key lives as long as the process. An authorization that is started before a
# restart cannot be completed afterwards and has to be started again.
_signing_key = secrets.token_bytes(32)


class InvalidAuthState(ValueError):
    pass


def _b64encode(raw: bytes) -> str:
    return base64.urlsafe_b64encode(raw).decode("ascii").rstrip("=")


def _b64decode(encoded: str) -> bytes:
    padding = "=" * (-len(encoded) % 4)
    return base64.urlsafe_b64decode(encoded + padding)


def _sign(payload: str) -> str:
    digest = hmac.new(_signing_key, payload.encode("utf-8"), hashlib.sha256).digest()
    return _b64encode(digest[:_SIGNATURE_BYTES])


def is_allowed_completion_url(url: str) -> bool:
    """Whether the callback may send the browser back to this URL.

    Self-hosted instances are often reached over plain HTTP inside a local
    network, so both schemes are accepted. The provider then returns the
    authorization code in clear text; an instance that is reachable from other
    networks should be served over HTTPS.
    """
    if url.startswith(EXTERNAL_COMPLETE_DEEPLINK):
        return True
    parsed = urlsplit(url)
    return parsed.scheme in ("http", "https") and bool(parsed.netloc)


def build_auth_state(external_entity_id: str, completion_url: Optional[str]) -> str:
    """Build the state sent to the provider and returned to the callback.

    The state is signed so that the callback only redirects to completion URLs
    that this server issued itself.
    """
    encoded_url = _b64encode(completion_url.encode("utf-8")) if completion_url else ""
    payload = f"{external_entity_id}{_SEPARATOR}{encoded_url}"
    return f"{payload}{_SEPARATOR}{_sign(payload)}"


def parse_auth_state(state: Optional[str]) -> tuple[str, Optional[str]]:
    """Return the external entity id and the completion URL of a signed state."""
    if not state:
        raise InvalidAuthState("Missing state")

    parts = state.split(_SEPARATOR)
    if len(parts) != 3:
        raise InvalidAuthState("Malformed state")

    external_entity_id, encoded_url, signature = parts
    payload = f"{external_entity_id}{_SEPARATOR}{encoded_url}"
    if (
        not external_entity_id
        or not signature.isascii()
        or not hmac.compare_digest(signature, _sign(payload))
    ):
        raise InvalidAuthState("Invalid state signature")

    if not encoded_url:
        return external_entity_id, None

    try:
        completion_url = _b64decode(encoded_url).decode("utf-8")
    except (ValueError, UnicodeDecodeError) as e:
        raise InvalidAuthState("Malformed completion URL") from e

    if not is_allowed_completion_url(completion_url):
        raise InvalidAuthState("Completion URL not allowed")

    return external_entity_id, completion_url
