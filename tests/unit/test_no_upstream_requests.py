# SPDX-License-Identifier: AGPL-3.0-only
"""Guard against new dependencies on upstream services under finanze.me.

Roadmap phase 0 is done when a fresh build makes no request to *.finanze.me.
This test scans the code that ships in a build for addresses on that domain.
"""

import re
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]

SCANNED = [
    ROOT / "finanze",
    ROOT / "frontend" / "app" / "src",
    ROOT / "frontend" / "app" / "electron",
    ROOT / "frontend" / "generate-env-config.sh",
    ROOT / "Dockerfile",
    ROOT / "frontend" / "Dockerfile",
    ROOT / "docker-compose.yml",
]
SUFFIXES = {".py", ".ts", ".tsx", ".js", ".jsx", ".sh", ".yml", ".yaml", ""}
EXCLUDED_PARTS = {"node_modules", "__pycache__", "locales"}

UPSTREAM = re.compile(r"(?:https?://|@|\b)(?:[a-z0-9-]+\.)*finanze\.me\b", re.I)

# Files that still reference finanze.me, each with the change that removes it.
# Remove an entry once that change is merged; do not add new ones.
PENDING = {
    # Upstream cloud UI; hidden unless the CLOUD feature flag is on. Links and
    # e-mail addresses only, nothing is requested.
    "frontend/app/src/components/settings/CloudTab.tsx": "upstream cloud UI",
    "frontend/app/src/components/ui/PlusMessage.tsx": "upstream cloud UI",
    # Source code and licence links (#24)
    "frontend/app/src/about/AboutWindow.tsx": "#24",
    "frontend/app/src/components/settings/GeneralTab.tsx": "#24",
    "frontend/app/src/pages/EntityIntegrationsPage.tsx": "#24",
    # Google Sheets redirect (#30)
    "finanze/infrastructure/sheets/sheets_service_loader.py": "#30",
    # Feature flags and public keychain (#31)
    "finanze/infrastructure/client/features/feature_flag_client.py": "#31",
    "finanze/infrastructure/client/keychain/public_keychain_client.py": "#31",
    # Static data, ticker icons and cloud backups (#32)
    "finanze/infrastructure/client/cloud/backup/backup_client.py": "#32",
    "finanze/infrastructure/client/rates/crypto/crypto_dataset_client.py": "#32",
    "finanze/infrastructure/client/rates/metal/historic_metal_price_client.py": "#32",
    "frontend/app/src/components/contributions/ContributionsView.tsx": "#32",
    "frontend/app/src/components/transactions/ManualTransactionDialog.tsx": "#32",
    "frontend/app/src/pages/StocksInvestmentPage.tsx": "#32",
}


def _files():
    for path in SCANNED:
        if path.is_file():
            yield path
            continue
        for file in path.rglob("*"):
            if (
                file.is_file()
                and file.suffix in SUFFIXES
                and not EXCLUDED_PARTS.intersection(file.parts)
            ):
                yield file


def _references() -> dict[str, list[str]]:
    found: dict[str, list[str]] = {}
    for file in _files():
        # A file that cannot be read as UTF-8 fails the scan instead of being skipped
        text = file.read_text(encoding="utf-8")
        for number, line in enumerate(text.splitlines(), start=1):
            if UPSTREAM.search(line):
                relative = file.relative_to(ROOT).as_posix()
                found.setdefault(relative, []).append(f"{relative}:{number}")
    return found


def test_pattern_matches_upstream_addresses():
    for text in (
        "https://finanze.me",
        "https://api.finanze.me/v1",
        "http://static.finanze.me/icons",
        "contact@finanze.me",
        "features.api.finanze.me",
    ):
        assert UPSTREAM.search(text), text

    for text in ("finanze.metadata", "https://ferrofolio.app", "finanze/me"):
        assert not UPSTREAM.search(text), text


def test_shipped_code_makes_no_new_upstream_requests():
    unexpected = [
        location
        for file, locations in sorted(_references().items())
        if file not in PENDING
        for location in locations
    ]

    assert unexpected == [], (
        "Code must not reference services under finanze.me (AGENTS.md). "
        f"Found: {unexpected}"
    )
