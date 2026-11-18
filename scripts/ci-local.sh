#!/usr/bin/env bash
# SPDX-License-Identifier: AGPL-3.0-only
#
# Runs the jobs of .github/workflows/ci.yml on this machine, in containers, with act
# (https://github.com/nektos/act). The workflow stays the only definition of the
# checks: this script adds no commands of its own.
#
# Usage: scripts/ci-local.sh [--base REF] [--all] [act options...]
#
#   --base REF  The branch the pull request goes to (default: develop). Jobs run as in
#               a pull request from the current commit to REF, so the path filters of
#               the "changes" job decide which jobs run.
#   --all       Run every job, whatever changed.
#   Further options go to act, for example "--job rust" to run a single job.
#
# Uncommitted changes are included: act copies the working tree into the containers.
# Needs Docker or Podman; act itself is downloaded once, checked against a pinned
# SHA-256 digest and kept in ${XDG_CACHE_HOME:-~/.cache}/ferrofolio.

set -euo pipefail

ACT_VERSION="0.2.96"
ACT_SHA256="b861e901c9062d841c6e8ecd36cdf27e0ce39ea0124cfd93de6efcebea0e16e7"
RUNNER_IMAGE="ghcr.io/catthehacker/ubuntu:act-24.04"

repo_root="$(git rev-parse --show-toplevel)"
cd "$repo_root"

base="develop"
all=false
act_args=()
while [[ $# -gt 0 ]]; do
  case "$1" in
    --base)
      base="${2:?--base needs a branch}"
      shift 2
      ;;
    --all)
      all=true
      shift
      ;;
    *)
      act_args+=("$1")
      shift
      ;;
  esac
done

cache_dir="${XDG_CACHE_HOME:-$HOME/.cache}/ferrofolio"
act_bin="$cache_dir/act-$ACT_VERSION/act"
if [[ ! -x "$act_bin" ]]; then
  mkdir -p "$(dirname "$act_bin")"
  archive="$(mktemp)"
  trap 'rm -f "$archive"' EXIT
  echo "Downloading act $ACT_VERSION"
  curl --fail --silent --show-error --location --max-time 300 --output "$archive" \
    "https://github.com/nektos/act/releases/download/v$ACT_VERSION/act_Linux_x86_64.tar.gz"
  echo "$ACT_SHA256  $archive" | sha256sum --check --quiet
  tar --extract --gzip --file "$archive" --directory "$(dirname "$act_bin")" act
  rm -f "$archive"
  trap - EXIT
fi

# Podman serves the Docker API on a socket of its own; act and the "docker" job
# inside the runner talk to whichever engine DOCKER_HOST names.
if [[ -z "${DOCKER_HOST:-}" ]]; then
  podman_socket="${XDG_RUNTIME_DIR:-/run/user/$(id -u)}/podman/podman.sock"
  if [[ -S "$podman_socket" ]]; then
    export DOCKER_HOST="unix://$podman_socket"
  fi
fi

# The pull_request event the workflow expects. A base commit that does not exist
# makes the "changes" job fail to list the changed files, and the workflow then runs
# every job, as it does in CI.
head_sha="$(git rev-parse HEAD)"
no_commit="0000000000000000000000000000000000000000"
base_sha="$no_commit"
if [[ "$all" == false && -n "$(git status --porcelain)" ]]; then
  # The path filters compare commits and would miss uncommitted changes.
  echo "The working tree has uncommitted changes; running every job." >&2
  all=true
fi
if [[ "$all" == false ]]; then
  if ! base_sha="$(git merge-base "$base" HEAD 2>/dev/null)"; then
    echo "No common commit with $base; running every job." >&2
    base_sha="$no_commit"
  fi
fi
event="$(mktemp)"
artifacts="$(mktemp -d)"
trap 'rm -rf "$event" "$artifacts"' EXIT
cat > "$event" <<EOF
{
  "pull_request": {
    "number": 0,
    "base": {"ref": "$base", "sha": "$base_sha"},
    "head": {"ref": "$(git rev-parse --abbrev-ref HEAD)", "sha": "$head_sha"}
  }
}
EOF

# In a Git worktree, .git is a file that points into the main repository, which act
# does not copy. The repository is mounted read-only at its own path so that the
# "changes" job can still compare commits.
git_common_dir="$(cd "$(git rev-parse --git-common-dir)" && pwd)"
container_options="--volume $git_common_dir:$git_common_dir:ro"

"$act_bin" pull_request \
  --workflows .github/workflows/ci.yml \
  --eventpath "$event" \
  --platform "ubuntu-latest=$RUNNER_IMAGE" \
  --artifact-server-path "$artifacts" \
  --container-options "$container_options" \
  --env GIT_CONFIG_COUNT=1 --env GIT_CONFIG_KEY_0=safe.directory --env "GIT_CONFIG_VALUE_0=*" \
  "${act_args[@]}"
