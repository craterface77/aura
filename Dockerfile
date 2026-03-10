# Multi-stage build — single builder compiles ALL workspace crates once.
# Both `ingestor` and `api` binaries come from the same builder layer,
# so RocksDB / alloy / revm are only compiled once.
#
# Targets:
#   ingestor-runtime  — used by docker-compose ingestor service
#   api-runtime       — used by docker-compose api service

# ---- Stage 1: Base with build tools ----
FROM rust:latest AS chef

RUN apt-get update && apt-get install -y \
    clang \
    cmake \
    libclang-dev \
    libssl-dev \
    pkg-config \
    protobuf-compiler \
    && rm -rf /var/lib/apt/lists/*

RUN cargo install cargo-chef --locked
WORKDIR /app

# ---- Stage 2: Planner — captures dependency manifest ----
FROM chef AS planner
COPY . .
RUN cargo chef prepare --recipe-path recipe.json

# ---- Stage 3: Builder — compile deps then ALL binaries ----
FROM chef AS builder

COPY --from=planner /app/recipe.json recipe.json

# Compile ALL workspace dependencies once (cached until Cargo.lock changes).
RUN cargo chef cook --release --recipe-path recipe.json

COPY . .

# Build both binaries in one pass — shared dep compilation.
RUN cargo build --release -p aura-l2-ingestor -p aura-l2-api

# ---- Stage 4a: Ingestor runtime ----
FROM rust:latest AS ingestor-runtime

WORKDIR /app
RUN mkdir -p /app/data
COPY --from=builder /app/target/release/ingestor ./ingestor
ENTRYPOINT ["./ingestor"]

# ---- Stage 4b: API runtime ----
FROM rust:latest AS api-runtime

WORKDIR /app
RUN mkdir -p /app/data
EXPOSE 50051 3000
COPY --from=builder /app/target/release/api ./api
ENTRYPOINT ["./api"]
