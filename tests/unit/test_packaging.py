import runpy
import shutil
import subprocess
import sys
from pathlib import Path

import package_llama
import pytest


ROOT = Path(__file__).resolve().parents[2]


@pytest.mark.parametrize("noconfirm", [False, True])
def test_package_can_replace_previous_ci_output(monkeypatch, tmp_path, noconfirm):
    commands = []
    backend_output = tmp_path / "backend" / "dist" / "finanze-server"
    release = tmp_path / "frontend" / "app" / "release"
    release.mkdir(parents=True)

    def run_command(command, *, shell, env=None):
        assert shell
        commands.append(command)
        if command.startswith("pyinstaller "):
            if backend_output.exists():
                assert "--noconfirm" in command
            backend_output.mkdir(parents=True, exist_ok=True)
        elif command.startswith("pnpm run dist"):
            (release / "Finanze-test.dmg").write_text(f"build {len(commands)}")
        return 0

    monkeypatch.setattr(subprocess, "call", run_command)
    monkeypatch.setattr(package_llama, "fetch_llama_server", lambda **kwargs: None)
    argv = ["package.py", "--target", "full", "--os", "mac"]
    if noconfirm:
        argv.append("--noconfirm")
    monkeypatch.setattr(sys, "argv", argv)

    for _ in range(2 if noconfirm else 1):
        monkeypatch.chdir(tmp_path)
        runpy.run_path(str(ROOT / "package.py"), run_name="__main__")

    pyinstaller_commands = [cmd for cmd in commands if cmd.startswith("pyinstaller ")]
    assert len(pyinstaller_commands) == (2 if noconfirm else 1)
    assert all(("--noconfirm" in cmd) == noconfirm for cmd in pyinstaller_commands)
    assert (tmp_path / "dist" / "Finanze-test.dmg").read_text() == (
        "build 6" if noconfirm else "build 3"
    )


def test_package_preserves_existing_output_by_default(monkeypatch, tmp_path):
    release = tmp_path / "frontend" / "app" / "release"
    release.mkdir(parents=True)
    (release / "Finanze-test.dmg").write_text("new build")
    dist = tmp_path / "dist"
    dist.mkdir()
    existing = dist / "Finanze-test.dmg"
    existing.write_text("existing build")
    monkeypatch.chdir(tmp_path)
    monkeypatch.setattr(sys, "argv", ["package.py", "--target", "frontend"])
    monkeypatch.setattr(subprocess, "call", lambda *args, **kwargs: 0)

    with pytest.raises(shutil.Error):
        runpy.run_path(str(ROOT / "package.py"), run_name="__main__")

    assert existing.read_text() == "existing build"
