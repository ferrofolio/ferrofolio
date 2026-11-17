# SPDX-License-Identifier: AGPL-3.0-only
import logging
import os
import shlex
import shutil
import sys
from pathlib import Path, PureWindowsPath
from types import SimpleNamespace

import pytest

import args as app_args_module
import data_dir
from data_dir import STAGING_SUFFIX, DataDirMigration, migrate_legacy_data_dir


def _legacy_with_db(tmp_path):
    legacy = tmp_path / "Finanze"
    (legacy / "logs").mkdir(parents=True)
    (legacy / "user.db").write_bytes(b"encrypted")
    return legacy


def test_moves_legacy_data_when_target_is_missing(tmp_path):
    legacy = _legacy_with_db(tmp_path)
    target = tmp_path / "Ferrofolio"

    result = migrate_legacy_data_dir(target, legacy)

    assert result.data_dir == target
    assert result.moved_from == legacy
    assert not legacy.exists()
    assert (target / "user.db").read_bytes() == b"encrypted"
    assert (target / "logs").is_dir()


def test_moves_legacy_data_when_target_is_empty(tmp_path):
    legacy = _legacy_with_db(tmp_path)
    target = tmp_path / "Ferrofolio"
    target.mkdir()

    assert migrate_legacy_data_dir(target, legacy).moved_from == legacy

    assert (target / "user.db").read_bytes() == b"encrypted"


def test_never_overwrites_existing_data(tmp_path):
    legacy = _legacy_with_db(tmp_path)
    target = tmp_path / "Ferrofolio"
    target.mkdir()
    (target / "user.db").write_bytes(b"newer")

    result = migrate_legacy_data_dir(target, legacy)

    assert result.data_dir == target
    assert result.conflict == legacy
    assert result.moved_from is None
    assert (legacy / "user.db").read_bytes() == b"encrypted"
    assert (target / "user.db").read_bytes() == b"newer"


@pytest.mark.parametrize("create_legacy", [False, True])
def test_does_nothing_without_legacy_data(tmp_path, create_legacy):
    legacy = tmp_path / "Finanze"
    if create_legacy:
        legacy.mkdir()
    target = tmp_path / "Ferrofolio"

    result = migrate_legacy_data_dir(target, legacy)

    assert result == data_dir.DataDirMigration(target)
    assert not target.exists()


def test_does_nothing_when_legacy_is_target(tmp_path):
    legacy = _legacy_with_db(tmp_path)

    assert migrate_legacy_data_dir(legacy, legacy).moved_from is None

    assert (legacy / "user.db").exists()


def _rename_fails_across_devices(monkeypatch, legacy):
    real_rename = os.rename

    def rename(src, dst):
        if os.fspath(src) == os.fspath(legacy):
            raise OSError(18, "Invalid cross-device link")
        return real_rename(src, dst)

    monkeypatch.setattr(data_dir.os, "rename", rename)


def test_copies_through_staging_across_file_systems(tmp_path, monkeypatch):
    legacy = _legacy_with_db(tmp_path)
    target = tmp_path / "Ferrofolio"
    _rename_fails_across_devices(monkeypatch, legacy)

    result = migrate_legacy_data_dir(target, legacy)

    assert result.moved_from == legacy
    assert (target / "user.db").read_bytes() == b"encrypted"
    assert not legacy.exists()
    assert not target.with_name(target.name + STAGING_SUFFIX).exists()


def test_failed_copy_leaves_no_partial_target(tmp_path, monkeypatch):
    legacy = _legacy_with_db(tmp_path)
    target = tmp_path / "Ferrofolio"
    _rename_fails_across_devices(monkeypatch, legacy)
    real_copytree = shutil.copytree

    def copytree_then_fail(src, dst, *args, **kwargs):
        real_copytree(src, dst, *args, **kwargs)
        if os.fspath(src) == os.fspath(legacy):
            raise OSError(28, "No space left on device")

    monkeypatch.setattr(data_dir.shutil, "copytree", copytree_then_fail)

    result = migrate_legacy_data_dir(target, legacy)

    assert result.data_dir == legacy
    assert result.moved_from is None
    assert "No space left on device" in result.error
    assert not target.exists()
    assert not target.with_name(target.name + STAGING_SUFFIX).exists()
    assert (legacy / "user.db").read_bytes() == b"encrypted"


def _parse(monkeypatch, tmp_path, argv, configured=None):
    legacy = _legacy_with_db(tmp_path)
    target = tmp_path / "Ferrofolio"
    monkeypatch.setattr(app_args_module, "default_data_dir", lambda: target)
    monkeypatch.setattr(data_dir, "legacy_data_dir", lambda: legacy)

    def configure_logging(parsed):
        if configured is not None:
            configured.append(parsed)

    monkeypatch.setattr(app_args_module, "configure_logging", configure_logging)
    monkeypatch.setattr(sys, "argv", ["finanze", *argv])
    return legacy, target, app_args_module.parse_args()


def test_parse_args_migrates_the_default_directory(monkeypatch, tmp_path):
    legacy, target, parsed = _parse(monkeypatch, tmp_path, [])

    assert parsed.data_dir == target
    assert parsed.log_dir == target / "logs"
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


def test_parse_args_keeps_using_legacy_data_when_moving_fails(monkeypatch, tmp_path):
    monkeypatch.setattr(
        data_dir.shutil,
        "copytree",
        lambda *a, **k: (_ for _ in ()).throw(OSError(28, "No space")),
    )
    real_rename = os.rename

    def rename(src, dst):
        if os.fspath(src).endswith("Finanze"):
            raise OSError(18, "Invalid cross-device link")
        return real_rename(src, dst)

    monkeypatch.setattr(data_dir.os, "rename", rename)

    legacy, target, parsed = _parse(monkeypatch, tmp_path, [])

    assert parsed.data_dir == legacy
    assert parsed.log_dir == legacy / "logs"
    assert not target.exists()


def test_conflict_is_logged_after_logging_is_configured(monkeypatch, tmp_path, caplog):
    target = tmp_path / "Ferrofolio"
    target.mkdir()
    (target / "user.db").write_bytes(b"newer")
    configured = []

    with caplog.at_level(logging.WARNING):
        _, _, parsed = _parse(monkeypatch, tmp_path, [], configured)

    assert configured == [parsed]
    assert parsed.data_dir == target
    assert "Found data of a previous installation" in caplog.text


def _conflict_instruction(caplog, conflict):
    with caplog.at_level(logging.WARNING):
        app_args_module._log_migration(
            DataDirMigration(Path("/data/Ferrofolio"), conflict=conflict)
        )
    return caplog.text.split("pass ", 1)[1].split(" to keep using", 1)[0]


@pytest.mark.skipif(os.name == "nt", reason="POSIX shell quoting")
def test_conflict_instruction_quotes_the_previous_directory(caplog):
    conflict = Path("/Users/ada/Library/Application Support/Finanze")

    instruction = _conflict_instruction(caplog, conflict)

    assert shlex.split(instruction) == ["--data-dir", str(conflict)]


def test_conflict_instruction_uses_windows_quoting_on_windows(monkeypatch, caplog):
    monkeypatch.setattr(app_args_module, "os", SimpleNamespace(name="nt"))
    conflict = PureWindowsPath(r"C:\Users\Ada Lovelace\AppData\Local\Finanze")

    instruction = _conflict_instruction(caplog, conflict)

    assert instruction == f'--data-dir "{conflict}"'
