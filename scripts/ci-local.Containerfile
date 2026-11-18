# SPDX-License-Identifier: AGPL-3.0-only
#
# The runner image for scripts/ci-local.sh: act's Ubuntu image plus the tools that
# GitHub's ubuntu-latest runner has preinstalled and the workflow relies on. It adds
# tools only, no checks; the checks stay in .github/workflows/ci.yml.
FROM ghcr.io/catthehacker/ubuntu:act-24.04

# rustup; the workflow installs the toolchain from rust-toolchain.toml with it.
ARG RUSTUP_VERSION=1.28.2
ARG RUSTUP_SHA256=20a06e644b0d9bd2fbdbfd52d42540bdde820ea7df86e92e533c073da0cdd43c
ENV RUSTUP_HOME=/opt/rust/rustup \
    CARGO_HOME=/opt/rust/cargo \
    PATH=/opt/rust/cargo/bin:$PATH
RUN curl --fail --silent --show-error --location --output /tmp/rustup-init \
      "https://static.rust-lang.org/rustup/archive/${RUSTUP_VERSION}/x86_64-unknown-linux-gnu/rustup-init" && \
    echo "${RUSTUP_SHA256}  /tmp/rustup-init" | sha256sum --check --quiet && \
    chmod +x /tmp/rustup-init && \
    /tmp/rustup-init -y --no-modify-path --profile minimal --default-toolchain none && \
    rm /tmp/rustup-init
