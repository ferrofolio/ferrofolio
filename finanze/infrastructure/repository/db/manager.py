import asyncio
import hashlib
import logging
from pathlib import Path
from sqlite3 import Connection
from typing import Optional
from asyncio import Lock

from sqlcipher3 import dbapi2 as sqlcipher
from sqlcipher3._sqlite3 import DatabaseError

from application.ports.datasource_initiator import DatasourceInitiator
from domain.data_init import (
    AlreadyUnlockedError,
    DatasourceInitParams,
    DecryptionError,
    MigrationError,
)
from domain.user import User
from infrastructure.repository.db.client import DBClient, UnderlyingConnection
from infrastructure.repository.db.upgrader import DatabaseUpgrader
from infrastructure.repository.db.version_registry import versions

DB_NAME = "data.db"


class DBManager(DatasourceInitiator):
    def __init__(self, db_client: DBClient):
        self._log = logging.getLogger(__name__)
        self._client = db_client
        self._lock = Lock()
        self._pass = None
        self._user: User | None = None
        self._unlocked = False

    @property
    def unlocked(self) -> bool:
        return self._unlocked

    async def lock(self):
        async with self._lock:
            if not self._unlocked:
                self._log.warning("Database is already locked.")
                return

            self._unlocked = False
            self._user = None
            self._pass = None
            await self._client.close()
            self._log.debug("Database locked successfully.")

    async def initialize(self, params: DatasourceInitParams):
        await self._initialize(params)

    async def _initialize(self, params: DatasourceInitParams) -> UnderlyingConnection:
        user_path = params.user.path

        user_db_path = user_path / DB_NAME
        self._log.info(f"Attempting to connect and unlock database at {user_db_path}")

        async with self._lock:
            if self._unlocked:
                self._log.info("Database is already unlocked")
                raise AlreadyUnlockedError()

            self._unlocked = False
            connection = None
            try:
                connection = self._base_connect(user_db_path)

                self._unlock_and_setup(connection, params.password)
                self._log.info("Database unlocked successfully")

                self._unlocked = True
                self._client.set_connection(connection)

                await self._setup_database_schema(params)
                self._pass = params.password
                self._user = params.user

                return connection

            except Exception as e:
                if isinstance(e, DatabaseError):
                    self._log.exception(f"Failed to unlock database: {e}")
                    if "file is not a database" in str(e) or "encrypted" in str(e):
                        raise DecryptionError(
                            "Failed to decrypt database. Incorrect password or corrupted file"
                        ) from e

                elif not isinstance(e, MigrationError):
                    self._log.exception(e)

                self._unlocked = False
                if connection:
                    connection.close()
                self._client.set_connection(None)
                raise

    @staticmethod
    def _base_connect(user_db_path: Path) -> Connection:
        connection = sqlcipher.connect(
            database=str(user_db_path),
            isolation_level=None,
            check_same_thread=False,
        )
        return connection

    @staticmethod
    def _sanitize_password(password: str) -> str:
        return password.replace(r"'", r"''")

    @staticmethod
    def _unlock_and_setup(connection: UnderlyingConnection, password: str):
        sanitized_pass = DBManager._sanitize_password(password)
        connection.execute(f"PRAGMA key='{sanitized_pass}';")

        connection.execute("SELECT count(*) FROM sqlite_master WHERE type='table';")

        connection.execute("PRAGMA journal_mode = WAL;")

        connection.execute("PRAGMA foreign_keys = ON;")
        connection.row_factory = sqlcipher.Row

        return connection

    async def change_password(
        self, user_params: DatasourceInitParams, new_password: str
    ):
        if not new_password:
            raise ValueError("New password cannot be empty")

        if self._unlocked:
            raise Exception(
                "Database is unlocked, it must be locked before changing password"
            )

        connection = await self._initialize(user_params)
        self._change_password(connection, new_password)
        await self.lock()

    def _change_password(self, connection: UnderlyingConnection, new_password: str):
        if not self._unlocked:
            raise Exception("Database must be unlocked before changing password")

        sanitized_pass = self._sanitize_password(new_password)
        connection.execute(f"PRAGMA rekey='{sanitized_pass}';")

        self._log.info("Database password changed successfully")

    async def _setup_database_schema(self, params: DatasourceInitParams):
        if not self._unlocked:
            raise Exception("Database must be unlocked before setting up schema")

        self._log.debug("Setting up database schema...")

        upgrader = DatabaseUpgrader(self._client, versions, params.context)

        try:
            await upgrader.upgrade()
            self._log.info("Database schema setup complete")
        except MigrationError:
            raise
        except Exception as e:
            raise MigrationError(f"Database schema setup failed: {e}") from e

    async def get_hashed_password(self) -> Optional[str]:
        if not self._unlocked:
            return None

        return hashlib.sha3_256(self._pass.encode("utf-8")).hexdigest()

    async def verify_password(self, password: str) -> bool:
        user = self._user
        if not self._unlocked or user is None or not password:
            return False

        return await asyncio.to_thread(
            self._check_password, user.path / DB_NAME, password
        )

    @staticmethod
    def _check_password(user_db_path: Path, password: str) -> bool:
        connection = DBManager._base_connect(user_db_path)
        try:
            sanitized_pass = DBManager._sanitize_password(password)
            connection.execute(f"PRAGMA key='{sanitized_pass}';")
            connection.execute("SELECT count(*) FROM sqlite_master;")
            return True
        except DatabaseError:
            return False
        finally:
            connection.close()

    def get_user(self) -> Optional[User]:
        return self._user

    def set_user(self, user: User):
        self._user = user
