# Multi-stage build with cargo-chef for dependency layer caching.
# This separates slow dependency compilation (cached) from fast source compilation.
#
# Stage order: chef → planner → builder → runtime
# Rebuilding after a source-only change reuses the "builder" layer up to
# the `COPY . .` step, so only the final `cargo build` re-runs.

# ---- Stage 1: Base with build tools ----
FROM rust:latest AS chef

# RocksDB (rocksdb crate) requires clang + cmake + C++ stdlib.
# These are only needed in the build stages; the runtime image stays lean.
RUN apt-get update && apt-get install -y \
    clang \
    cmake \
    libclang-dev \
    libssl-dev \
    pkg-config \
    && rm -rf /var/lib/apt/lists/*

RUN cargo install cargo-chef --locked
WORKDIR /app

# ---- Stage 2: Planner — captures dependency manifest ----
FROM chef AS planner
COPY . .
RUN cargo chef prepare --recipe-path recipe.json

# ---- Stage 3: Builder — compile deps then source ----
FROM chef AS builder

COPY --from=planner /app/recipe.json recipe.json

# Compile dependencies only (this layer is cached until Cargo.lock changes).
RUN cargo chef cook --release --recipe-path recipe.json

# Now copy the full source and do the final build.
COPY . .
RUN cargo build --release -p aura-l2-ingestor

# ---- Stage 4: Runtime — same base as builder to guarantee ABI compatibility ----
# Using rust:latest (same image as the builder) avoids glibc/libstdc++ version mismatches.
FROM rust:latest AS runtime

WORKDIR /app

COPY --from=builder /app/target/release/ingestor ./ingestor

# State DB path is configurable via STATE_DB_PATH env var.
RUN mkdir -p /app/data

ENTRYPOINT ["./ingestor"]
