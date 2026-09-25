# Pokedex — Schema & Ingest

Rev 10 · 2026-09-25 · **Postgres; held items; battles and transcripts stored per line**

Resumption document. Read this to pick the work back up; read the SQL for structure.

---

## 1. State

`pokedex/` is a Rust library + CLI over a temporal Postgres schema (SQLite until rev 9; see §9). It builds, tests, containerizes,
and has ingested the real Showdown snapshots produced by the data-sourcing thread.

| | |
|---|---|
| Schema | 20 tables (+ `schema_migrations`), 8 views; migrations 0001–0005 |
| Seeded | 18 types, 324 type-chart rules, 25 natures, 7 variant kinds |
| Tests | `cargo test` — 15/15 pass against Postgres; clippy clean |
| Container | `pokedex` service in root `compose.yaml` (`tools` profile), stateless, non-root |
| Real data | 348 pokemon / 500 moves / 14,192 learnset rows / 148 items across 2 regulations |

**Proven on real data** (`ingest/out/snapshot.regulation-m-*.json`):
- Both regulations import; re-import is a no-op
- **Sparseness works:** M-B added 38 stat rows for 348 pokemon and **zero** `move_data` rows.
  The 38 were all newly-introduced species, not rebalances.
- 140 variants: **76 mega, 49 form, 15 regional** — generalizing megas to `variant` was
  load-bearing, 46% of variants are not megas
- Effectiveness correct against real rows (Electric→Gyarados 400, Ground→Charizard 0)
- The sourcing thread's independently-derived `typechart-check.json` matches our seeded
  324-row chart with **zero mismatches** — corroborates decision 2 for this dataset

## 2. Running it

```bash
docker compose up -d postgres     # from the repo root; listens on 127.0.0.1:5432
cd pokedex
cargo test                        # 13 integration tests, each in its own throwaway schema
cargo build --release

export DATABASE_URL=postgres://pokedex:pokedex@127.0.0.1:5432/pokedex
./target/release/pokedex init
./target/release/pokedex regulation add \
    --name "Regulation M-A" --effective-from 2026-04-08
./target/release/pokedex import \
    --file ingest/out/snapshot.regulation-m-a.json

# container (from the repo root); auto-migrates, DATABASE_URL points at the postgres service
docker compose run --rm pokedex status
docker compose run --rm -T pokedex import < pokedex/ingest/out/snapshot.regulation-m-a.json
```

As a library: `Db::connect(url)` → `db.migrate()` → `ingest::apply(db.client(), &snapshot)`, or
`ingest::apply_in(&mut tx, &snapshot)` to batch several snapshots into one commit.

CLI: `init | migrate | status | regulation add|list | import [--file X|stdin] [--dry-run]`.
Env: `DATABASE_URL`, `POKEDEX_AUTO_MIGRATE`. Tests: `POKEDEX_TEST_DATABASE_URL`.

## 3. Layout

```
pokedex/
  migrations/0001_init.sql        AUTHORITATIVE schema; reasoning inline at each decision
             0002_seed_types.sql  18 types + 324 chart rules   } GENERATED — regenerate,
             0003_seed_ref.sql    25 natures + 7 variant kinds  } do not hand-edit
  src/lib.rs      db · migrate · error · model · resolve · regulation · ingest
      main.rs     thin clap shell over the library
  tests/integration.rs            13 tests, fresh Postgres schema each
  ingest/                         OTHER THREAD: Showdown -> snapshot.json exporter (Node)
  Dockerfile .dockerignore        service definitions live in the root compose.yaml
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

All three interval tables share one invariant, enforced by a partial unique index: **at most one
open window per logical key.** Two open rows for a key is not bad data, it is broken resolution — the
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
| 4b | ~~`/data` mount~~ | superseded by rev 9: the container is stateless, state lives in Postgres | — |
| 5 | Temporal `pokemon_type` | **yes** | moderate |
| 6 | Fire/Fire UNIQUE | **dropped** — data-quality not structural; ingest checks it in ~3 lines | trivial |
| 7 | Megas | **generalized to `variant`/`variant_kind`** — real data proved this right, 46% of variants are not megas | moderate |

On #2, if it must ever be reversed: scope by **regulation**, not generation. A second independent
time axis forces "which generation is Regulation M-B?" into every query.

On #6: the dropped constraint only worked *paired* with atomic slot-set versioning — alone it
misses slot 1 opened in an earlier regulation and left open while slot 2 is later set to the same
type. Dropping the pair let the constraint simplify to one-open-per-slot and kept interval versioning
uniform across all three tables.

## 6. Gotchas

- **Integer columns are `BIGINT`.** The Rust side is `i64` throughout and the `postgres` driver
  refuses to decode `INT4` into `i64` (a runtime error, not a compile error). New columns that
  Rust reads as `i64` must be `BIGINT`, or cast in the query.
- **`regulation.effective_from` is `DATE`**; queries select it as `::text` so callers keep ISO
  strings. Rust still checks the `YYYY-MM-DD` shape, because Postgres also accepts `2025/1/1`.
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
- **Migrations take `pg_advisory_xact_lock`** and all pending ones run in one transaction, so
  several containers starting at once apply each migration exactly once (verified with three
  concurrent `migrate` runs).

## 7. Next

1. **`pokemon_legality` — the largest known hole.** "Regulation" in VGC is fundamentally about
   what is *legal*, but it was out of the original scope. Stats resolution gives a crude proxy
   ("exists from regulation X onward") but **cannot express removal-then-return** — megas legal in
   Gen 6-7, gone in Gen 8, back later. A scalar stat row resolves forward indefinitely, so a mega
   currently looks legal in regulations where it was not. Fix: an interval table reusing the
   half-open window shape and partial-unique-index pattern already defined. Cheap now, expensive once more data
   lands.
2. Wire `ingest/` and the Rust importer into one command so a regulation refresh is a single step.
3. One of 348 pokemon has no stat row (347 stored) — worth finding out which and why.
4. `genus` is null across the Showdown export; source it elsewhere if it matters.
5. Query surface — the pokedex half is read only through the views so far.
6. Link transcript lines to events (see §10): the tables exist, the linkage does not.
7. `variant.required_item` is still free text ('Charizardite X'); it could now reference
   `item(id)`, since every mega stone is an item row.
8. Regulations: M-A 2026-04-08, M-B 2026-06-17, M-C 2026-09-09 (current). M-C has no
   snapshot yet: Showdown's latest npm release (0.11.11, 2026-07-28) predates it. Until
   one is imported, M-C resolves to M-B's data carried forward, bans included.

## 8. Deliberate non-goals

Ability and item descriptions unversioned · type chart unversioned (decision 2) · individual/caught-pokemon
tracking (the only thing `nature` would join to) · two overlapping *closed* interval windows are
not caught by the partial unique index (unreachable via normal ingest; needs a deliberate backdated insert).

## 9. Rev 9: SQLite → Postgres (2026-09-25)

Moved to make deployment simpler: a managed or containerized Postgres instead of a database
file on a volume that has to live next to the one process writing it.

- Driver: synchronous `postgres` crate, `NoTls`. The library API stayed synchronous. The async
  `server` should call it through `spawn_blocking`, or query the database directly.
- Schema: same tables, views, and names. `INTEGER` became `BIGINT`, auto ids became
  `GENERATED ALWAYS AS IDENTITY`, `WITHOUT ROWID` was dropped, and `effective_from` became `DATE`.
  The six one-open-window triggers became three partial unique indexes
  (`*_one_open_window`): declarative, and safe under concurrent writers.
- Migrations are tracked in `schema_migrations` instead of `PRAGMA user_version`.
- Verified: 13/13 tests. The real M-A + M-B import reproduces rev 8's numbers exactly (348 /
  500 / 14,192; M-B wrote 38 stat rows and 0 move rows). Re-import is a no-op, and
  Electric→Gyarados 400 / Ground→Charizard 0.
- Nothing to migrate: the old `data/pokedex.db` only held test rows.
- Not done: TLS (needed for most managed Postgres hosts; add `postgres-native-tls` or
  `tokio-postgres-rustls` when a host is picked).

## 10. Rev 10: items, battles, transcripts (2026-09-25)

**Items (0004).** `item` holds identity and unversioned facts: slug, `display_name` as battle
text prints it, category (`mega-stone` / `berry` / `other`) and Fling power. `item_legality`
is an interval table, because legality is set membership: M-A has 117 legal items, M-B 148,
and an item can be banned and later return. The snapshot gained `items`, with the usual rule:
absent leaves legality alone, and a present list is complete.

**Battles (0005).** Written by `server/`, not by ingest. It is in the same migration chain so
that links from transcript lines can be real foreign keys into the pokedex.

```
video 1──1 battle 1──N transcript 1──N transcript_line
 upload     match     one per run       one per message line (seq, t0, t1, text, conf, clean)
```

- **Runs are kept, not replaced.** "Transcribe again" adds a `transcript` row, so links made
  against an older run's lines keep pointing at real rows. The newest `done` run is current.
- A partial unique index allows at most one queued or transcribing run per battle.
- `battle.regulation_id` is NULL until something determines it.
- `battle.video_id` is UNIQUE for now; drop that if one recording ever holds several battles.
- The server applies migrations at startup through `pokedex::Db::migrate`, run on a blocking
  thread. The advisory lock makes that safe alongside the CLI.
- Redis is gone. Its one video and 16-line transcript were copied into Postgres. A
  re-transcription then produced a second run of 16 lines; one line changed, now reading
  "It’s" where the old run had "lt’s".

---

Tags: database, development
