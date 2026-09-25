#!/usr/bin/env bash
# Dump the battle tables to backups/battles-<timestamp>.sql (gitignored).
#
#   scripts/backup-battles.sh postgres://pokedex:pokedex@127.0.0.1:5432/pokedex
#
# These rows exist nowhere else: turn marks are made by hand, and transcripts are slow to
# regenerate. The pokedex does not need this; it rebuilds from committed snapshots.
#
# Data only, so it restores into a schema created by `pokedex migrate`:
#   psql <url> -v ON_ERROR_STOP=1 -1 -f backups/battles-<timestamp>.sql
# battle.regulation_id is a raw id; it lines up when regulations were added oldest first,
# as rebuild-pokedex.sh and load-reference.sh both do. Uploaded video files live in the
# compose `videos` volume and are not included.
#
# pg_dump runs from postgres:17-alpine: it refuses servers newer than itself.
set -euo pipefail

url=${1:?usage: backup-battles.sh <postgres-url>}
repo=$(cd "$(dirname "$0")/.." && pwd)
mkdir -p "$repo/backups"
out=$repo/backups/battles-$(date -u +%Y%m%dT%H%M%SZ).sql

docker run --rm --network host --user "$(id -u):$(id -g)" -e HOME=/tmp postgres:17-alpine \
  pg_dump "$url" --data-only --no-owner \
    -t video -t battle -t transcript -t transcript_line -t turn_end >"$out.tmp"
mv "$out.tmp" "$out"

echo "wrote $out ($(du -h "$out" | cut -f1))"
