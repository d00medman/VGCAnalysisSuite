# syntax=docker/dockerfile:1.7

FROM rust:1.94-slim-bookworm AS builder
WORKDIR /src
COPY Cargo.toml Cargo.lock ./
COPY src ./src
COPY migrations ./migrations
# `target/` is a cache mount and does NOT persist into the layer, so the binary must
# be copied out inside this same RUN. --locked so a container build cannot resolve a
# different dependency graph than the local one.
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/src/target \
    cargo build --release --locked \
 && install -m0755 target/release/pokedex /usr/local/bin/pokedex

FROM debian:bookworm-slim AS runtime
# rusqlite `bundled` links SQLite statically, so the runtime needs only glibc.
RUN useradd --uid 10001 --create-home --shell /usr/sbin/nologin pokedex \
 && mkdir -p /data && chown pokedex:pokedex /data
COPY --from=builder /usr/local/bin/pokedex /usr/local/bin/pokedex
USER 10001:10001
# /data is the state boundary. Deliberately no VOLUME instruction: it would create an
# ungarbage-collected anonymous volume on every `docker run` and freeze the path.
# Migrations are include_str!'d into the binary, so no migrations dir is shipped.
ENV POKEDEX_DB=/data/pokedex.db
ENTRYPOINT ["pokedex"]
CMD ["--help"]
