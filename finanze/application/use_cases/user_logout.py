import logging
from typing import Optional

from application.ports.agent_runtime_port import AgentRuntimePort
from application.ports.api_session_port import ApiSessionPort
from application.ports.config_port import ConfigPort
from application.ports.datasource_initiator import DatasourceInitiator
from application.ports.error_reporter_port import ErrorReporterPort
from application.ports.import_preview_store import ImportPreviewStore
from application.ports.sheets_initiator import SheetsInitiator
from domain.use_cases.user_logout import UserLogout


class UserLogoutImpl(UserLogout):
    def __init__(
        self,
        source_initiator: DatasourceInitiator,
        config_port: ConfigPort,
        sheets_initiator: SheetsInitiator,
        error_reporter: Optional[ErrorReporterPort] = None,
        agent_runtime_port: Optional[AgentRuntimePort] = None,
        import_preview_store: Optional[ImportPreviewStore] = None,
        api_session_port: Optional[ApiSessionPort] = None,
    ):
        self._source_initiator = source_initiator
        self._config_port = config_port
        self._sheets_initiator = sheets_initiator
        self._error_reporter = error_reporter
        self._agent_runtime_port = agent_runtime_port
        self._import_preview_store = import_preview_store
        self._api_session_port = api_session_port
        self._log = logging.getLogger(__name__)

    async def execute(self):
        if self._api_session_port:
            self._api_session_port.revoke_all()
        await self._config_port.disconnect()
        self._sheets_initiator.disconnect()
        if self._agent_runtime_port:
            await self._agent_runtime_port.release_all()
        if self._import_preview_store:
            self._import_preview_store.clear()
        await self._source_initiator.lock()

        if self._error_reporter:
            self._error_reporter.set_user(None)

        self._log.info("User logged out successfully")
