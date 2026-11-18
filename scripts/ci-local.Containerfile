# SPDX-License-Identifier: AGPL-3.0-only
#
# The runner image for scripts/ci-local.sh: act's Ubuntu image plus the tools that
# GitHub's ubuntu-latest runner has preinstalled and the workflow relies on. It adds
# tools only, no checks; the checks stay in .github/workflows/ci.yml.
FROM ghcr.io/catthehacker/ubuntu:act-24.04

# rustup; the workflow installs the toolchain from rust-toolchain.toml with it.
# TARGETARCH is set by the image builder (amd64 or arm64).
ARG TARGETARCH
ARG RUSTUP_VERSION=1.28.2
ENV RUSTUP_HOME=/opt/rust/rustup \
    CARGO_HOME=/opt/rust/cargo \
    PATH=/opt/rust/cargo/bin:$PATH
RUN case "${TARGETARCH:-$(dpkg --print-architecture)}" in \
      amd64) target=x86_64-unknown-linux-gnu; \
             sha256=20a06e644b0d9bd2fbdbfd52d42540bdde820ea7df86e92e533c073da0cdd43c ;; \
      arm64) target=aarch64-unknown-linux-gnu; \
             sha256=e3853c5a252fca15252d07cb23a1bdd9377a8c6f3efa01531109281ae47f841c ;; \
      *) echo "No rustup installer is pinned for this architecture" >&2; exit 1 ;; \
    esac && \
    curl --fail --silent --show-error --location --output /tmp/rustup-init \
      "https://static.rust-lang.org/rustup/archive/${RUSTUP_VERSION}/${target}/rustup-init" && \
    echo "${sha256}  /tmp/rustup-init" | sha256sum --check --quiet && \
    chmod +x /tmp/rustup-init && \
    /tmp/rustup-init -y --no-modify-path --profile minimal --default-toolchain none && \
    rm /tmp/rustup-init
