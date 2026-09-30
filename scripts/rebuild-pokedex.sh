#!/usr/bin/env bash
# Rebuild the pokedex from the committed snapshots: migrate an empty database, add every
# regulation, then import the snapshots oldest first.
#
#   scripts/rebuild-pokedex.sh postgres://pokedex:pokedex@127.0.0.1:55432/pokedex
#
# The database must be empty: ingest refuses a snapshot older than one already loaded,
# so a rebuild only makes sense from nothing. The pokedex CLI is built from this checkout
# with cargo; set POKEDEX to use another binary.
set -euo pipefail

export DATABASE_URL=${1:?usage: rebuild-pokedex.sh <postgres-url>}
repo=$(cd "$(dirname "$0")/.." && pwd)
snapshots=$repo/pokedex/snapshots

if [[ -n ${POKEDEX:-} ]]; then
  pokedex=("$POKEDEX")
else
  cargo build -q --release --manifest-path "$repo/pokedex/Cargo.toml"
  pokedex=("$repo/pokedex/target/release/pokedex")
fi
unset POKEDEX_AUTO_MIGRATE

# Oldest first. Dates are the in-game start of each ranked season. Notes live here, not
# only in a database, so every rebuild writes the same regulation.csv.
regulations=(
  "Regulation M-A|2026-04-08|snapshot.regulation-m-a.json|"
  "Regulation M-B|2026-06-17|snapshot.regulation-m-b.json|ran 2026-06-17 to 2026-09-09"
  "Regulation M-C|2026-09-09|snapshot.regulation-m-c.json|"
)

"${pokedex[@]}" migrate
if [[ -n $("${pokedex[@]}" regulation list) ]]; then
  echo "refusing: this database already has regulations; rebuild needs an empty one" >&2
  exit 1
fi

for entry in "${regulations[@]}"; do
  IFS='|' read -r name from _ notes <<<"$entry"
  "${pokedex[@]}" regulation add --name "$name" --effective-from "$from" ${notes:+--notes "$notes"}
done

for entry in "${regulations[@]}"; do
  IFS='|' read -r name _ file _ <<<"$entry"
  echo
  "${pokedex[@]}" import --regulation "$name" --file "$snapshots/$file"
done

echo
"${pokedex[@]}" status
