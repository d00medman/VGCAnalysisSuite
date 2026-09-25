# syntax=docker/dockerfile:1.7

FROM rust:1.94-slim-bookworm AS builder
WORKDIR /src
COPY Cargo.toml Cargo.lock ./
COPY src ./src
COPY migrations ./migrations
# `target/` is a cache mount and does NOT persist into the layer, so the binary must
# be copied out inside this same RUN. --locked so a container build cannot resolve a
# different dependency graph than the local one.
#
# The `touch` is load-bearing. COPY preserves host mtimes, so a source file can land
# OLDER than an artifact left in the target cache mount by an earlier build. Cargo
# fingerprints on mtime, would call the crate fresh, and would silently link a stale
# library against freshly-built code -- which fails as a confusing "unresolved import"
# rather than as a cache error. Touching after COPY forces our own crate to rebuild
# while dependency artifacts in the cache stay valid.
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/src/target \
    touch src/*.rs Cargo.toml \
 && cargo build --release --locked \
 && install -m0755 target/release/pokedex /usr/local/bin/pokedex

FROM debian:bookworm-slim AS runtime
# The postgres driver is pure Rust, so the runtime needs only glibc. The container is
# stateless: all state lives in Postgres, reached through DATABASE_URL.
RUN useradd --uid 10001 --create-home --shell /usr/sbin/nologin pokedex
COPY --from=builder /usr/local/bin/pokedex /usr/local/bin/pokedex
USER 10001:10001
# Migrations are include_str!'d into the binary, so no migrations dir is shipped.
ENTRYPOINT ["pokedex"]
CMD ["--help"]
