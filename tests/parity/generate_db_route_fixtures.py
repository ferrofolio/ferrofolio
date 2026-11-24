# SPDX-License-Identifier: AGPL-3.0-only
"""Generate the parity fixtures for the routes that read the database.

    python tests/parity/generate_db_route_fixtures.py --output crates/server/tests/fixtures/db_routes
    python tests/parity/generate_db_route_fixtures.py --check crates/server/tests/fixtures/db_routes

--output creates one encrypted database per scenario with the Python backend
(every migration of this version and the scenario's data), writes databases.json
with the migrations and the data of each, and records every case of every slice
into <slice>.json through the real Quart routes and the production session guard.
The Rust tests in crates/server/tests open the same databases and replay the cases.

--check exits with status 1 when the committed databases do not hold exactly the
data their scenarios define, lack a migration of this version, or when Python
answers a case differently now. Regenerate after adding a migration or changing a
slice; see db_routes/harness.py for how a slice is written.
"""

import argparse
import asyncio
import sys
from pathlib import Path

from ci_python import require_ci_python
from db_routes import (
    contributions,
    harness,
    labeling_rules,
    labels,
    pending_flows,
    periodic_flows,
    settings,
)

SLICES = [
    settings.SLICE,
    contributions.SLICE,
    labels.SLICE,
    labeling_rules.SLICE,
    periodic_flows.SLICE,
    pending_flows.SLICE,
]


def main() -> int:
    require_ci_python()
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    group = parser.add_mutually_exclusive_group(required=True)
    group.add_argument("--output", type=Path, help="directory to write the fixtures to")
    group.add_argument("--check", type=Path, help="directory of the fixtures to check")
    args = parser.parse_args()

    if args.output:
        asyncio.run(harness.output(SLICES, args.output))
        return 0
    problems = asyncio.run(harness.check(SLICES, args.check))
    if problems:
        print(
            "; ".join(problems)
            + ". Regenerate with: python tests/parity/generate_db_route_fixtures.py "
            f"--output {args.check}",
            file=sys.stderr,
        )
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
