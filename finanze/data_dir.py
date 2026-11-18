# SPDX-License-Identifier: AGPL-3.0-only
import os
import shutil
import uuid
from dataclasses import dataclass
from pathlib import Path
from typing import Optional

import appdirs

APP_DIR_NAME = "Ferrofolio"
LEGACY_APP_DIR_NAME = "Finanze"
STAGING_SUFFIX = ".migrating"
MIGRATED_SUFFIX = ".migrated"


def default_data_dir() -> Path:
    return Path(appdirs.user_data_dir(APP_DIR_NAME, False))


def legacy_data_dir() -> Path:
    return Path(appdirs.user_data_dir(LEGACY_APP_DIR_NAME, False))


@dataclass(frozen=True)
class DataDirMigration:
    """Outcome of moving the data of a previous installation.

    data_dir is the directory to use. It is the legacy directory when moving
    failed, so the existing data stays available.
    """

    data_dir: Path
    moved_from: Optional[Path] = None
    conflict: Optional[Path] = None
    error: Optional[str] = None


def _has_entries(path: Path) -> bool:
    try:
        return path.is_dir() and any(path.iterdir())
    except FileNotFoundError:
        # Removed in the meantime, e.g. moved away by a concurrent start
        return False


def _is_populated_dir(path: Path) -> bool:
    try:
        return _has_entries(path)
    except OSError:
        return False


def _copy_then_publish(legacy: Path, target: Path) -> None:
    """Copy into a staging directory and only publish it as target when complete.

    Each attempt uses its own staging directory, so a concurrent start can never
    remove or publish another attempt's partial copy. The legacy directory is
    set aside instead of deleted: a Finanze process still running could have
    written to it after the copy, and those writes must not be lost.
    """
    staging = target.with_name(f"{target.name}{STAGING_SUFFIX}-{uuid.uuid4().hex}")
    try:
        shutil.copytree(legacy, staging, symlinks=True)
        os.rename(staging, target)
    except BaseException:
        shutil.rmtree(staging, ignore_errors=True)
        raise
    try:
        os.rename(
            legacy,
            legacy.with_name(f"{legacy.name}{MIGRATED_SUFFIX}-{uuid.uuid4().hex}"),
        )
    except OSError:
        # Kept in place; the next start reports it as a conflict
        pass


def migrate_legacy_data_dir(
    target: Path, legacy: Optional[Path] = None
) -> DataDirMigration:
    """Move the data of an existing Finanze installation to the Ferrofolio directory.

    Only runs when the target does not exist yet or is empty, so data that is
    already in the new directory is never overwritten. The target path never
    holds a partial copy: the move is an atomic rename, or, across file systems,
    a copy into a staging directory that is renamed once it is complete.
    """
    legacy = legacy if legacy is not None else legacy_data_dir()

    try:
        if not _has_entries(legacy) or legacy.resolve() == target.resolve():
            return DataDirMigration(target)
    except OSError as e:
        if _is_populated_dir(target):
            return DataDirMigration(target)
        return DataDirMigration(legacy, error=f"{type(e).__name__}: {e}")

    try:
        if target.exists() and not target.is_dir():
            return DataDirMigration(
                legacy, error=f"{target} exists and is not a directory"
            )
        if _has_entries(target):
            return DataDirMigration(target, conflict=legacy)
    except OSError as e:
        return DataDirMigration(legacy, error=f"{type(e).__name__}: {e}")

    try:
        if target.exists():
            target.rmdir()
        target.parent.mkdir(parents=True, exist_ok=True)
        try:
            os.rename(legacy, target)
        except OSError:
            _copy_then_publish(legacy, target)
    except OSError as e:
        if _has_entries(target):
            # The target was empty before, so a concurrent start published it
            return DataDirMigration(target)
        return DataDirMigration(legacy, error=f"{type(e).__name__}: {e}")

    return DataDirMigration(target, moved_from=legacy)
