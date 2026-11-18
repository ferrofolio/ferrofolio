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
# SHA-256 digest and kept in ${XDG_CACHE_HOME:-~/.cache}/ferrofolio, and the runner
# image is built locally from scripts/ci-local.Containerfile.

set -euo pipefail

ACT_VERSION="0.2.96"
# SHA-256 digests of the release archives, from the release's checksums.txt.
act_sha256() {
  case "$1" in
    Linux_x86_64) echo "b861e901c9062d841c6e8ecd36cdf27e0ce39ea0124cfd93de6efcebea0e16e7" ;;
    Linux_arm64) echo "158a16eee7394c266ee87dee9f15ca5f219d47193f7a1df9bad9093774e340c4" ;;
    Darwin_x86_64) echo "84a95ddc407e66a5bb84e8debfe27b50fb83f009487a4be84e17f90a82b05992" ;;
    Darwin_arm64) echo "fba2638fb6e1def14fbbb704b27888c7b697d041da5b08cac7d06d055ef9d690" ;;
  esac
}

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

sha256() {
  if command -v sha256sum > /dev/null; then
    sha256sum "$1" | cut -d ' ' -f 1
  else
    shasum -a 256 "$1" | cut -d ' ' -f 1
  fi
}

# act runs on the host, so the archive is the one for the host's system.
case "$(uname -m)" in
  x86_64 | amd64) arch="x86_64" ;;
  aarch64 | arm64) arch="arm64" ;;
  *) arch="$(uname -m)" ;;
esac
platform="$(uname -s)_$arch"
expected_sha256="$(act_sha256 "$platform")"
if [[ -z "$expected_sha256" ]]; then
  echo "No act release is pinned for $platform; install act $ACT_VERSION and add it." >&2
  exit 1
fi
cache_dir="${XDG_CACHE_HOME:-$HOME/.cache}/ferrofolio"
act_bin="$cache_dir/act-$ACT_VERSION-$platform/act"
if [[ ! -x "$act_bin" ]]; then
  mkdir -p "$cache_dir"
  # Unpacked next to its final place and renamed when complete, so an interrupted
  # download never leaves a partial executable in the cache.
  staging="$(mktemp -d "$cache_dir/act-download.XXXXXX")"
  trap 'rm -rf "$staging"' EXIT
  echo "Downloading act $ACT_VERSION for $platform"
  curl --fail --silent --show-error --location --max-time 300 --output "$staging/act.tar.gz" \
    "https://github.com/nektos/act/releases/download/v$ACT_VERSION/act_$platform.tar.gz"
  if [[ "$(sha256 "$staging/act.tar.gz")" != "$expected_sha256" ]]; then
    echo "The act archive does not match its pinned SHA-256 digest." >&2
    exit 1
  fi
  tar -xzf "$staging/act.tar.gz" -C "$staging" act
  mkdir -p "$(dirname "$act_bin")"
  mv "$staging/act" "$act_bin"
  rm -rf "$staging"
  trap - EXIT
fi

# Podman serves the Docker API on a socket of its own; act and the "docker" job
# inside the runner talk to whichever engine DOCKER_HOST names. Without DOCKER_HOST,
# a working Docker (in whatever context it is configured) is used as it is.
podman=false
if [[ -z "${DOCKER_HOST:-}" ]]; then
  # Only Docker reports a ServerVersion; Podman's docker command fails on it.
  if ! docker info --format '{{.ServerVersion}}' > /dev/null 2>&1; then
    podman_socket="${XDG_RUNTIME_DIR:-/run/user/$(id -u)}/podman/podman.sock"
    if [[ ! -S "$podman_socket" ]]; then
      echo "Neither Docker nor Podman's API socket is running. Start one, for example" >&2
      echo "  systemctl --user start podman.socket" >&2
      echo "or point DOCKER_HOST at a running Docker or Podman API." >&2
      exit 1
    fi
    export DOCKER_HOST="unix://$podman_socket"
    podman=true
  elif docker_host="$(docker context inspect --format '{{.Endpoints.docker.Host}}' 2> /dev/null)" \
    && [[ -n "$docker_host" ]]; then
    # act does not read Docker contexts and only looks for sockets in a few fixed
    # places (Docker Desktop on Linux is not one), so it is given the context's
    # endpoint to use the same engine as the Docker CLI.
    export DOCKER_HOST="$docker_host"
  fi
elif [[ "$DOCKER_HOST" == *podman* ]]; then
  podman=true
fi

# The runner image: act's Ubuntu image plus what GitHub's runner has preinstalled
# (scripts/ci-local.Containerfile). It is built once per version of that file.
engine="$(command -v docker || command -v podman || true)"
if [[ -z "$engine" ]]; then
  echo "Docker or Podman is needed." >&2
  exit 1
fi
containerfile="scripts/ci-local.Containerfile"
runner_image="localhost/ferrofolio-ci-runner:$(sha256 "$containerfile" | cut -c1-12)"
if ! "$engine" image inspect "$runner_image" > /dev/null 2>&1; then
  echo "Building the runner image $runner_image"
  "$engine" build --tag "$runner_image" --file "$containerfile" scripts
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
# act splits these options like a shell, so a path with spaces is escaped.
printf -v git_volume '%q' "$git_common_dir:$git_common_dir:ro"
container_options="--volume $git_volume"
# act asks for an OOM score adjustment of 0. A rootless Podman service started by
# systemd runs at 200 and cannot go lower, so every command act runs in the container
# would fail; the jobs get the highest value, which is also the first to be freed
# when memory runs out.
container_options+=" --oom-score-adj 1000"

# Podman builds images itself and has no BuildKit API; the Docker CLI in the runner
# would otherwise start a BuildKit container, which a rootless Podman service cannot
# run commands in either.
engine_env=()
if [[ "$podman" == true ]]; then
  engine_env=(--env DOCKER_BUILDKIT=0)
fi

"$act_bin" pull_request \
  --workflows .github/workflows/ci.yml \
  --eventpath "$event" \
  --platform "ubuntu-latest=$runner_image" \
  --pull=false \
  --artifact-server-path "$artifacts" \
  --container-options "$container_options" \
  --env GIT_CONFIG_COUNT=1 --env GIT_CONFIG_KEY_0=safe.directory --env "GIT_CONFIG_VALUE_0=*" \
  ${engine_env[@]+"${engine_env[@]}"} \
  ${act_args[@]+"${act_args[@]}"}
