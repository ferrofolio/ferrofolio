# SPDX-License-Identifier: AGPL-3.0-only
"""Generate the decimal arithmetic fixtures for core_domain::Dezimal.

Python's Dezimal computes in the default decimal context: 28 significant digits,
ROUND_HALF_EVEN, and quantize(Decimal("0.01"), ROUND_HALF_UP) for cents. This
script records the results of seeded random and hand-picked operations, which
crates/core-domain/tests/decimal_parity.rs replays against the Rust type.

    python tests/parity/generate_decimal_fixtures.py --output FILE
    python tests/parity/generate_decimal_fixtures.py --check FILE
"""

import argparse
import decimal
import json
import random
import sys
from decimal import ROUND_HALF_UP, Decimal, getcontext
from pathlib import Path

SEED = 28
CENT = Decimal("0.01")


def random_operand(rng, max_digits=34):
    digits = rng.randint(1, max_digits)
    coefficient = str(rng.randint(10 ** (digits - 1), 10**digits - 1))
    exponent = rng.randint(-34, 20)
    sign = rng.choice(["", "-"])
    return Decimal(f"{sign}{coefficient}E{exponent}")


def rate(rng):
    annual = Decimal(rng.randint(-200, 1500)) / Decimal(10000)
    return annual / rng.choice([1, 2, 4, 6, 12, 24, 26, 52])


def amount(rng):
    return Decimal(rng.randint(0, 10**9)) / 100


def outcome(compute):
    try:
        return str(compute())
    except decimal.DivisionByZero:
        return "DivisionByZero"
    except decimal.InvalidOperation:
        return "InvalidOperation"
    except decimal.Overflow:
        return "Overflow"


def case(op, a, b=None):
    operations = {
        "add": lambda: a + b,
        "sub": lambda: a - b,
        "neg": lambda: -a,
        "mul": lambda: a * b,
        "div": lambda: a / b,
        "pow": lambda: a**b,
        "cents": lambda: a.quantize(CENT, rounding=ROUND_HALF_UP),
        "cmp": lambda: a.compare(b),
    }
    entry = {"op": op, "a": str(a)}
    if b is not None:
        entry["b"] = str(b)
    entry["result"] = outcome(operations[op])
    return entry


def cases():
    rng = random.Random(SEED)
    one = Decimal(1)
    entries = [
        # 150 at 1 % a year, monthly: Python rounds the product to exactly 0.125.
        case("mul", Decimal(150), Decimal("0.01") / 12),
        case("cents", Decimal(150) * (Decimal("0.01") / 12)),
        case("div", Decimal("0.035"), Decimal(12)),
        case("pow", one + Decimal("0.0025"), -360),
        case("pow", one + Decimal("0.0025"), 360),
        case("pow", Decimal("0.5"), -95),
        case("pow", Decimal(2), -100),
        case("pow", Decimal(0), -3),
        case("pow", Decimal("-0"), 3),
        case("pow", Decimal("-0"), 2),
        case("pow", Decimal("-0.00"), 1),
        case("pow", Decimal("-1.5"), 7),
        case("pow", Decimal("1.000"), -50),
        case("div", one, Decimal(0)),
        case("div", Decimal(0), Decimal("3.5")),
        case("add", Decimal("1E+30"), Decimal("1E-30")),
        case("sub", Decimal("1E+30"), Decimal("1E-30")),
        case("sub", Decimal("1.5"), Decimal("1.5")),
        # Negation rounds to the context, and the negation of a zero is positive.
        case("neg", Decimal("1.00499999999999999999999999999")),
        case("neg", Decimal("-1.5")),
        case("neg", Decimal("0")),
        case("neg", Decimal("-0")),
        case("neg", Decimal("-0.00")),
        case("neg", Decimal("9.9999999999999999999999999999E+999999")),
        # Subtraction rounds once: the right operand is not rounded on its own.
        case("sub", Decimal("1"), Decimal("1.00499999999999999999999999999")),
        case("sub", Decimal("0"), Decimal("1.00499999999999999999999999999")),
        case("cents", Decimal("2.505")),
        case("cents", Decimal("-2.505")),
        case("cents", Decimal("99999999999999999999999999.99")),
        case("cents", Decimal("99999999999999999999999999.995")),
        case("cmp", Decimal("1.50"), Decimal("1.5")),
        case("cmp", Decimal("-0"), Decimal("0")),
        # Powers of one are exact for any exponent.
        case("pow", one, 1000000),
        case("pow", -one, 1000001),
        # Subnormal values below 10^Emin keep fewer digits down to Etiny.
        case("mul", Decimal("1e-999999"), Decimal("0.1")),
        case("mul", Decimal("1E-1000000"), Decimal("1e999999")),
        case("mul", Decimal("1e-1000026"), Decimal("0.1")),
        case("div", Decimal("1e-999990"), Decimal("3")),
        # Powers with large exponents and subnormal intermediates, where libmpdec's
        # working precision also sets the smallest exponent.
        case("pow", Decimal("1.22474e-500013"), 2),
        case("pow", Decimal("1.0000001"), -100000),
        case("pow", Decimal("1.0000001"), 123456789),
        case("pow", Decimal("0.999"), 10**15),
        case("pow", Decimal("2"), 3500000),
        case("pow", Decimal("-1.5"), -12345),
        # Intermediates that round to zero stay zero for any further exponent.
        case("pow", Decimal("1e-600000"), 2**62),
        case("pow", Decimal("-1e-600000"), 2**62 + 1),
        case("pow", Decimal("1e-600000"), -(2**62)),
    ]
    for _ in range(600):
        a, b = random_operand(rng), random_operand(rng)
        for op in ("add", "sub", "mul", "div", "cmp"):
            entries.append(case(op, a, b))
    for _ in range(400):
        a, b = random_operand(rng, 28), random_operand(rng, 28)
        entries.append(case(rng.choice(["add", "sub", "mul", "div"]), a, b))
        entries.append(case("cents", random_operand(rng, 30)))
    for _ in range(500):
        # The calculators' patterns: amounts times period rates and their powers.
        r = rate(rng)
        n = rng.randint(1, 600)
        entries.append(case("pow", one + r, -n))
        entries.append(case("pow", one + r, n))
        entries.append(case("mul", amount(rng), r))
        entries.append(case("cents", amount(rng) * r))
        entries.append(case("div", amount(rng), Decimal(rng.randint(1, 600))))
    return entries


def render(entries):
    lines = ["["]
    lines.append(
        ",\n".join("  " + json.dumps(entry, sort_keys=True) for entry in entries)
    )
    lines.append("]")
    return "\n".join(lines) + "\n"


def main():
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    group = parser.add_mutually_exclusive_group(required=True)
    group.add_argument("--output", type=Path, help="write the fixtures to this file")
    group.add_argument("--check", type=Path, help="compare the fixtures in this file")
    args = parser.parse_args()

    # The context Python's Dezimal uses: the default one.
    getcontext().prec = 28
    getcontext().rounding = decimal.ROUND_HALF_EVEN
    rendered = render(cases())
    if args.output:
        args.output.write_text(rendered, encoding="utf-8")
        return 0
    if args.check.read_text(encoding="utf-8") != rendered:
        print(
            f"{args.check} is out of date; regenerate it with --output", file=sys.stderr
        )
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
