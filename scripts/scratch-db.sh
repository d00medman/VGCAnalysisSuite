#!/usr/bin/env bash
# A throwaway Postgres for trying schema changes off the shared dev DB
# (devlog/SchemaIteration.md §4, steps 2-3): the pokedex rebuilt from snapshots, then a
# battle backup restored on top.
#
#   scripts/scratch-db.sh [backups/battles-<timestamp>.sql]
#   -> postgres://pokedex:pokedex@127.0.0.1:55432/pokedex
#
# With no argument it restores the newest backup, looking in this checkout's backups/ and
# then the main checkout's (worktrees have no backups of their own). Any earlier scratch
# container is replaced. Stop it with `docker stop pokedex-scratch`; it removes itself.
#
# A data-only backup restores only into the schema version it was taken from. To try a new
# migration against old data, build the pokedex at the old version and pass it in, then
# migrate with the new one:
#   cargo build --release --manifest-path ../pokemon_recordings/pokedex/Cargo.toml
#   POKEDEX=../pokemon_recordings/pokedex/target/release/pokedex scripts/scratch-db.sh
#   DATABASE_URL=postgres://pokedex:pokedex@127.0.0.1:55432/pokedex pokedex/target/release/pokedex migrate
set -euo pipefail

repo=$(cd "$(dirname "$0")/.." && pwd)
main=$(dirname "$(git -C "$repo" rev-parse --path-format=absolute --git-common-dir)")
port=${SCRATCH_PORT:-55432}
name=pokedex-scratch
url=postgres://pokedex:pokedex@127.0.0.1:$port/pokedex

backup=${1:-}
if [[ -z $backup ]]; then
  backup=$(ls -1 "$repo"/backups/battles-*.sql "$main"/backups/battles-*.sql 2>/dev/null \
             | awk -F/ '{print $NF "\t" $0}' | sort | tail -n1 | cut -f2 || true)
  [[ -n $backup ]] || { echo "no backups/battles-*.sql found; pass one" >&2; exit 1; }
fi
[[ -f $backup ]] || { echo "no such backup: $backup" >&2; exit 1; }

docker rm -f "$name" >/dev/null 2>&1 || true
docker run --rm -d --name "$name" \
  -e POSTGRES_USER=pokedex -e POSTGRES_PASSWORD=pokedex -e POSTGRES_DB=pokedex \
  -p "127.0.0.1:$port:5432" postgres:17-alpine >/dev/null
# pg_isready passes during the image's init restart, so wait for a real query instead.
for _ in $(seq 60); do
  docker exec "$name" psql -U pokedex -d pokedex -qtAc 'SELECT 1' >/dev/null 2>&1 && break
  sleep 0.5
done

psql() { docker exec -i "$name" psql -U pokedex -d pokedex -v ON_ERROR_STOP=1 "$@"; }

"$repo/scripts/rebuild-pokedex.sh" "$url" >/dev/null
echo "restoring $backup"
psql -q -1 <"$backup" >/dev/null

echo "scratch DB at $url"
psql -tAc "SELECT max(version) FROM schema_migrations" | sed 's/^/schema version /'
for t in video battle transcript transcript_line turn_end; do
  printf '%-16s %s\n' "$t" "$(psql -tAc "SELECT count(*) FROM $t")"
done
