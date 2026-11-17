# SPDX-License-Identifier: AGPL-3.0-only
import logging
import shutil
from pathlib import Path
from typing import Optional

import appdirs

APP_DIR_NAME = "Ferrofolio"
LEGACY_APP_DIR_NAME = "Finanze"


def default_data_dir() -> Path:
    return Path(appdirs.user_data_dir(APP_DIR_NAME, False))


def legacy_data_dir() -> Path:
    return Path(appdirs.user_data_dir(LEGACY_APP_DIR_NAME, False))


def migrate_legacy_data_dir(
    target: Path, legacy: Optional[Path] = None
) -> Optional[Path]:
    """Move the data of an existing Finanze installation to the Ferrofolio directory.

    Only runs when the target does not exist yet or is empty, so data that is
    already in the new directory is never overwritten. Returns the directory the
    data was moved from, or None if nothing was moved.
    """
    legacy = legacy if legacy is not None else legacy_data_dir()

    if not legacy.is_dir() or legacy.resolve() == target.resolve():
        return None
    if not any(legacy.iterdir()):
        return None

    if target.exists():
        if not target.is_dir() or any(target.iterdir()):
            logging.warning(
                f"Found data of a previous installation in {legacy}, but {target} "
                f"is not empty. Leaving both untouched; pass --data-dir {legacy} "
                f"to keep using the previous data."
            )
            return None
        target.rmdir()

    target.parent.mkdir(parents=True, exist_ok=True)
    shutil.move(str(legacy), str(target))
    return legacy
