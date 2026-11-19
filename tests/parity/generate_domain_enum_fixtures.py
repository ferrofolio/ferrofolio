# SPDX-License-Identifier: AGPL-3.0-only
"""Generate the fixtures for the domain modules ported to crates/core-domain.

Two files are written to the fixture directory:

- domain_enums.json lists every enum of the ported modules, keyed by module and
  class name, with the values of its members in their order. For a module that is
  ported completely every enum it defines is listed, so an enum added in Python
  makes --check fail until the fixture, and with it the Rust port, is updated.
- domain_cases.json records inputs and the outputs Python computes for every
  function and method with logic in the ported modules.

crates/core-domain/tests/domain_parity.rs requires Rust to have the same enums and
to compute the same outputs.

    python tests/parity/generate_domain_enum_fixtures.py --output DIRECTORY
    python tests/parity/generate_domain_enum_fixtures.py --check DIRECTORY
"""

import argparse
import inspect
import json
import sys
from enum import Enum
from pathlib import Path

from ci_python import require_ci_python

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / "finanze"))

import domain.constants as constants  # noqa: E402
import domain.earnings_expenses as earnings_expenses  # noqa: E402
import domain.entity as entity  # noqa: E402
import domain.entity_account as entity_account  # noqa: E402
import domain.external_integration as external_integration  # noqa: E402
import domain.fetch_record as fetch_record  # noqa: E402
import domain.public_key as public_key  # noqa: E402

ENUMS = "domain_enums.json"
CASES = "domain_cases.json"

# Modules ported completely: every enum they define must exist in Rust.
COMPLETE_MODULES = [
    entity,
    entity_account,
    fetch_record,
    public_key,
]

# Modules ported only as far as the complete ones need them.
PARTIAL_MODULES = {
    earnings_expenses: ["FlowFrequency"],
    external_integration: ["ExternalIntegrationId"],
}


def module_name(module) -> str:
    return module.__name__.removeprefix("domain.")


def defined_enums(module) -> list[type[Enum]]:
    """The enums a module defines, in the order it defines them."""
    return [
        value
        for value in vars(module).values()
        if inspect.isclass(value)
        and issubclass(value, Enum)
        and value.__module__ == module.__name__
    ]


def enum_entry(module, enum: type[Enum]) -> tuple[str, list[str]]:
    if not issubclass(enum, str):
        raise SystemExit(f"{enum.__qualname__} is not a str enum; port it by hand")
    return f"{module_name(module)}.{enum.__name__}", [member.value for member in enum]


def enums() -> dict[str, list[str]]:
    entries = {}
    for module in COMPLETE_MODULES:
        for enum in defined_enums(module):
            key, values = enum_entry(module, enum)
            entries[key] = values
    for module, names in PARTIAL_MODULES.items():
        for name in names:
            key, values = enum_entry(module, getattr(module, name))
            entries[key] = values
    return entries


def entity_cases() -> dict:
    return {
        "is_account_scoped_entity": [
            {
                "type": entity_type.value,
                "result": entity.is_account_scoped_entity(
                    entity.Entity(
                        id=None,
                        name="Entity",
                        natural_id=None,
                        type=entity_type,
                        origin=entity.EntityOrigin.NATIVE,
                        icon_url=None,
                    )
                ),
            }
            for entity_type in entity.EntityType
        ],
    }


def constant_cases() -> dict:
    return {
        "constants": {
            "CAPITAL_GAINS_BASE_TAX": str(constants.CAPITAL_GAINS_BASE_TAX),
            "SUPPORTED_CURRENCIES": list(constants.SUPPORTED_CURRENCIES),
            "PUSD_SYMBOL": constants.PUSD_SYMBOL,
            "PUSD_CONTRACT_ADDRESS": constants.PUSD_CONTRACT_ADDRESS,
        },
    }


def cases() -> dict:
    groups = {}
    for build in (entity_cases, constant_cases):
        for name, group in build().items():
            if name in groups:
                raise SystemExit(f"case group {name} is defined twice")
            groups[name] = group
    return groups


def render(value) -> str:
    return json.dumps(value, indent=2, ensure_ascii=False) + "\n"


def render_cases(groups: dict) -> str:
    """The case groups with one case per line, which keeps the file short."""

    def line(value) -> str:
        return json.dumps(value, ensure_ascii=False, sort_keys=True)

    entries = []
    for name, group in groups.items():
        if isinstance(group, list) and group:
            body = ",\n".join(f"    {line(case)}" for case in group)
            entries.append(f"  {line(name)}: [\n{body}\n  ]")
        else:
            entries.append(f"  {line(name)}: {line(group)}")
    return "{\n" + ",\n".join(entries) + "\n}\n"


def main():
    require_ci_python()
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    group = parser.add_mutually_exclusive_group(required=True)
    group.add_argument("--output", type=Path, help="write the fixtures to DIRECTORY")
    group.add_argument("--check", type=Path, help="compare the fixtures in DIRECTORY")
    args = parser.parse_args()

    rendered = {ENUMS: render(enums()), CASES: render_cases(cases())}
    if args.output:
        for name, text in rendered.items():
            (args.output / name).write_text(text, encoding="utf-8")
        return 0
    stale = [
        name
        for name, text in rendered.items()
        if not (args.check / name).is_file()
        or (args.check / name).read_text(encoding="utf-8") != text
    ]
    for name in stale:
        print(
            f"{args.check / name} is out of date; regenerate it with --output",
            file=sys.stderr,
        )
    return 1 if stale else 0


if __name__ == "__main__":
    sys.exit(main())
