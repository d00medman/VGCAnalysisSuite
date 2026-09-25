#!/usr/bin/env bash
# Regenerate every committed snapshot in pokedex/snapshots/.
#
#   ./export.sh <showdown-git-sha> [outDir]
#
# M-A comes from npm pokemon-showdown 0.11.11, the last source that still has
# `championsregma`, and runs on the host's Node. M-B and M-C come from Showdown master at
# the given commit: it is fetched into .showdown/<sha>, built once, and exported inside
# node:22, because master needs Node >= 22. Each snapshot records its source.
# See devlog/DataSourcingResearchResults.md, rev 3.
set -euo pipefail

sha=${1:?usage: export.sh <showdown-git-sha> [outDir]}
[[ $sha =~ ^[0-9a-f]{40}$ ]] || { echo "need a full 40-character commit SHA, got '$sha'" >&2; exit 1; }

here=$(cd "$(dirname "$0")" && pwd)
out=$(mkdir -p "${2:-$here/../snapshots}" && cd "${2:-$here/../snapshots}" && pwd)
cache=$here/.showdown/$sha
as_me=(--user "$(id -u):$(id -g)" -e HOME=/tmp)

if [[ ! -f $cache/dist/sim/index.js ]]; then
  echo "== fetching and building Showdown $sha"
  rm -rf "$cache" && mkdir -p "$cache"
  git -C "$cache" init -q
  git -C "$cache" fetch -q --depth 1 https://github.com/smogon/pokemon-showdown.git "$sha"
  git -C "$cache" checkout -q FETCH_HEAD
  docker run --rm "${as_me[@]}" -v "$cache:/ps" -w /ps node:22 \
    sh -c 'npm ci --no-audit --no-fund --loglevel=error && node build'
fi

echo "== M-A from npm"
have=$(node -p "require('$here/node_modules/pokemon-showdown/package.json').version" 2>/dev/null || true)
[[ $have == 0.11.11 ]] || (cd "$here" && npm ci --no-audit --no-fund)
node "$here/showdown-snapshot.js" m-a "$out"

echo "== M-B, M-C from Showdown $sha"
docker run --rm "${as_me[@]}" \
  -e SHOWDOWN_PATH=/ps -e SHOWDOWN_SHA="$sha" \
  -v "$cache:/ps:ro" -v "$here/showdown-snapshot.js:/ingest/showdown-snapshot.js:ro" -v "$out:/out" \
  node:22 sh -c 'node /ingest/showdown-snapshot.js m-b /out && node /ingest/showdown-snapshot.js m-c /out'

# A verification aid for migration 0002, rewritten by each export above. It belongs in
# out/ for ad-hoc runs, not beside the snapshots.
rm -f "$out/typechart-check.json"
