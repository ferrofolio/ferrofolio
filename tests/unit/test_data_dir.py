# SPDX-License-Identifier: AGPL-3.0-only
import sys

import pytest

import args as app_args_module
import data_dir
from data_dir import migrate_legacy_data_dir


def _legacy_with_db(tmp_path):
    legacy = tmp_path / "Finanze"
    (legacy / "logs").mkdir(parents=True)
    (legacy / "user.db").write_bytes(b"encrypted")
    return legacy


def test_moves_legacy_data_when_target_is_missing(tmp_path):
    legacy = _legacy_with_db(tmp_path)
    target = tmp_path / "Ferrofolio"

    assert migrate_legacy_data_dir(target, legacy) == legacy

    assert not legacy.exists()
    assert (target / "user.db").read_bytes() == b"encrypted"
    assert (target / "logs").is_dir()


def test_moves_legacy_data_when_target_is_empty(tmp_path):
    legacy = _legacy_with_db(tmp_path)
    target = tmp_path / "Ferrofolio"
    target.mkdir()

    assert migrate_legacy_data_dir(target, legacy) == legacy

    assert (target / "user.db").read_bytes() == b"encrypted"


def test_never_overwrites_existing_data(tmp_path):
    legacy = _legacy_with_db(tmp_path)
    target = tmp_path / "Ferrofolio"
    target.mkdir()
    (target / "user.db").write_bytes(b"newer")

    assert migrate_legacy_data_dir(target, legacy) is None

    assert (legacy / "user.db").read_bytes() == b"encrypted"
    assert (target / "user.db").read_bytes() == b"newer"


@pytest.mark.parametrize("create_legacy", [False, True])
def test_does_nothing_without_legacy_data(tmp_path, create_legacy):
    legacy = tmp_path / "Finanze"
    if create_legacy:
        legacy.mkdir()
    target = tmp_path / "Ferrofolio"

    assert migrate_legacy_data_dir(target, legacy) is None

    assert not target.exists()


def test_does_nothing_when_legacy_is_target(tmp_path):
    legacy = _legacy_with_db(tmp_path)

    assert migrate_legacy_data_dir(legacy, legacy) is None

    assert (legacy / "user.db").exists()


def _parse(monkeypatch, tmp_path, argv):
    legacy = _legacy_with_db(tmp_path)
    target = tmp_path / "Ferrofolio"
    monkeypatch.setattr(app_args_module, "default_data_dir", lambda: target)
    monkeypatch.setattr(data_dir, "legacy_data_dir", lambda: legacy)
    monkeypatch.setattr(app_args_module, "configure_logging", lambda _args: None)
    monkeypatch.setattr(sys, "argv", ["finanze", *argv])
    return legacy, target, app_args_module.parse_args()


def test_parse_args_migrates_the_default_directory(monkeypatch, tmp_path):
    legacy, target, parsed = _parse(monkeypatch, tmp_path, [])

    assert parsed.data_dir == target
    assert (target / "user.db").exists()
    assert not legacy.exists()


def test_parse_args_leaves_an_explicit_directory_alone(monkeypatch, tmp_path):
    explicit = tmp_path / "custom"
    legacy, target, parsed = _parse(
        monkeypatch, tmp_path, ["--data-dir", str(explicit)]
    )

    assert parsed.data_dir == explicit
    assert explicit.is_dir()
    assert (legacy / "user.db").exists()
    assert not target.exists()
