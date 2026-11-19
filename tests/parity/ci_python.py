# SPDX-License-Identifier: AGPL-3.0-only
"""The Python version CI runs the parity generators with.

Error messages, the Unicode database and other details differ between Python
versions, so a generator run with another version would report or record
differences that come from the interpreter, not from the backend. The version is
read from the workflow, which stays its only definition.
"""

import platform
import re
from pathlib import Path

WORKFLOW = Path(__file__).resolve().parents[2] / ".github" / "workflows" / "ci.yml"


def ci_version() -> str:
    """The Python version that `.github/workflows/ci.yml` sets up."""
    text = WORKFLOW.read_text(encoding="utf-8")
    versions = set(re.findall(r"python-version:\s*['\"]?([0-9][0-9.]*)", text))
    if len(versions) != 1:
        raise SystemExit(
            f"{WORKFLOW} names {len(versions)} Python versions; the parity "
            "generators need exactly one"
        )
    return versions.pop()


def require_ci_python() -> None:
    """Stops unless this interpreter is the CPython version CI uses."""
    expected = ci_version()
    implementation = platform.python_implementation()
    actual = platform.python_version()
    if implementation != "CPython" or actual != expected:
        raise SystemExit(
            f"The parity generators need CPython {expected}, the version CI uses; "
            f"this is {implementation} {actual}."
        )
