# Rust backend

The backend moves from Python to Rust one route at a time (roadmap, section 5). This document describes the state of
that port and the rules for continuing it.

## Crates

| Crate | Python counterpart | Content |
|---|---|---|
| `core-domain` | `finanze/domain` | `Dezimal`, domain types and errors as they are needed |

`Dezimal` reproduces Python's `decimal` arithmetic in its default context: every result of `+ - * /` is the exact
result rounded to 28 significant digits with `ROUND_HALF_EVEN`, and integer powers follow libmpdec. Values below
10^-999999 become subnormal and keep fewer digits, down to the smallest one, 10^-1000026; a result of at most half
of that rounds to zero, as in Python. A decimal with a bounded scale such as
`rust_decimal` (at most 28 decimal places and a 96-bit coefficient) cannot reproduce 28 significant digits in
general: `150 × (0.01 / 12)` is 0.125 in Python, rounded to cents with `ROUND_HALF_UP` that is 0.13, while
`rust_decimal` cut `0.01 / 12` at 28 decimal places, got a product just below 0.125 and rounded it to 0.12.
Its operators return a `Checked` value that carries a decimal signal (overflow, division by zero, invalid
operation) through a whole formula instead of panicking; the formula is unwrapped once with `value()`. Amounts
become `f64` only when a response is serialised, because the API carries them as JSON numbers.

Amounts are parsed exactly from their text. Strings follow Python's `Decimal(str)`: surrounding whitespace, every
underscore and digits of any script are accepted. Amounts keep up to 38 significant digits; a longer one is rejected
rather than rounded, because rounding it could change a cent.

## Parity tests

`tests/parity/generate_decimal_fixtures.py` records thousands of operations from Python's `decimal`, and
`crates/core-domain/tests/decimal_parity.rs` requires the same results. The table of Unicode digits that the parser
accepts is generated from Python's Unicode database by `tests/parity/generate_unicode_tables.py`. CI runs both
generators with `--check` in the Python job, so neither can drift from the Python version the backend uses.

```sh
python tests/parity/generate_decimal_fixtures.py --output crates/core-domain/tests/fixtures/decimal_parity.json
python tests/parity/generate_unicode_tables.py --write
```

## Next steps

1. `server`: the Rust binary becomes the entry point of the server image and forwards every route to Python.
2. The calculators as the first ported routes, with parity fixtures recorded from the Python routes.
3. `infra-db`: open existing SQLCipher databases with `rusqlite` and read the migration state.
