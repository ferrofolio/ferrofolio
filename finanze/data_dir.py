# SPDX-License-Identifier: AGPL-3.0-only
"""The default data directory and the one-time move from the upstream name.

Without --data-dir the data lives in `Ferrofolio` in the platform's user data
location. Installations from before the rename kept it in `Finanze` next to it;
that directory is renamed once, on the first start that finds it. The Rust server
resolves the default the same way (`crates/server/src/user_data.rs`) and passes it
on, so behind it this module only runs when Python is started on its own.
"""

import os
from dataclasses import dataclass
from pathlib import Path
from typing import Optional

import appdirs

APP_NAME = "Ferrofolio"
LEGACY_APP_NAME = "Finanze"


@dataclass(frozen=True)
class DefaultDataDir:
    path: Path
    # Logged once logging is configured, which needs the directory first
    message: Optional[str] = None
    warning: bool = False


def default_data_dir() -> DefaultDataDir:
    return adopt_legacy_dir(
        Path(appdirs.user_data_dir(LEGACY_APP_NAME, False)),
        Path(appdirs.user_data_dir(APP_NAME, False)),
    )


def adopt_legacy_dir(legacy: Path, target: Path) -> DefaultDataDir:
    """Renames `legacy` to `target` unless `target` already holds something.

    If both hold data, `target` is used and `legacy` is left alone. If the rename
    fails, `legacy` stays in use, so that the data is not hidden behind an empty
    directory.
    """
    if not legacy.is_dir():
        return DefaultDataDir(target)
    try:
        if _has_entries(target):
            return DefaultDataDir(
                target,
                f"Both {legacy} and {target} exist; using {target} and leaving "
                f"{legacy} unchanged. Start with --data-dir {legacy} to use the "
                "old directory instead",
                warning=True,
            )
        # Replaces an empty target directory atomically (POSIX rename)
        os.rename(legacy, target)
    except OSError as e:
        if _is_missing(legacy) and target.is_dir():
            # Another start moved it in the meantime
            return DefaultDataDir(target)
        return DefaultDataDir(
            legacy,
            f"Could not move the data directory {legacy} to {target}, keeping "
            f"the old one: {e}",
            warning=True,
        )
    return DefaultDataDir(target, f"Moved the data directory from {legacy} to {target}")


def _is_missing(path: Path) -> bool:
    """Whether nothing exists at path; an error other than not found does not count."""
    try:
        os.stat(path)
    except FileNotFoundError:
        return True
    except OSError:
        return False
    return False


def _has_entries(directory: Path) -> bool:
    try:
        with os.scandir(directory) as entries:
            return next(entries, None) is not None
    except FileNotFoundError:
        return False
