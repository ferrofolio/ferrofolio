from dataclasses import dataclass, field
from enum import Enum
from typing import Optional, TypeAlias

from domain.platform import OS
from domain.user import User


class FFStatus(str, Enum):
    ON = "ON"
    OFF = "OFF"


FFValue = FFStatus | str

FeatureFlags: TypeAlias = dict[str, FFValue]


class Capability(str, Enum):
    """A feature of the API that a client can check for in the status.

    Clients use these to detect a server that is too old for them. A value is
    only ever added, never renamed or removed.
    """

    # Login and non-guest signup return a bearer token, which every request
    # outside the public paths must carry once the data is unlocked.
    API_SESSIONS = "apiSessions"


class LoginStatusCode(str, Enum):
    LOCKED = "LOCKED"
    UNLOCKED = "UNLOCKED"


class BackendLogLevel(str, Enum):
    NONE = "NONE"
    DEBUG = "DEBUG"
    INFO = "INFO"
    WARNING = "WARNING"
    ERROR = "ERROR"
    CRITICAL = "CRITICAL"


@dataclass
class BackendOptions:
    data_dir: Optional[str] = None
    port: Optional[int] = None
    log_level: Optional[BackendLogLevel] = None
    log_dir: Optional[str] = None
    log_file_level: Optional[BackendLogLevel] = None
    third_party_log_level: Optional[BackendLogLevel] = None


@dataclass
class BackendDetails:
    version: str
    platform_type: OS
    options: BackendOptions
    platform_version: Optional[str] = None


@dataclass
class GlobalStatus:
    status: LoginStatusCode
    server: BackendDetails
    features: FeatureFlags
    user: Optional[User] = None
    last_logged: Optional[str] = None
    pending_register: bool = False
    api_session: bool = False
    capabilities: list[Capability] = field(default_factory=list)
