# SPDX-License-Identifier: AGPL-3.0-only
"""The default data directory and the move from the upstream name."""

import sys

import pytest

import args as args_module
from args import app_args, parse_args
from data_dir import DefaultDataDir, adopt_legacy_dir


@pytest.fixture
def dirs(tmp_path):
    return tmp_path / "Finanze", tmp_path / "Ferrofolio"


def _with_data(directory):
    (directory / "profiles").mkdir(parents=True)
    (directory / "profiles.json").write_text('{"profiles": []}')
    return directory


def test_uses_the_new_directory_without_legacy_data(dirs):
    legacy, target = dirs

    assert adopt_legacy_dir(legacy, target) == DefaultDataDir(target)
    assert not target.exists()


def test_moves_the_legacy_directory(dirs):
    legacy, target = _with_data(dirs[0]), dirs[1]

    result = adopt_legacy_dir(legacy, target)

    assert result.path == target
    assert not result.warning
    assert "Moved" in result.message
    assert not legacy.exists()
    assert (target / "profiles.json").read_text() == '{"profiles": []}'
    assert (target / "profiles").is_dir()


def test_moves_into_an_empty_new_directory(dirs):
    legacy, target = _with_data(dirs[0]), dirs[1]
    target.mkdir()

    result = adopt_legacy_dir(legacy, target)

    assert result.path == target
    assert not legacy.exists()
    assert (target / "profiles.json").exists()


def test_never_mixes_two_directories_with_data(dirs):
    legacy, target = _with_data(dirs[0]), dirs[1]
    target.mkdir()
    (target / "profiles.json").write_text("new")

    result = adopt_legacy_dir(legacy, target)

    assert result.path == target
    assert result.warning
    assert f"--data-dir {legacy}" in result.message
    assert (legacy / "profiles.json").read_text() == '{"profiles": []}'
    assert (target / "profiles.json").read_text() == "new"
    assert not (target / "profiles").exists()


def test_keeps_the_legacy_directory_when_the_move_fails(dirs, monkeypatch):
    legacy, target = _with_data(dirs[0]), dirs[1]

    def fail(source, destination):
        raise PermissionError("denied")

    monkeypatch.setattr("data_dir.os.rename", fail)

    result = adopt_legacy_dir(legacy, target)

    assert result.path == legacy
    assert result.warning
    assert "denied" in result.message
    assert (legacy / "profiles.json").exists()
    assert not target.exists()


def test_keeps_the_legacy_directory_when_the_new_path_is_a_file(dirs):
    legacy, target = _with_data(dirs[0]), dirs[1]
    target.write_text("not a directory")

    result = adopt_legacy_dir(legacy, target)

    assert result.path == legacy
    assert result.warning
    assert (legacy / "profiles.json").exists()


def test_data_and_log_directory_default_to_none_in_the_parser():
    parsed = app_args().parse_args([])

    assert parsed.data_dir is None
    assert parsed.log_dir is None


def _parse(monkeypatch, argv):
    monkeypatch.setattr(sys, "argv", ["finanze", *argv])
    monkeypatch.setattr(args_module, "configure_logging", lambda parsed: None)
    return parse_args()


def test_explicit_data_dir_does_not_move_anything(tmp_path, monkeypatch):
    def unexpected():
        raise AssertionError("the default directory must not be resolved")

    monkeypatch.setattr(args_module, "default_data_dir", unexpected)
    explicit = tmp_path / "storage"

    parsed = _parse(monkeypatch, ["--data-dir", str(explicit)])

    assert parsed.data_dir == explicit
    assert parsed.log_dir == explicit / "logs"
    assert explicit.is_dir()


def test_default_data_dir_is_resolved_and_logs_follow_it(tmp_path, monkeypatch):
    default = tmp_path / "Ferrofolio"
    monkeypatch.setattr(
        args_module, "default_data_dir", lambda: DefaultDataDir(default)
    )

    parsed = _parse(monkeypatch, ["--log-dir", str(tmp_path / "elsewhere")])

    assert parsed.data_dir == default
    assert parsed.log_dir == str(tmp_path / "elsewhere")
    assert default.is_dir()
