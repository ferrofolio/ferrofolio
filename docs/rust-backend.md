# Rust backend

The backend moves from Python to Rust one route at a time (roadmap, section 5). This document describes the state of
that port and the rules for continuing it.

## Request flow

```mermaid
flowchart LR
  C[Browser or app] -->|port 7592| R[Rust server<br/>crates/server]
  R -->|ported routes| U[Rust use cases<br/>crates/core-app]
  R -->|everything else,<br/>unchanged| P[Python backend<br/>127.0.0.1, internal port]
```

The `ferrofolio` binary is the entry point of the server image. It first takes the public port, then binds a loopback
socket for the Python backend, starts Python as a child process that serves that socket (`--socket-fd`), and waits
until Python has answered an HTTP request on it. Only then does it listen on the public port; connections made
earlier are refused, as by the Python server. Because the gateway created the backend socket and only the child holds it, no other process can answer
in Python's place.

- **Forwarding.** Method, path, query, body and headers, including `Host`, `Authorization` and `X-Forwarded-*`, reach
  Python unchanged, except that the token of a valid session becomes the backend token and a login asks for a lasting
  one (see [Sessions](#sessions)). Hop-by-hop headers are removed and the client address is appended to `X-Forwarded-For`, which
  uvicorn trusts from the loopback interface; a body without a known length is sent on with chunked framing, whatever
  the method. Responses are streamed, so long fetches, downloads and the MCP endpoint
  keep working. If Python cannot be reached the server answers `502` with the code `BACKEND_UNAVAILABLE`.
- **Lifecycle.** `SIGTERM` or `SIGINT` stops the public server, gives open requests three seconds, then sends
  `SIGTERM` to Python and kills it after six more seconds, which stays within Docker's default stop timeout of ten
  seconds. If Python exits on its own, the server exits with an error; with a
  restart policy such as the `unless-stopped` of the example compose file, Docker starts both again, otherwise the
  container stays stopped until someone restarts it.
- **Options.** The server accepts the options of `finanze/args.py` and passes them on, except that `--port` cannot be
  `0`: Python builds redirect URLs with that port. `--log-level NONE` silences the server's console output entirely.
  Python is started with
  the inherited socket, `--host 127.0.0.1`, its port and `--public-port <public port>`, which keeps the redirect URLs it builds for
  GoCardless and the ChatGPT sign-in on the public port. Without `--data-dir`, the server takes Python's default,
  `appdirs.user_data_dir("Finanze")` (`$XDG_DATA_HOME/Finanze`, else `~/.local/share/Finanze`, on macOS
  `~/Library/Application Support/Finanze`), and passes it on, so that both use the same directory. Additional
  settings:

| Variable / option | Default | Meaning |
|---|---|---|
| `FERROFOLIO_BACKEND_PORT` / `--backend-port` | `0` (a free port) | Loopback port for the Python backend |
| `FERROFOLIO_PYTHON` / `--python` | `python3` | Interpreter that starts the backend |
| `FERROFOLIO_BACKEND_APP` / `--backend-app` | `finanze` | Path of the Python package |

## Crates

| Crate | Python counterpart | Content |
|---|---|---|
| `core-domain` | `finanze/domain` | `Dezimal`, calendar arithmetic with `dateutil.relativedelta` semantics, domain types and errors as they are needed |
| `core-app` | `finanze/application` | Ports such as `Clock` and `LoanCalculatorPort`, use cases, the loan calculator |
| `infra-db` | `finanze/infrastructure/repository/db` | Opens the encrypted database read-only and reads its migration state |
| `server` | `finanze/infrastructure/controller`, `server.py`, `infrastructure/api_session`, `infrastructure/user_files` | Forwarding, process management, API sessions, unlocking the database at a login, ported routes |

`Dezimal` reproduces Python's `decimal` arithmetic in its default context: every result of `+ - * /` is the exact
result rounded to 28 significant digits with `ROUND_HALF_EVEN`, and integer powers follow libmpdec. Values below
10^-999999 become subnormal and keep fewer digits, down to the smallest one, 10^-1000026; a result of at most half
of that rounds to zero, as in Python. A decimal with a bounded scale such as
`rust_decimal` (at most 28 decimal places and a 96-bit coefficient) cannot reproduce 28 significant digits in
general: `150 × (0.01 / 12)` is 0.125 in Python, rounded to cents with `ROUND_HALF_UP` that is 0.13, while `rust_decimal` cut
`0.01 / 12` at 28 decimal places, got a product just below 0.125 and rounded it to 0.12.
`tests/parity/generate_decimal_fixtures.py` records
thousands of operations from Python, and `crates/core-domain/tests/decimal_parity.rs` requires the same results.
Its operators return a `Checked` value that carries a decimal signal (overflow, division by zero, invalid
operation) through a whole formula instead of panicking; the formula is unwrapped once with `value()`. Amounts
become `f64` only in the response types of the server, because the API carries them as JSON numbers; requests never
pass an amount through a float for arithmetic.

Amounts in requests are parsed exactly from their text. Strings follow Python's `Decimal(str)`: surrounding
whitespace, every underscore and digits of any script are accepted. JSON numbers are read from their digits, with or
without a fraction or an exponent; Python reads a JSON number with a fraction or an exponent as a float first, so
the two differ whenever that conversion changes the value, for example `9007199254740993.0`, `9007199254740993e0` or
a number with more than 17 significant digits: a recorded deviation. Amounts keep up to 38 significant digits;
a longer one is rejected rather than rounded, because rounding it could change a cent. One test reads a float on
purpose: whether a savings scenario has a `target_amount` follows Python's truth test of the JSON value, which reads
a number with a fraction or an exponent as a float, so a value that underflows to zero, such as `1e-400`, means no
target in both. The amount of a target that is kept is parsed exactly.

## Ported routes

| Route | Python origin | Since |
|---|---|---|
| `POST /api/v1/calculation/loan` | `routes/calculate_loan.py`, `calculations/loan_calculator.py` | Calculators (phase 4, slice 8, pulled forward) |
| `POST /api/v1/calculations/savings` | `routes/calculate_savings.py`, `use_cases/calculate_savings.py` | Calculators |

The list in code is `PORTED_ROUTES` in `crates/server/src/app.rs`; `crates/server/ported_routes.json` repeats it for
`tests/unit/infrastructure/controller/test_ported_routes.py`, which checks that Python still declares every ported
route, and a Rust test keeps the two lists equal. Other methods on these paths, in particular CORS preflight
requests, still go to Python, which keeps the routes until the cutover. Python decodes percent-escapes in the path,
collapses leading slashes and ignores the case of the method before routing, so Rust serves a ported route also when
it is spelled `/api/v1/calculation/%6Coan`, `//api/v1/calculation/loan`, `/%2Fapi/v1/calculation/loan` or with a
lowercase `post`, with the same limits; requests it forwards keep the method and path as the client sent them. Responses of ported routes carry the CORS headers quart-cors would add.

Calculations run on the blocking thread pool, at most one per core; further requests wait up to 30 seconds and are
then refused with `503 SERVER_BUSY` and `Retry-After: 5`. Within the input limits the slowest request found, 50
savings scenarios solving a 200-year retirement at a negative performance, holds its core for about four seconds. A
request body of a
ported route may have at most 1 MiB (a deviation: Python accepts 50 MB). Bodies count against a budget of 64 MiB
while they are read and decoded, so that waiting requests cannot hold an unbounded amount of memory; a body without
`Content-Length` counts with the largest size. One deadline of 60 seconds covers the wait for the budget and the
read. On shutdown, calculations still running get half a second after the server has stopped.

Startup keeps Python's behaviour of refusing connections until the backend is ready: the public address is bound
first but listens only then. Two servers started at the same time on the same address can therefore both start
their backend before the second fails to listen, as two Python servers could before; separate containers do not
share addresses.

## Sessions

Rust holds the API sessions (phase 4, slice 1). It issues and checks the tokens clients use, as Python's
`InMemoryApiSessionStore` did: 32 random bytes in URL-safe Base64, kept only as their SHA-256 digest, valid until the
next logout, or for a remembered session until `FINANZE_API_SESSION_REMEMBER_DAYS` days have passed.
`FINANZE_API_SESSION` and `FINANZE_API_SESSION_REMEMBER_DAYS` are read by both processes from the same environment,
with the rules of `finanze/args.py`; with sessions off, Rust keeps none and passes login, signup and logout on
unchanged. Python keeps its session guard, its login and its logout, and stays the second line of defence:

```mermaid
sequenceDiagram
  participant C as Client
  participant R as Rust
  participant P as Python
  C->>R: POST /api/v1/login with username, password, remember
  R->>P: the same body, with remember false appended
  P-->>R: 200 with the backend token
  R-->>C: 200 with a new client token, not to be cached
  C->>R: GET /api/v1/positions with the client token
  R->>P: GET /api/v1/positions with the backend token
```

- **Login and signup.** When Python answers `200` with its token, Rust keeps that token as the backend token, starts
  a session and answers with a token of its own; Python's token never reaches a client. A `200` without a token is
  answered with `502` rather than passed on. Every other answer, such as `204` for a guest's signup or an error, passes
  through and starts no session. Rust reads the body to do this, with the limits and the body budget of the ported
  routes. A logout that ends the sessions while a login is with Python may have ended the backend token as well, so
  that login starts no session and is answered with `500 UNEXPECTED_ERROR`; the client logs in again.
- **The backend token lasts until the logout.** Rust appends `"remember": false` to the JSON object of a login, and
  Python's `json` module keeps the last of repeated keys; the rest of the body reaches Python unchanged. Rust applies
  the client's `remember` to its own session, so the backend token outlives the session. Each session keeps the backend
  token of its own login and never uses a later one, so a session that Python has ended stays ended there, whatever
  logins follow. A body Rust cannot read as a JSON object goes on unchanged, so that Python answers it as
  before; should Python accept it, Rust starts no session and answers `400 INVALID_REQUEST`.
- **Requests.** A request with the token of a valid session reaches Python with `Authorization: Bearer` and the
  backend token instead. Every other request reaches Python as the client sent it, so Python's session guard and its
  status route decide about it as before: with a token Python does not know, the data counts as locked for the
  status, and a guarded route answers `401 NOT_LOGGED` while the data is unlocked. The token of the MCP endpoint,
  which Python checks itself, passes through untouched.
- **Logout.** Rust passes it on, with the backend token for a valid session. Python's logout handler ends its sessions
  before anything else, so they may be gone whenever the request reached it, and a session Rust kept would come back
  with the backend token of the next login. A logout with a valid session carries the backend token, which Python's
  guard accepts, so Rust ends every session and forgets the backend token before it passes the logout on; a login
  still with Python then starts no session either. Any other logout ends them once Python has answered, unless it
  refused with a `3xx` or `4xx`, such as the `401` of its guard while the data is unlocked: on success, on a server
  error and when the connection fails. Such a logout runs to its end even when the client goes away. Python only lets
  it through while the data is locked; until it answers, Rust's sessions still pass the ported routes, while their
  backend tokens, which Python has ended, are refused on every forwarded route.
- **Unlocking.** Once Python has accepted a login or signup, with or without a token, and unless it is a guest's
  signup, Rust opens the user's database itself, read-only, with the password from the request: it finds the user's
  id in `profiles.json` in the data directory, as Python's `get_user` does, and opens `profiles/<id>/data.db` with
  `infra-db` (see [Database](#database)). Python has migrated the database before it answered. Rust also opens it
  with the credentials of Python's automatic login at startup (`USERNAME` and `PASSWORD`), and, without API sessions,
  at a login Python refuses with `409` because the data is already unlocked, when the password opens the database.
  The logout that ends the sessions closes it, and a login that a logout overtakes keeps it closed. If Rust cannot
  open it, for example because the profile is missing, the login still succeeds. Rust keeps the credentials while the
  database is open, as Python's `DBManager` keeps the password: a backup import replaces `data.db` with a new file,
  and a reader then opens the new one (`UserDatabase::current`) instead of reading the old one.
- **Ported routes** need the data unlocked. With API sessions they check the session in Rust, which exists only
  between a login and the logout. Without them they still ask Python, which also unlocks and locks the data without a
  login Rust sees (the automatic login, a guest's backup import, a failed restore): they send `GET /api/v1/status` and
  proceed only on `UNLOCKED`, answer `502` when Python cannot be reached or its answer is not a readable status, and
  `504` when it does not answer within 30 seconds.

Rust learns of an unlock and a lock only through the login, signup and logout it passes on, and the automatic login.
Python can also lock the data on its own, when a backup import fails after it has closed the database, and unlock it,
when a guest imports a backup from the cloud. In the first case the sessions stay valid in both processes, as they
did in Python, and ported routes keep accepting them; in the second the client has no session in either, and logs in.
Without API sessions such a login is refused with `409`, and Rust opens the database with its password. The
database Rust holds matters only to routes that read data, none of which is ported yet; a route that needs it answers
as for locked data while Rust has none open.

## Parity tests

A route moves to Rust together with fixtures recorded from the Python route.
`tests/parity/generate_calculation_fixtures.py` sends hand-written and seeded random requests through the real Quart
routes and the production session guard, with a fixed date, and writes the responses to
`crates/server/tests/fixtures/calculations_parity.json`.
`crates/server/tests/parity_calculations.rs` replays every case against the Rust router. CI runs the generator with
`--check` in the Python job, so the fixtures cannot drift from the Python code.

Every generator under `tests/parity` runs only on the CPython version CI uses, which it reads from
`.github/workflows/ci.yml`, and stops with a message on any other: error messages and the Unicode database differ
between versions, and a fixture recorded with another one would not match what CI records.

Regenerate the fixtures after changing a Python calculator:

```sh
python tests/parity/generate_calculation_fixtures.py --output crates/server/tests/fixtures/calculations_parity.json
```

Each case states what it compares:

| Mode | Rust must return |
|---|---|
| `full` | The status and the body of Python's response; numbers compare by value |
| `status_code` | The status and the `code` field; used where Python's message comes from pydantic |
| `status` | The status only; available for answers that are not JSON, no case uses it at present |
| `deviation` | The status and code recorded with the reason for the deviation, or, for a case with an `equivalent_request`, Python's status and body for that request as Rust reads it (stored in `deviation.body`); the fixture keeps a digest of Python's body so that `--check` notices when Python's answer changes |

### Known deviations

| Input | Python | Rust |
|---|---|---|
| A period rate of exactly −100 % | Computes with `Decimal` infinities, `0 ** -n` is `Infinity` | `500 UNEXPECTED_ERROR`, division by zero |
| `NaN` or `Infinity` as an amount | Accepted by `Decimal`; the loan calculator rejects it with `400 INVALID_REQUEST` (L12), the savings calculator fails later | `400 INVALID_REQUEST` when the request is read |
| An amount with more than 38 significant digits, in any form | Kept exactly as a string or integer, rounded to a float as a number with a fraction or an exponent | `400 INVALID_REQUEST` |
| A JSON number with a fraction or an exponent whose value a float cannot hold exactly, up to 38 significant digits | Rounded to a float first | Kept exactly |
| A request body above 1 MiB | Accepted up to 50 MB | `413 PAYLOAD_TOO_LARGE`; a decoded JSON value takes many times the memory of its text |
| An object whose first key is `$serde_json::private::Number`, serde_json's private token for exact numbers | An unknown field, ignored | `400 INVALID_REQUEST` |
| A request body that is JSON but neither an object nor empty (for example `[1]`), or a `retirement` that is neither an object nor `null` | `500` | `400 INVALID_REQUEST` |
| Malformed JSON | `400` with an HTML page | `400 INVALID_REQUEST` as JSON |
| The literals `NaN`, `Infinity` and `-Infinity`, which are not JSON | Accepted by the `json` module; the loan calculator rejects them with `400`, as not finite | `400 INVALID_REQUEST`, the body is not JSON |
| An integer field beyond the float range, such as `"years": 1e400` | Read as infinity, fails with `500` | `400 INVALID_REQUEST` with Python's message |
| `fixed_years` beyond 64 bits | Accepted; the fixed period never ends | `400 INVALID_REQUEST` |
| `withdrawal_years` beyond 64 bits | Computed; without a `withdrawal_amount` the withdrawal loop runs for that many periods and does not finish in practice | `400 INVALID_REQUEST` |
| An amount whose exponent is beyond 32 bits, such as `"1e-2147483649"` | Accepted by `Decimal`, which takes any exponent | `400 INVALID_REQUEST` |
| A number in the response beyond the float range, such as a rate of `1e400` echoed back when no amount has to be rounded | `200` with `Infinity`, which is not JSON | `500 UNEXPECTED_ERROR` |
| Messages of pydantic validation errors | Full pydantic text | The first lines of it |
| More than 200 years of accumulation or retirement, or more than 50 scenarios (savings) | Computed, however long it takes and however much memory it needs | `400 INVALID_REQUEST` |
| A calculation that finds every core busy for 30 seconds (no fixture case, which would need concurrent requests; `tests/gateway.rs` covers it) | Waits; a calculation blocks Python's event loop and every other request with it | `503 SERVER_BUSY` with `Retry-After: 5` |
| A request body nested deeper than 128 levels, also in a field the route ignores | Accepted | `400 INVALID_REQUEST`; serde_json's recursion limit keeps a crafted body from exhausting the stack |
| A ported route while the database is locked, with or without a session | Allowed: the session guard lets everything through while locked | `401 NOT_LOGGED` (fixture cases "no session while the data is locked" and "session while the data is locked"; Rust holds no session while the data is locked, so the second case sends the token of a session a logout has ended) |
| A login or signup body above 1 MiB | Accepted up to 50 MB | `413 PAYLOAD_TOO_LARGE`; Rust reads the body to ask for a lasting backend token |
| The end of a remembered session | The remember period is added to the local wall-clock time, so a change to or from daylight saving time in between moves the end by the size of the change | Exactly that many days of 24 hours after the login |
| A login body Python reads but serde_json does not, such as one with `NaN`, a lone surrogate escape or in UTF-16 | Logged in | `400 INVALID_REQUEST` and no session, as Rust cannot ask for a lasting backend token; Python has unlocked the data |
| A `remember` that is an object whose first key is serde_json's private number token | True, a non-empty dict | Read as the number, so `{"$serde_json::private::Number": "0"}` is false and the session lasts until the logout, as with `false` |
| A logout whose answer does not arrive, for example because the connection to Python broke | Its sessions end if the handler ran | A logout with a valid session has ended Rust's sessions before it was passed on; any other ends them when the connection fails, or when a late answer arrives that is not a `3xx` or `4xx` refusal. If the request never reached the handler, Python's data stays unlocked until the next logout |
| A logout without a session that Python, having locked the data on its own, lets through and answers late | Its sessions have ended | Until the answer, Rust's sessions still pass the ported routes, which read no data; forwarded requests carry their own, ended backend tokens and are refused by Python |
| A login that Python answers with `200` but without a token | Passed on | `502 BACKEND_UNAVAILABLE`; the answer is not passed on |
| A login that is still with Python when a logout ends the sessions | Whichever Python handles last decides | `500 UNEXPECTED_ERROR`, no session |

### Weaknesses of the Python reference

Reviews of the port found weaknesses in the Python calculators. They are fixed in Python and Rust together, with
regenerated fixtures, and each pull request points the change out to a maintainer, because it changes money
arithmetic (AGENTS.md). A weakness found later is reproduced in Rust until it is fixed, marked with a comment at the
place and listed here, because a ported route keeps its behaviour.

The loan schedule in one sentence: the loan is disbursed on `start`, installment `i` is due `i` periods later (counted
from `start` each time), the last one on or before `end`, and only a loan that ends before its first period is over
has its single installment on `end`; the installments due before today are paid, and the one due
today or next is the current installment. After the last due date that last installment stays the current one, as
before, and a loan whose outstanding is computed from its amount or a tracking reference is repaid: outstanding,
payment and interests are zero, whenever the reference was taken. An outstanding given in the request is still owed
then and due in full, with the last period's interest, also when the loan amount is given too.

| | Where | Fix |
|---|---|---|
| L1 | Loan, outstanding simulated from the start | The simulation applied one installment more than were due, from a schedule that stepped from date to date, so variable and mixed loans reported a lower outstanding than a fixed loan at the same rate. It now applies exactly the installments due before today, each over the installments left, the last one included |
| L2 | Loan, monthly to yearly schedules | Periods were counted in calendar months while the schedule stepped from date to date, so a start on the 31st drifted to the 28th and could count one installment less. Every due date is now counted from the start (31 January, 28 February, 31 March) |
| L3 | Loan, fixed rate with a known amount | Which installments counted as paid depended on how the next installment date had been reached. The installments due before today are paid; the one due today is the current one |
| L4 | Loan, remaining balance | With inconsistent counts the balance could become zero while installments were still unpaid. All paths now count the same installments, so inconsistent counts no longer mark a balance repaid early; rounding small installments to cents can still bring it to zero before the last one |
| L12 | Loan, amounts and rates that are not finite | `NaN` or `Infinity` as an amount or a rate failed later with `500`, or, for a loan past its last installment, answered `200` with zeros. Python now rejects them with `400 INVALID_REQUEST`; Rust never reads them as numbers |
| L5 | Loan, schedule | The loan had an installment on `start` in one place and none in another, with the annuity spread over one installment fewer than the schedule had. The loan is disbursed on `start`; installment `i` is due `i` periods later, the last one on or before `end` |
| L6 | Loan, dates | An `end` before `start` was accepted and gave an installment after the end of the loan; it is now rejected with `400 INVALID_REQUEST`, and so is a negative `fixed_years`, whose fixed period would end before the calendar begins |
| L7 | Loan, end of the calendar | Counting and stepping the schedule went one period past `end`, so a schedule ending at the end of the calendar, or a fixed period of a mixed loan that ends beyond it, failed with `500`; the schedule now takes no step beyond 31 December 9999, and such a fixed period never ends |
| L8 | Loan, request dates | A date that was neither a string nor `null` failed with `500`; it is now rejected with `400 INVALID_REQUEST`, as by the Rust route |
| L9 | Loan, mixed rate | The current installment took the rate in force today, the simulation of the paid installments the rate in force when each period began, so the installment due on the day the fixed period ended paid a different rate than the one applied to it the day after. Every installment now pays the rate in force when its period began |
| L10 | Loan, after the last installment | The last installment never counted as paid, so a loan that had ended still reported the principal of its last installment as outstanding. Once every installment is past, a loan computed from its amount or a tracking reference reports zero outstanding, payment and interests, and an outstanding given in the request is due in full on every path. The tracked-loan refresh, which skipped matured loans, now records that a fixed loan tracked from a reference is repaid |
| L11 | Loan, mixed rate at the end of the calendar | A fixed period ending beyond the calendar was treated as ending on 31 December 9999, so a period beginning on that day paid the floating rate. Such a fixed period never ends |
| S1 | Savings, contribution solved for a retirement | The search counted a withdrawal the balance could not cover as paid. It now counts every withdrawal that is not funded in full, widens its upper bound until that bound funds all of them, stops at once when the base alone does, and returns the smallest contribution in cents with which the projection funds every withdrawal |
| S2 | Savings, contribution solved for a target | The solved contribution used the factor of an ordinary annuity, while the projection lets each contribution earn its period's return, so the projection missed the target by about one period's return. It now uses the factor of an annuity due and, searching from that estimate in both directions, returns the smallest contribution in cents with which the projection reaches the target, also when the base alone would reach it without rounding its returns |
| S3 | Savings, horizon solved for a target | The search applied unrounded returns while the projection rounds them to cents, so the projection could fall short of the target. The search now rounds the same way; a target the rounded returns never reach has no horizon |
| S4 | Savings, retirement | A negative `withdrawal_amount` added to the balance; it is now rejected with `400 INVALID_REQUEST` |
| S5 | Savings, contributions solved for a target or a retirement | A rate too small to change `1 + rate` made an annuity factor zero and the request fail with `500`; such a rate now counts as zero |

## Database

The Python backend keeps each user's data in one SQLCipher database (`data.db` in the user's directory), encrypted
with the user's password, and owns its schema and migrations until the cutover (roadmap, section 5). `infra-db` opens
it read-only and unlocks it as Python does, with SQLCipher's `PRAGMA key` and the password as passphrase; a wrong
password is reported as a decryption error without the password in the message. It reads the applied migrations from
the `migrations` table. SQLCipher is compiled from the sources in `sqlcipher-src` and, on Linux, uses the system's
OpenSSL `libcrypto` for its ciphers, hashes and random numbers: building needs the OpenSSL headers (`libssl-dev` on
Debian and Ubuntu, which GitHub's runner has), and the binary loads `libcrypto.so.3` at run time, which the server
image's Python base already contains; the image's Rust build stage installs `libssl-dev` and `pkg-config`. The
server opens a user's database with it at a login (see [Sessions](#sessions)) while Python keeps its own connection
open; Python runs the database in WAL mode, so the read-only connection reads alongside it. The fixture test checks that this build opens a database written by the Python backend's `sqlcipher3`
with the same password.

`tests/parity/generate_db_fixture.py` creates `crates/infra-db/tests/fixtures/data.db` with the Python backend's
`DBManager`, every migration applied, and records the migrations in `migrations.json`; the Rust test opens the same
file with the same password. SQLCipher salts every database, so the file differs on each run: `--check`, which CI runs,
opens it with Python and compares its migrations, version and name, with `migrations.json`, and their names with the
migrations of the current version; it does not compare the SQL of the migrations.
After adding a migration, regenerate it:

```sh
python tests/parity/generate_db_fixture.py --output crates/infra-db/tests/fixtures
```

## Running locally

```sh
cargo run --bin ferrofolio -- --port 7592 --data-dir .storage --log-dir .storage/logs --log-level DEBUG
```

The server starts `python3 finanze` from the repository root; set `FERROFOLIO_PYTHON` to the interpreter of the
virtual environment if needed.

## Next steps

1. Phase 4, slice 1, continued: Rust holds the sessions and opens the database at a login. Next it serves the status
   and then the settings, which it reads from the database.
2. Read paths (positions, transactions, timelines), each with parity fixtures from a fixture database.
