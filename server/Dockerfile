# syntax=docker/dockerfile:1.7
# Build context is the repo root (see compose.yaml): the server links the analyzer crate,
# and the pokedex crate for the database migrations it applies at startup.

FROM rust:1.94-slim-bookworm AS builder
WORKDIR /src
COPY analyzer/Cargo.toml analyzer/
COPY analyzer/src analyzer/src
COPY pokedex/Cargo.toml pokedex/
COPY pokedex/src pokedex/src
COPY pokedex/migrations pokedex/migrations
COPY server/Cargo.toml server/Cargo.lock server/
COPY server/src server/src
# `target/` is a cache mount and does not persist into the layer, so the binary is copied
# out in this same RUN. The `touch` matters for the same reason as in pokedex/Dockerfile:
# COPY keeps host mtimes, and a source file older than a cached artifact would make cargo
# link stale code.
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/src/server/target \
    touch analyzer/src/*.rs pokedex/src/*.rs server/src/*.rs \
 && cd server && cargo build --release --locked \
 && install -m0755 target/release/server /usr/local/bin/server

# The pinned static ffmpeg, fetched and checksum-verified by the same script as local dev.
FROM debian:bookworm-slim AS ffmpeg
RUN apt-get update \
 && apt-get install -y --no-install-recommends curl ca-certificates xz-utils \
 && rm -rf /var/lib/apt/lists/*
COPY analyzer/vendor/fetch-ffmpeg.sh /vendor/
RUN /vendor/fetch-ffmpeg.sh

FROM debian:bookworm-slim AS runtime
RUN useradd --uid 10001 --create-home --shell /usr/sbin/nologin analyzer \
 && mkdir -p /data/videos && chown -R analyzer:analyzer /data
COPY --from=builder /usr/local/bin/server /usr/local/bin/server
COPY --from=ffmpeg /vendor/ffmpeg /usr/local/bin/ffmpeg
COPY analyzer/atlas/glyphs.txt /app/atlas/glyphs.txt
COPY analyzer/atlas/lexicon.txt /app/atlas/lexicon.txt
ENV ANALYZER_FFMPEG=/usr/local/bin/ffmpeg \
    ANALYZER_ATLAS=/app/atlas/glyphs.txt \
    VIDEO_DIR=/data/videos \
    BIND=0.0.0.0:8080
USER 10001:10001
EXPOSE 8080
CMD ["server"]
