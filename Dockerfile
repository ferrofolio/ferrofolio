# Stage 1: Rust gateway
FROM rust:1.99.1-slim-trixie AS rust-builder

WORKDIR /build

# SQLCipher in infra-db is built against the system's OpenSSL (docs/rust-backend.md).
RUN apt-get update && \
    apt-get install --no-install-recommends -y libssl-dev pkg-config && \
    apt-get clean && \
    rm -rf /var/lib/apt/lists/*

COPY Cargo.toml Cargo.lock ./
COPY crates ./crates

RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/build/target,sharing=locked \
    cargo build --release --locked --bin ferrofolio && \
    cp target/release/ferrofolio /usr/local/bin/ferrofolio

# Stage 2: Python backend dependencies
FROM python:3.14.10-slim AS builder

# Install build-time dependencies (curl for rust)
RUN apt-get update && \
    apt-get install --no-install-recommends -y curl && \
    apt-get clean && \
    rm -rf /var/lib/apt/lists/*

# Install Rust for pydantic compilation. pipefail lets a failed download fail the build
# instead of handing sh an empty script.
SHELL ["/bin/bash", "-o", "pipefail", "-c"]
ENV PATH="/root/.cargo/bin:${PATH}"
RUN curl https://sh.rustup.rs -sSf | sh -s -- -y --default-toolchain stable --profile minimal

# Create and activate virtual environment
RUN python -m venv /opt/venv
ENV PATH="/opt/venv/bin:$PATH"

# Install Python dependencies
WORKDIR /build
COPY requirements.txt requirements-selenium.txt ./
RUN pip install --no-cache-dir -r requirements.txt

ARG SELENIUM_SUPPORT=true
RUN if [ "$SELENIUM_SUPPORT" = "true" ]; then \
    pip install --no-cache-dir -r requirements-selenium.txt; \
    fi

# Stage 3: Final Runtime Image
FROM python:3.14.10-slim

ARG SELENIUM_SUPPORT=true

# Install runtime system dependencies conditionally
RUN if [ "$SELENIUM_SUPPORT" = "true" ]; then \
    apt-get update && \
    apt-get install --no-install-recommends -y ffmpeg flac && \
    apt-get clean && \
    rm -rf /var/lib/apt/lists/*; \
    fi

# Copy virtual environment from builder
COPY --from=builder /opt/venv /opt/venv

# Activate virtual environment
ENV PATH="/opt/venv/bin:$PATH"

ENV FINANZE_DISTRIBUTION=docker
ENV FINANZE_HOST=0.0.0.0

# Copy application code
COPY finanze /finanze

# The Rust gateway is the entry point: it serves the ported routes and starts the
# Python backend on the loopback interface for all others (docs/roadmap.md, section 5).
COPY --from=rust-builder /usr/local/bin/ferrofolio /usr/local/bin/ferrofolio
ENV FERROFOLIO_PYTHON=python
ENV FERROFOLIO_BACKEND_APP=/finanze

ENTRYPOINT ["ferrofolio"]
