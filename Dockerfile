# Stage 1: Build and test
FROM rust:1.84-bookworm AS builder

WORKDIR /build

# Copy manifests first for dependency caching
COPY Cargo.toml Cargo.lock ./
COPY crates/wyoming/Cargo.toml crates/wyoming/Cargo.toml
COPY crates/satellite/Cargo.toml crates/satellite/Cargo.toml

# Create stub source files so cargo can resolve the dependency graph and cache deps
RUN mkdir -p crates/wyoming/src crates/satellite/src \
    && echo "pub fn stub() {}" > crates/wyoming/src/lib.rs \
    && echo "fn main() {}" > crates/satellite/src/main.rs \
    && cargo build --release --workspace 2>&1 \
    && rm -rf crates/wyoming/src crates/satellite/src

# Copy real source code
COPY crates/ crates/

# Rebuild with real sources (deps are cached) and run tests
RUN cargo test --workspace \
    && cargo build --release --workspace

# Stage 2: Slim runtime image
FROM debian:bookworm-slim

RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates \
    && rm -rf /var/lib/apt/lists/*

COPY --from=builder /build/target/release/wyoming-satellite /usr/local/bin/wyoming-satellite

ENTRYPOINT ["wyoming-satellite"]
