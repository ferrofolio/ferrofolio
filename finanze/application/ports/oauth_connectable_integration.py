import abc
from typing import Optional

from application.ports.connectable_integration import ConnectableIntegration
from domain.external_integration import (
    CompleteExternalIntegrationConnectionRequest,
    ExternalIntegrationPayload,
)


class OAuthConnectableIntegration(ConnectableIntegration, metaclass=abc.ABCMeta):
    async def setup(self, payload: ExternalIntegrationPayload):
        pass

    @abc.abstractmethod
    async def start_authorization(
        self,
        payload: ExternalIntegrationPayload,
        completion_url: Optional[str],
        redirect_host: Optional[str] = None,
    ) -> str:
        """Return the link where the user authorizes access.

        redirect_host is the base URL under which this server is reachable from
        the user's browser, for providers that redirect back to the server.
        """
        raise NotImplementedError

    @abc.abstractmethod
    async def complete_authorization(
        self, request: CompleteExternalIntegrationConnectionRequest
    ) -> ExternalIntegrationPayload:
        raise NotImplementedError

    async def callback_destination(self, state: str) -> Optional[str]:
        return None
