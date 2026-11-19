import re
import sys
from pathlib import Path

import pytest

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "parity"))

import ci_python  # noqa: E402


def test_reads_the_python_version_of_the_workflow():
    assert re.fullmatch(r"3\.\d+\.\d+", ci_python.ci_version())


def test_refuses_another_python_version(monkeypatch):
    monkeypatch.setattr(ci_python.platform, "python_version", lambda: "3.13.0")

    with pytest.raises(SystemExit, match="need CPython .*this is CPython 3.13.0"):
        ci_python.require_ci_python()


def test_refuses_another_implementation(monkeypatch):
    monkeypatch.setattr(ci_python, "ci_version", lambda: "3.14.10")
    monkeypatch.setattr(ci_python.platform, "python_version", lambda: "3.14.10")
    monkeypatch.setattr(ci_python.platform, "python_implementation", lambda: "PyPy")

    with pytest.raises(SystemExit, match="this is PyPy 3.14.10"):
        ci_python.require_ci_python()


def test_accepts_the_python_version_of_the_workflow(monkeypatch):
    monkeypatch.setattr(ci_python, "ci_version", lambda: "3.14.10")
    monkeypatch.setattr(ci_python.platform, "python_version", lambda: "3.14.10")
    monkeypatch.setattr(ci_python.platform, "python_implementation", lambda: "CPython")

    ci_python.require_ci_python()
