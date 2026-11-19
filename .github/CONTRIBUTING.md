# Contributing to Ferrofolio

Thanks for your interest in contributing to **Ferrofolio**, the self-hosted personal finance and net worth tracker!

Ferrofolio is an independent fork of [Finanze](https://github.com/finanze/finanze). All project communication, code comments, commit messages and documentation are written in English.

## Getting Started

See the [developer guide](../docs/development.md) for complete setup instructions (backend and frontend), and the
[agent guide](../AGENTS.md) for the rules every change follows.

## Ways to Contribute

- **Report bugs:** Use the [bug report template](ISSUE_TEMPLATE/bug_report.md).
- **Request features:** Use the [feature request template](ISSUE_TEMPLATE/feature_request.md).
- **Add connectors:** Add support for new banks, financial entities, crypto exchanges, or blockchain networks.
- **Improve codebase:** Submit fixes, refactors, and performance improvements via Pull Requests.

---

## Licensing of Contributions

Ferrofolio is licensed under the [AGPL-3.0](../LICENSE). Contributions are accepted under the same license
("inbound = outbound"): by opening a pull request you agree that your contribution is licensed under the AGPL-3.0.

There is **no contributor license agreement** and no sign-off requirement. You keep the copyright in your work and
grant nobody the right to relicense it.

---

## Pull Request Guidelines

1. **Branching:** Fork the repository and create a descriptive branch from `develop` (e.g., `feature/super-cool` or `bugfix/sync-error`).
2. **Code Style & Formatting:**
   - **Backend:** Run `ruff format` and `ruff check`.
   - **Rust:** Run `cargo fmt --all` and `cargo clippy --workspace --all-targets -- -D warnings`.
   - **Frontend:** Run `pnpm format` and `pnpm lint`.
   - More on this in the [developer guide](../docs/development.md#format-and-lint).
3. **Testing:**
   - Add unit tests or integration tests for new features and infrastructure adapters in `tests/` and `e2e/` if possible.
   - Ensure all tests pass: `pytest` (Python backend), `cargo test --workspace` (Rust backend) and `pnpm test` (frontend).
   - `scripts/ci-local.sh` runs the CI checks that apply to your change locally in containers, `--all` runs every one (needs Docker or Podman).
   - More on this in the [developer guide](../docs/development.md#test).
4. **Submit:** Open the Pull Request against the `develop` branch with a clear title and description of your changes.

---

## Reporting Security Issues

Please **do not** open public GitHub issues for security vulnerabilities. Instead, follow our [Security Policy](SECURITY.md) to report them responsibly.
