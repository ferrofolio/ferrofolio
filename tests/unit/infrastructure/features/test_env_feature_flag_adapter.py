# SPDX-License-Identifier: AGPL-3.0-only
import pytest

from domain.status import FFStatus
from infrastructure.features.env_feature_flag_adapter import EnvFeatureFlagAdapter


@pytest.fixture(autouse=True)
def _clean_env(monkeypatch):
    import os

    for key in list(os.environ):
        if key.startswith("FF_"):
            monkeypatch.delenv(key)


def test_defaults_without_environment():
    flags = EnvFeatureFlagAdapter().get_all()

    assert flags == {"TR_V2_AUTH_API": FFStatus.ON}


def test_upstream_cloud_is_off_by_default():
    adapter = EnvFeatureFlagAdapter()

    assert adapter.get_value("CLOUD") is None
    assert adapter.get_value("PLUS") is None


def test_environment_overrides_defaults(monkeypatch):
    monkeypatch.setenv("FF_TR_V2_AUTH_API", "OFF")
    monkeypatch.setenv("FF_CLOUD", "ON")

    adapter = EnvFeatureFlagAdapter()

    assert adapter.get_value("TR_V2_AUTH_API") == FFStatus.OFF
    assert adapter.get_value("CLOUD") == FFStatus.ON
