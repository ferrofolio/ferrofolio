# Agent guide

Rules for automated coding agents and for humans working with them in this repository.
They apply in addition to [CONTRIBUTING](.github/CONTRIBUTING.md).

## Project

Ferrofolio is a self-hosted personal finance and net worth tracker and an independent fork of
[Finanze](https://github.com/finanze/finanze). The plan for the project, including the migration of the
backend from Python to Rust, is in `docs/roadmap.md`. Read it before starting work that touches architecture.

The backend is being ported to Rust route by route; the Rust server forwards every route that is not ported yet to
the Python backend in the same container. How that works is described in `docs/rust-backend.md`.

This repository contains the server and the web app, which users build and run as two Docker images, the server and
the nginx frontend `ferrofolio-front` (the project does not publish images yet). The web app is becoming a PWA and the
browser is its only runtime. There is no desktop app: do not add Electron, desktop builds or other desktop code. The
Electron login windows for Trade Republic, ING, IBKR, Mintos and Unicaja were removed with it; they remain in the Git
history (last at `890f604e`) as a reference for the server-side sign-in. The native mobile apps live in their own
repositories (`ferrofolio/ios`, later `ferrofolio/android`), not here: do not add Capacitor, Pyodide or native shells.

| Path | Content |
|---|---|
| `finanze/domain` | Types, errors and use case interfaces. No dependencies on other layers |
| `finanze/application` | Use cases and ports. Depends on `domain` only |
| `finanze/infrastructure` | Controllers, repositories, connectors. Implements the ports |
| `crates/core-domain` | Rust port of `finanze/domain`. Depends on no other workspace crate |
| `crates/core-app` | Rust port of `finanze/application`. Depends on `core-domain` only, no other crate |
| `crates/server` | Rust entry point of the container: ported routes, forwarding of all others to Python |
| `frontend/app` | React web app |
| `tests` | Python unit and integration tests |
| `e2e/standard` | Playwright end-to-end tests against the web app |

## Language

Everything is written in English: code, comments, identifiers, documentation, commit messages, pull requests,
issues and review replies. The only exception is the content of locale files under `frontend/app/src/i18n/locales`.

## Workflow

1. Work on a branch created from `develop`. Never push to `develop` or `main` directly and never force-push them.
2. Keep pull requests small: one concern each, as a guideline under 500 changed lines not counting tests and fixtures.
3. Run the checks below, on your machine, before opening a pull request.
4. Open the pull request against `develop` with a description of what changed and why.
5. Address every review comment by fixing it or by explaining why not. Do not resolve a thread that still needs a
   decision from a maintainer.
6. A maintainer merges. An agent never merges its own pull request.

## Commits

- Follow [Conventional Commits](https://www.conventionalcommits.org/): `feat:`, `fix:`, `docs:`, `chore:`, `ci:`,
  `refactor:`, `test:`, with an optional scope such as `feat(i18n):`.
- The subject line states what changes, in the imperative, without a trailing period.
- No trailers of any kind. In particular no `Co-authored-by`, no `Signed-off-by` and no line naming a tool that
  helped produce the change. The same holds for pull request descriptions.

## Checks

The checks are the jobs of `.github/workflows/ci.yml`, which is their only definition. Before every commit that is
meant for a pull request, run them on your machine:

```bash
scripts/ci-local.sh
```

The script runs the workflow with [act](https://github.com/nektos/act) in containers and needs Docker or Podman.
With Podman, its API socket must be running; the script does not start it (`systemctl --user start podman.socket`
on Linux with systemd, `podman machine start` on macOS).
It runs the jobs that CI would run for a pull request to `develop`, or every job when the working tree has
uncommitted changes; `--all` runs every job, `--job <id>` a single one. Every job it runs must pass. A job that
fails only because of the local environment, not the change, is named in the pull request with the reason. To
change a check, change the workflow, never the script.

## Hard rules

- **Money is never a float.** Use `Dezimal` from `finanze/domain/dezimal.py`, in Rust `core_domain::Dezimal`.
  Amounts become JSON numbers only when a response is serialised.
- **Rust code does not panic.** No `unwrap`, `expect`, `panic!` or `unsafe` outside tests; the workspace lints
  enforce it.
- **Ported routes keep their behaviour.** A route moves to Rust together with parity fixtures recorded from the
  Python route. For the calculation routes the cases are written in `tests/parity/generate_calculation_fixtures.py`,
  which records them in `crates/server/tests/fixtures/calculations_parity.json`, the file CI checks. A route of
  another kind gets its own generator under `tests/parity` and fixture file under `crates/server/tests/fixtures`,
  added to the CI checks in the same pull request. A deliberate difference is recorded as a deviation case there
  and in `docs/rust-backend.md`.
- **SQL uses bound parameters.** Never build statements with string formatting.
- **No secrets in output.** Credentials, tokens, session payloads and IBANs must not appear in logs, error messages,
  test fixtures or commits.
- **Migrations are immutable.** Never edit a merged file under `finanze/infrastructure/repository/db/versions`;
  add a new migration with a test instead.
- **Tests are not weakened.** Do not delete tests or loosen assertions to make a change pass. If a test is wrong,
  say so in the pull request.
- **The API stays compatible.** Changes under `/api/v1` are additive.
- **Connectors are read-only.** No call that could move money or place an order.
- **No new upstream dependencies.** Do not add requests to services under `finanze.me`; the project is removing them.
- **Dependencies need a reason.** State in the pull request why a new dependency is necessary.
- **Respect the layers.** `domain` and `application` do not import from `infrastructure`.
- **User-facing text goes through i18n.** A new key must be added to every locale file.

## Areas that need a human decision

Stop and ask before changing behaviour in these areas, and point them out in the pull request:

- Authentication, sessions, CORS
- Cryptography and the backup format
- Database migrations
- Money arithmetic and rounding
- Anything that handles bank or broker credentials
- Licensing, the `NOTICE` file and third-party attributions
