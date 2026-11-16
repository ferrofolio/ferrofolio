# Agent guide

Rules for automated coding agents and for humans working with them in this repository.
They apply in addition to [CONTRIBUTING](.github/CONTRIBUTING.md).

## Project

Ferrofolio is a self-hosted personal finance and net worth tracker and an independent fork of
[Finanze](https://github.com/finanze/finanze). The plan for the project, including the migration of the
backend from Python to Rust, is in `docs/roadmap.md`. Read it before starting work that touches architecture.

| Path | Content |
|---|---|
| `finanze/domain` | Types, errors and use case interfaces. No dependencies on other layers |
| `finanze/application` | Use cases and ports. Depends on `domain` only |
| `finanze/infrastructure` | Controllers, repositories, connectors. Implements the ports |
| `frontend/app` | React web app |
| `tests` | Python unit and integration tests |
| `e2e/standard` | Playwright end-to-end tests against the web app |

## Language

Everything is written in English: code, comments, identifiers, documentation, commit messages, pull requests,
issues and review replies. The only exception is the content of locale files under `frontend/app/src/i18n/locales`.

## Workflow

1. Work on a branch created from `develop`. Never push to `develop` or `main` directly and never force-push them.
2. Keep pull requests small: one concern each, as a guideline under 500 changed lines not counting tests and fixtures.
3. Run the checks below before opening a pull request.
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

```bash
# Backend
ruff check .
ruff format --check .
pytest

# Frontend
cd frontend/app
pnpm run lint
pnpm exec prettier --check .
pnpm test

# End to end
cd e2e/standard
pnpm test
```

## Hard rules

- **Money is never a float.** Use `Dezimal` from `finanze/domain/dezimal.py`.
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
