# Pokedex — Schema & Ingest

Rev 8 · 2026-09-03 · **working end to end on real data**

Resumption document. Read this to pick the work back up; read the SQL for structure.

---

## 1. State

`pokedex/` is a Rust library + CLI over a temporal SQLite schema. It builds, tests, containerizes,
and has ingested the real Showdown snapshots produced by the data-sourcing thread.

| | |
|---|---|
| Schema | 14 tables, 7 views, 6 triggers, 5 indexes |
| Seeded | 18 types, 324 type-chart rules, 25 natures, 7 variant kinds |
| Tests | `cargo test` — 13/13 pass; clippy clean |
| Container | `pokedex:dev`, 78MB, non-root, bind-mounted `/data` |
| Real data | 348 pokemon / 500 moves / 14,192 learnset rows across 2 regulations |

**Proven on real data** (`ingest/out/snapshot.regulation-m-*.json`):
- Both regulations import; re-import is a no-op
- **Sparseness works:** M-B added 38 stat rows for 348 pokemon and **zero** `move_data` rows.
  The 38 were all newly-introduced species, not rebalances.
- 924K database from 2.3M of JSON
- 140 variants: **76 mega, 49 form, 15 regional** — generalizing megas to `variant` was
  load-bearing, 46% of variants are not megas
- Effectiveness correct against real rows (Electric→Gyarados 400, Ground→Charizard 0)
- The sourcing thread's independently-derived `typechart-check.json` matches our seeded
  324-row chart with **zero mismatches** — corroborates decision 2 for this dataset

## 2. Running it

```bash
cd pokedex
cargo test                        # 13 integration tests, no fixtures needed
cargo build --release

./target/release/pokedex --db ./data/pokedex.db init
./target/release/pokedex --db ./data/pokedex.db regulation add \
    --name "Regulation M-A" --effective-from 2025-01-01
./target/release/pokedex --db ./data/pokedex.db import \
    --file ingest/out/snapshot.regulation-m-a.json

# container — bind mount, so `sqlite3 data/pokedex.db` works from the host
export UID=$(id -u) GID=$(id -g)
docker compose run --rm pokedex init
docker compose run --rm -T pokedex import < ingest/out/snapshot.regulation-m-a.json
```

As a library: `Db::open` → `db.migrate()` → `ingest::apply(db.conn_mut(), &snapshot)`, or
`ingest::apply_in(&tx, &snapshot)` to batch several snapshots into one commit.

CLI: `init | migrate | status | regulation add|list | import [--file X|stdin] [--dry-run]`.
Env: `POKEDEX_DB`, `POKEDEX_AUTO_MIGRATE`, `POKEDEX_JOURNAL_MODE`.

## 3. Layout

```
pokedex/
  migrations/0001_init.sql        AUTHORITATIVE schema; reasoning inline at each decision
             0002_seed_types.sql  18 types + 324 chart rules   } GENERATED — regenerate,
             0003_seed_ref.sql    25 natures + 7 variant kinds  } do not hand-edit
  src/lib.rs      db · migrate · error · model · resolve · regulation · ingest
      main.rs     thin clap shell over the library
  tests/integration.rs            13 tests, fresh in-memory DB each
  ingest/                         OTHER THREAD: Showdown -> snapshot.json exporter (Node)
  Dockerfile compose.yaml .env .dockerignore
```

`ingest/` is the data-sourcing thread's work, not this one's. It emits exactly the `Snapshot`
shape `src/model.rs` consumes — verified, they line up with no adapter.

## 4. The mental model

Regulations make this **temporal**. Every versioned fact is one of two shapes, and picking wrong
is the main way this design goes bad:

| Shape | Question | Storage | Tables |
|---|---|---|---|
| **Scalar** | "what was the value at regulation R?" | sparse rows; *latest at or before R* wins | `pokemon_stats`, `move_data` |
| **Set membership** | "was this in the set at R?" | half-open interval `[valid_from, valid_to)` | `pokemon_type`, `pokemon_ability`, `pokemon_move` |

Scalar tables **cannot express removal** — "latest wins" always finds something. Interval tables
can, via `valid_to_regulation_id`. That is the entire reason for the split.

All three interval tables share one invariant, enforced by trigger: **at most one open window per
logical key.** Two open rows for a key is not bad data, it is broken resolution — the
effectiveness view would return two slot-1 rows for one regulation and silently duplicate.

**Ingest contract: feed dense, store sparse.** A caller submits a *complete snapshot for one
regulation*. The ingest layer diffs against stored state and writes only what changed. The
pipeline never computes deltas.

**Absence is destructive, and the types say so.** A snapshot is read as complete, so a present
set that omits an entry closes that entry's interval. Hence `Option` on the set-valued fields:

```rust
None          // not provided — leave stored data alone
Some(vec![])  // explicitly empty — close every interval
```

A scraper that fetched only stats must leave `learnset: None`, or it wipes a learnset it never
looked at.

## 5. Decision record

| # | Decision | Chosen | Reversal cost |
|---|---|---|---|
| 1 | Type chart density | **dense**, 324 rows — plain JOIN everywhere; sparse needs `COALESCE(...,100)` and a forgotten one silently drops neutral matchups instead of erroring | trivial |
| 2 | Chart scoped by generation/regulation | **no** — modern chart only. Corroborated: Showdown's Champions chart matches ours exactly | **HIGH** (enters a PK) |
| 3 | Seed natures | **yes** — static since Gen 3 | trivial |
| 4a | Runtime base image | **debian:bookworm-slim** — shell for debugging; binary links only glibc so distroless would also work | trivial |
| 4b | `/data` mount | **bind mount** + `user:` — host `sqlite3` access matters while developing | trivial |
| 5 | Temporal `pokemon_type` | **yes** | moderate |
| 6 | Fire/Fire UNIQUE | **dropped** — data-quality not structural; ingest checks it in ~3 lines | trivial |
| 7 | Megas | **generalized to `variant`/`variant_kind`** — real data proved this right, 46% of variants are not megas | moderate |

On #2, if it must ever be reversed: scope by **regulation**, not generation. A second independent
time axis forces "which generation is Regulation M-B?" into every query.

On #6: the dropped constraint only worked *paired* with atomic slot-set versioning — alone it
misses slot 1 opened in an earlier regulation and left open while slot 2 is later set to the same
type. Dropping the pair let the trigger simplify to one-open-per-slot and kept interval versioning
uniform across all three tables.

## 6. Gotchas

- **`PRAGMA foreign_keys` is a no-op inside a transaction** — it is set per-connection in
  `Db::open`, and must never move into a migration file.
- **Docker cache-mount staleness.** `COPY` preserves host mtimes, so a source file can land older
  than an artifact in the `/src/target` cache mount. Cargo fingerprints on mtime, calls the crate
  fresh, and links a stale library against fresh code — surfacing as a baffling
  `unresolved import` for code that compiles locally. The `touch src/*.rs Cargo.toml` in the
  Dockerfile is load-bearing; do not remove it.
- **`.dockerignore` is an allowlist** (`*` then re-include). A 198M `ingest/node_modules` tree
  appeared mid-session and silently entered the build context; enumerating exclusions loses that
  race every time.
- **Resolution views CROSS JOIN `regulation`** and must be queried with a filter (`regulation_id`
  and/or `pokemon_id`). An unfiltered scan is O(pokemon × regulations) correlated subqueries.
- **`pokemon_stats_effective` uses inner JOIN, not LEFT** — load-bearing. A pokemon with no stat
  row at or before R does not appear in R, so species introduced later correctly vanish from
  earlier regulations for free.
- **Natures look orphaned and that is correct.** Per-*individual* trait, not a species trait;
  nothing joins to `nature` until an individuals table exists.
- **`variant_kind`, not `variant_type`** — `type` already means elemental type here.
- WAL needs shared memory and POSIX locking: fine on native Linux bind mounts, breaks on Docker
  Desktop macOS/Windows shims. `POKEDEX_JOURNAL_MODE=DELETE` is the escape hatch, no rebuild.

## 7. Next

1. **`pokemon_legality` — the largest known hole.** "Regulation" in VGC is fundamentally about
   what is *legal*, but it was out of the original scope. Stats resolution gives a crude proxy
   ("exists from regulation X onward") but **cannot express removal-then-return** — megas legal in
   Gen 6-7, gone in Gen 8, back later. A scalar stat row resolves forward indefinitely, so a mega
   currently looks legal in regulations where it was not. Fix: an interval table reusing the
   half-open window shape and trigger pattern already defined. Cheap now, expensive once more data
   lands.
2. Wire `ingest/` and the Rust importer into one command so a regulation refresh is a single step.
3. One of 348 pokemon has no stat row (347 stored) — worth finding out which and why.
4. `genus` is null across the Showdown export; source it elsewhere if it matters.
5. Query surface — nothing reads this DB yet beyond the views.

## 8. Deliberate non-goals

Ability descriptions unversioned · type chart unversioned (decision 2) · individual/caught-pokemon
tracking (the only thing `nature` would join to) · two overlapping *closed* interval windows are
not caught by the trigger (unreachable via normal ingest; needs a deliberate backdated insert).

---

Tags: database, development
