# Developer guide

How to run Ferrofolio from source, format the code and run the tests. Read [CONTRIBUTING](../.github/CONTRIBUTING.md)
and [AGENTS.md](../AGENTS.md) for the rules of the project, and the [roadmap](roadmap.md) before work that touches the
architecture.

## Repository layout

| Path | Content |
|---|---|
| `finanze/` | Python backend: `domain`, `application` and `infrastructure` layers |
| `crates/` | Rust backend, which replaces the Python backend route by route, see [rust-backend.md](rust-backend.md) |
| `frontend/app/` | React web app |
| `tests/` | Python unit and integration tests, and the generators of the Rust parity fixtures |
| `e2e/standard/` | Playwright end-to-end tests against the web app |

## Requirements

- Python 3.14
- Rust; `rustup` installs the version pinned in `rust-toolchain.toml`
- Node 24 and `pnpm` for the frontend

## Setup

1. Clone the repository:

   ```sh
   git clone https://github.com/ferrofolio/ferrofolio.git
   cd ferrofolio
   ```

2. Create a virtual environment and activate it (pyenv is recommended to manage Python versions):

   ```sh
   python3 -m venv venv
   source venv/bin/activate
   ```

3. Install the dependencies and the Git hooks:

   ```sh
   pip install -r requirements.txt -r requirements-dev.txt -r requirements-lint.txt
   pip install -r requirements-selenium.txt  # only for Mintos, which needs Selenium for its reCAPTCHA
   pre-commit install
   ```

4. Install the frontend dependencies:

   ```sh
   cd frontend/app
   pnpm install
   cd ../..
   ```

## Run

Start the backend. It stays in the foreground:

```sh
python ./finanze --port 7592 --data-dir .storage --log-dir .storage/logs --log-level DEBUG --third-party-log-level DEBUG
```

In a second terminal, start the web app and point it at the backend. It is served at <http://localhost:5173>:

```sh
cd frontend/app
VITE_BASE_URL=http://localhost:7592 pnpm dev
```

To run the Rust server in front of the Python backend, as the Docker image does, see
[Running locally](rust-backend.md#running-locally) in the Rust backend guide.

The environment variables the server reads are listed in the [setup guide](self-hosting.md#settings).

## Format and lint

```sh
ruff format
ruff check

cargo fmt --all
cargo clippy --workspace --all-targets -- -D warnings

cd frontend/app
pnpm format
pnpm lint
```

The `Lint` job of CI also scans every pull request for secrets with gitleaks, lints the workflows with actionlint
(including shellcheck on their scripts) and the Dockerfiles with hadolint; `scripts/ci-local.sh --job lint` runs it
alone. A value that gitleaks takes for a secret but that is none, such as a public client identifier of a web app,
gets a `# gitleaks:allow` comment at the end of its line with a word on why. The rules that hadolint and actionlint
skip, and why, are in `.hadolint.yaml` and `.github/actionlint.yaml`.

## Test

```sh
pytest                   # Python backend
cargo test --workspace   # Rust backend

cd frontend/app
pnpm test                # Frontend

cd e2e/standard
pnpm test                # End-to-end tests in the browser
```

## CI checks

The checks are the jobs of [`.github/workflows/ci.yml`](../.github/workflows/ci.yml). Run them on your machine before
opening a pull request:

```sh
scripts/ci-local.sh
```

The script runs the workflow with [act](https://github.com/nektos/act) in containers and needs Docker or Podman. With
Podman, its API socket must be running (`systemctl --user start podman.socket` on Linux with systemd,
`podman machine start` on macOS). It runs the jobs that CI would run for a pull request to `develop`, or every job
when the working tree has uncommitted changes; `--all` runs every job and `--job <id>` a single one.
