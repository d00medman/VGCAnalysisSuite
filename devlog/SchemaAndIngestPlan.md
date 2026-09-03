# Pokedex — Schema Rationale & Ingest Plan

Date: 2026-09-03 (rev 5)
Status: **migrations written and validated. Rust + Docker not started.**
Location: `/home/barbaroja/Videos/pokemon_recordings/pokedex/`

## Source of truth

**The migrations are authoritative for _what_ the schema is. This file is authoritative for
_why_.**

```
pokedex/migrations/0001_init.sql       14 tables, 7 views, 6 triggers, 5 indexes
pokedex/migrations/0002_seed_types.sql 18 types + 324 chart rules  (GENERATED — regenerate, don't hand-edit)
pokedex/migrations/0003_seed_ref.sql   25 natures + 7 variant kinds (GENERATED)
```

Revs 1-4 of this file carried full DDL in prose. It was hand-typed, never parsed, and had at
least one real bug (`PRAGMA foreign_keys` inside a migration is a no-op — it must be set
per-connection in `Db::open`). **That DDL has been deleted from this file rather than kept in
sync.** A drifted spec is worse than no spec. Read the SQL for structure; read this for
reasoning. The migration files carry the same reasoning as inline comments at each decision
point, so the SQL is self-explaining if you never open this file.

### Validation already performed (in-memory, no artifacts)
All three migrations parse and apply cleanly on sqlite3 3.37.2. Verified semantically:
- Electric→Gyarados = 400, Ground→Charizard = 0, Dragon→Mega Charizard X = 200
- Mega Charizard X (BST 634) resolves different stats than base Charizard (534) in the same
  regulation, through the ordinary path
- Charizard stores **one** stat row and resolves correctly in two regulations, both reporting
  `sourced_from_regulation = 'Reg A'`
- A pokemon whose first stat row is in Reg C is entirely absent from Reg A
- A retroactively inserted Reg B re-linked the window chain with no data migration
- Second open window on an occupied slot → trigger ABORT
- Chart distribution: 8 immunities / 61 resists / 51 super-effective / 204 neutral (correct Gen 6+)

## Context

SQLite pokedex + a Rust tool to populate it, usable as **both** a CLI and a library module in a
larger pipeline. Scope: schema + migrations + insertion. Downstream consumption is out of scope.

Environment: cargo 1.94.0, sqlite3 3.37.2, Linux, not a git repo.
House style from sibling `../importer/` (`pokemon-import`): edition 2021, clap-derive + anyhow
+ serde.

Assumption (stated, never contradicted): the spec line "A *type* will have a single type, a base
damage, an accuracy and a secondary-effects column" describes a **move**.

## Organizing principle

Regulations make this a **temporal** schema. Every versioned fact is one of two shapes. Picking
the wrong one is the main way this design goes bad.

| Shape | Question | Storage | Tables |
|---|---|---|---|
| **Scalar** | "what was the value at regulation R?" | sparse rows; *latest at or before R* wins | `pokemon_stats`, `move_data` |
| **Set membership** | "was this in the set at R?" | half-open interval `[valid_from, valid_to)` | `pokemon_type`, `pokemon_ability`, `pokemon_move` |

Scalar tables **cannot express removal** — "latest wins" always finds something. Interval tables
can, via `valid_to_regulation_id`. That is the entire reason for the split.

All three interval tables share one invariant, enforced by trigger: **at most one open window per
logical key.** Two open rows for a key is not bad data, it is broken resolution — the
effectiveness view would return two slot-1 rows for one regulation and silently duplicate
results.

## Reasoning notes (structure lives in the SQL)

**`regulation.effective_to` is derived by `LEAD()`, never stored.** Storing both ends forces every
insert to patch its predecessor; one missed patch silently creates an overlap or gap. Deriving
makes those unrepresentable *and* lets a regulation be inserted retroactively between two
existing ones with the chain re-linking itself. `UNIQUE (effective_from)` is what makes the
ordering total and the linked list well-defined — **do not drop it.** ISO-8601 TEXT sorts
lexicographically = chronologically, so there are no date functions anywhere in the schema.

**Type chart is dense (324 rows), integer percent, unversioned.** Integer `multiplier_pct` makes
dual-type stacking `(a*b)/100` — exact, no float comparison. Dense costs 10KB over sparse's 4KB
and buys a plain `JOIN` at every use site; sparse would need `LEFT JOIN` + `COALESCE(...,100)`,
and **a forgotten COALESCE does not error**, it silently drops neutral matchups from a damage
calc.

**`pokemon` is identity only; `variant` generalizes megas.** A variant is still its own `pokemon`
row, so stats/types/abilities/learnsets need zero special-casing — differing stats for Mega
Charizard X and Alolan Ninetales fall out of the ordinary path. Presence in `variant` is what
makes something a variant; absence means base form. This replaced a
`form_kind`/`base_form_id`/`required_item` trio on `pokemon` plus a `CHECK ((form_kind='base') =
(base_form_id IS NULL))` — the bad state is now unrepresentable rather than merely checked. The
self-FK means **base rows must be inserted before their variants** (ingest two-pass).

**`move_data.type_id` is on the versioned side on purpose** — move types change (Bite went
Normal→Dark). Only `name` is identity. NULL `accuracy` = never misses (Swift, Aerial Ace); NULL
`power` = status/variable; both distinct from `0`.

**`pokemon_stats_effective` uses an inner JOIN, not LEFT — that is load-bearing.** A pokemon with
no stat row at or before R does not appear in R at all, so species introduced later correctly
vanish from earlier regulations for free.

**Resolution views CROSS JOIN `regulation` and must be queried with a filter** (`regulation_id`
and/or `pokemon_id`). An unfiltered scan of `pokemon_stats_effective` is O(pokemon × regulations)
correlated subqueries.

**Natures will look orphaned.** They are a per-*individual* trait, not a species trait, so nothing
in this schema joins to `nature`. Correct and unused until an individuals table exists.

**`variant_kind`, not `variant_type`** — `type` already means elemental type here, and
`variant.type_id` beside `pokemon_type.type_id` is a live footgun in queries.

## Decision record

| # | Decision | Chosen | Reversal cost |
|---|---|---|---|
| 1 | Type chart density | **dense**, 324 rows | trivial |
| 2 | Chart generation/regulation scoping | **none** — modern Gen 6+ chart only | **HIGH** (enters a PK, every effectiveness query gains a dimension, seed rewritten) |
| 3 | Seed natures | **yes** — static since Gen 3 | trivial |
| 4a | Runtime base image | **debian:bookworm-slim** (shell for debugging; revisit if this ships to others) | trivial |
| 4b | `/data` mount | **bind mount** + `user:` — host `sqlite3` access matters while developing | trivial |
| 5 | Temporal `pokemon_type` | **yes** | moderate |
| 6 | Fire/Fire UNIQUE | **dropped** — data-quality, not structural; ingest checks it in ~3 lines | trivial |
| 7 | Megas | **generalized to `variant`/`variant_kind`** | moderate |

On #2, if it ever must be reversed: scope the chart by **regulation**, NOT by generation. A
second independent time axis would force "which generation is Regulation G?" into every query.

On #6: the dropped constraint was `UNIQUE (pokemon_id, type_id, valid_from)`, which **only works
paired with atomic slot-set versioning** — alone it misses the case where slot 1 was opened in an
earlier regulation and left open while slot 2 is later set to the same type. Dropping the pair is
what let the trigger simplify to one-open-per-slot and kept interval versioning uniform across all
three tables.

## Ingest plan (NOT YET IMPLEMENTED)

### Contract: feed dense, store sparse
The caller submits a **complete snapshot for one regulation** — every stat, move, ability,
learnset entry. The ingest layer diffs:
- **Scalar tables:** resolve the value as of the previous regulation; write a row only if it
  differs. Unchanged pokemon cost zero rows.
- **Interval tables:** in snapshot but not in open set → open an interval. In open set but not in
  snapshot → close it (`valid_to_regulation_id = <this regulation>`). Unchanged → untouched.
  **Close before open** — the trigger enforces it.

The pipeline never computes deltas or tracks what changed; sparseness is the storage layer's
problem. Re-submitting an identical snapshot is a no-op, so idempotency holds.

### Validation that must live in Rust (SQLite does not declare it)
- slot 2 requires slot 1 (types)
- no duplicate type across slots (replaces the dropped UNIQUE — see decision 6)
- base pokemon inserted before its variants (self-FK ordering — the two-pass)

### Record format — **OPEN, blocks the Rust phase**
Named but never specified through rev 4. A fresh session would invent a different shape. Draft
for approval:
```json
{
  "regulation": "Regulation G",
  "abilities": [ {"name":"blaze","description":"Powers up Fire-type moves in a pinch."} ],
  "moves": [ {"name":"flamethrower","type":"fire","damage_class":"special","power":90,
              "accuracy":100,"pp":15,"priority":0,
              "secondary_effect":"May burn the target.","effect_chance":10} ],
  "pokemon": [
    {"national_dex_no":6,"form_slug":"","name":"Charizard","genus":"Flame Pokémon",
     "height_dm":17,"weight_hg":905,
     "variant": null,
     "stats":{"hp":78,"attack":84,"defense":78,"sp_attack":109,"sp_defense":85,"speed":100},
     "types":["fire","flying"],
     "abilities":{"primary":"blaze","secondary":null,"hidden":"solar-power"},
     "learnset":[{"move":"flamethrower","method":"level-up","level":46},
                 {"move":"fly","method":"machine"}]},
    {"national_dex_no":6,"form_slug":"mega-x","name":"Mega Charizard X",
     "variant":{"base_form_slug":"","kind":"mega","required_item":"Charizardite X"},
     "stats":{"hp":78,"attack":130,"defense":111,"sp_attack":130,"sp_defense":85,"speed":100},
     "types":["fire","dragon"],
     "abilities":{"primary":"tough-claws","secondary":null,"hidden":null},
     "learnset":[]}
  ]
}
```
Shape rationale: `types` is an **ordered array** so slot 1/2 is positional and "slot 2 requires
slot 1" is unrepresentable rather than validated. `abilities` is an object with named slots.
Moves and abilities are defined **once** at top level and referenced by name from each pokemon,
so a 1000-pokemon snapshot doesn't repeat move definitions. `variant.base_form_slug` identifies
the base within the same `national_dex_no`.

## Containerization (NOT YET IMPLEMENTED)

DB is a file → the container is stateless and state lives on a mount.
- **`/data` is the state boundary.** Defaults to `/data/pokedex.db`, overridable via `--db` or
  `POKEDEX_DB` → clap needs the `env` feature:
  `#[arg(long, env = "POKEDEX_DB", default_value = "/data/pokedex.db")]`.
- **Migrations are absent from the runtime image** — `include_str!`'d at compile time. The final
  image is one binary. No mount, no image/disk drift.
- **No shell in the entrypoint**, so "migrate then run" cannot be a wrapper script →
  `POKEDEX_AUTO_MIGRATE=1` handled inside the binary.
- Multi-stage `rust:1.94-slim-bookworm` → `debian:bookworm-slim`, non-root uid 10001.
- **BuildKit cache mounts, not cargo-chef.** The `install` into `/usr/local/bin` must be in the
  *same* `RUN` — `target/` is a cache mount and does not persist into the layer.
- **No `VOLUME ["/data"]`.** It creates an ungarbage-collected anonymous volume on every
  `docker run` and freezes the path. Declare the mount at run time.
- **`--locked`** so container builds cannot resolve a different dep graph than local.
- compose uses `user: "${UID:-1000}:${GID:-1000}"` + `./data:/data`, with `.env` carrying
  `UID`/`GID` — shells do not export those by default.
- `.dockerignore`: `target/`, `data/`, `*.db`, `*.db-wal`, `*.db-shm`, `.git`. Excluding
  `target/` matters — sibling `../importer/target` is large and would dominate build context.

### Two gotchas
1. **WAL + bind mounts.** WAL needs shared memory (`-shm`) and working POSIX locking. Fine on
   native Linux bind mounts; **breaks on Docker Desktop macOS/Windows FS shims.** Linux here, so
   fine — but journal mode is configurable via `POKEDEX_JOURNAL_MODE` so falling back to `DELETE`
   is an env var, not a rebuild.
2. **UID mismatch.** Container is uid 10001, host dir is uid 1000 → cannot write. SQLite needs
   write permission on the **directory**, not just the DB file, to create `-wal`/`-shm`/journal
   siblings — chmod'ing the file alone will not help. Hence `user:` + `.env`.

## Rust layout (NOT YET IMPLEMENTED)
```
Cargo.toml   rusqlite(bundled), serde, serde_json, clap(derive,env), anyhow, thiserror
src/lib.rs  db.rs  migrate.rs  error.rs  model.rs  regulation.rs  resolve.rs  ingest.rs
    main.rs                                    # [[bin]] name = "pokedex"
```
- **Library is the API; CLI is a shell over it.** `Db::open` sets `foreign_keys=ON`,
  `journal_mode`, `busy_timeout` — note `foreign_keys` MUST be here, not in a migration.
- **`Db::open` does NOT auto-migrate** — the library caller decides. Only the CLI honors
  `POKEDEX_AUTO_MIGRATE`.
- **Records reference types/moves/abilities by name, not ID.** `resolve.rs` holds cached name→id
  maps and errors on unknown names. Pipeline stages must not need surrogate keys.
- Migration runner: `include_str!` the SQL, apply under `PRAGMA user_version` in a transaction.
  Hand-rolled (~40 lines) rather than refinery/sqlx — keeps it sync and dep-light.

### CLI surface
```
pokedex init | migrate | status
pokedex regulation add --name <n> --effective-from <YYYY-MM-DD> | regulation list
pokedex import --regulation <name> [--file X | stdin] [--dry-run]
```
Regulations cannot be seeded (names/dates are research output) but the DB is unusable with zero
regulations → `regulation add` is required plumbing, not a convenience.

## Plan of attack

Ordering set by the user: migrations first (written, not applied), then Rust + Docker, then test
in conjunction.

| # | Phase | Status |
|---|---|---|
| 1 | Migrations `0001`-`0003`, syntax + semantics validated in memory | **DONE** |
| 2 | Settle the JSON record format | **BLOCKS phase 4** |
| 3 | Scaffold + Docker: `cargo init`, Cargo.toml, Dockerfile, compose, `.env`, `.dockerignore` | not started |
| 4 | Rust core: `db.rs`, `migrate.rs`, `error.rs`, `model.rs`, `resolve.rs`, `regulation.rs` | not started |
| 5 | Rust ingest: scalar diffing, interval open/close, the 3 validations | not started |
| 6 | CLI wiring | not started |
| 7 | Integrated test: full flow from empty dir, natively **and** in-container | not started |

### Phase 7 verification targets
Marked ✓ where already proven against the raw SQL in phase 1; those still need re-proving through
the Rust path.
- ✓ Electric→Gyarados = 400, Ground→Charizard = 0
- ✓ Variant resolves different stats than its base in the same regulation
- ✓ Late arrival absent from earlier regulations
- ✓ Retroactive regulation re-links windows
- ✓ Trigger ABORTs a second open window
- Identical snapshots for two regulations → second adds **zero** `pokemon_stats` rows
- Change one stat in reg 2 → correct old/new resolution and `sourced_from_regulation`
- Drop a move from a reg-2 learnset → interval closes; present in reg 1, absent in reg 2
- A one-move learnset change does NOT rewrite other learnset entries
- Full re-import is a no-op
- `pokedex init` in a clean container, bind mount, correct file ownership on the host

## Known gaps / deliberate non-goals

- **Legality is not modelled — largest known hole.** "Regulation" in VGC is fundamentally about
  what is *legal*, but it was not in the request. Stats resolution gives a crude proxy ("exists
  from regulation X onward") but **cannot express removal-then-return** — megas were legal in
  Gen 6-7, gone in Gen 8, back later. A scalar stat row resolves forward indefinitely, so a mega's
  early stat row makes it look legal in regulations where it was not. Fix: a `pokemon_legality`
  interval table reusing the half-open window shape and trigger pattern already defined. ~1 hour
  if folded in before data exists; considerably more after.
- Ability descriptions unversioned.
- Type chart unversioned (decision 2).
- Individual/caught-pokemon tracking — the only thing `nature` would join to — out of scope.
- Two overlapping *closed* interval windows are not caught by the trigger (unreachable via normal
  ingest; would require a deliberate backdated insert).

---

Tags: database, development
