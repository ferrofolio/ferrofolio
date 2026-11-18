import os

from quart import request

PUBLIC_URL_ENV = "FINANZE_PUBLIC_URL"
_SCHEMES = ("http", "https")


def _first(value: str | None) -> str | None:
    if not value:
        return None
    return value.split(",")[0].strip() or None


def public_base_url() -> str | None:
    """Base URL under which this server is reachable from the user's browser.

    FINANZE_PUBLIC_URL takes precedence. Without it the URL is derived from the
    current request, honouring the headers a reverse proxy sets.
    """
    configured = (os.environ.get(PUBLIC_URL_ENV) or "").strip().rstrip("/")
    if configured:
        return configured

    headers = request.headers
    host = _first(headers.get("X-Forwarded-Host")) or headers.get("Host")
    if not host:
        return None

    scheme = (_first(headers.get("X-Forwarded-Proto")) or "").lower()
    if scheme not in _SCHEMES:
        # Behind the Ferrofolio gateway uvicorn takes the request scheme from
        # X-Forwarded-Proto as well, so it is checked the same way.
        scheme = (getattr(request, "scheme", None) or "").lower()
        if scheme not in _SCHEMES:
            scheme = "http"

    return f"{scheme}://{host}"
