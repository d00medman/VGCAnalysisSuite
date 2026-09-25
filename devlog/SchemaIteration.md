# Schema Iteration — lessons from the M-C harmonization

Rev 1 · 2026-09-25 · **How to keep changing the schema ad hoc without losing ground truth**

Schema work here happens as ideas arrive, not in planned releases. This records what made
2026-09-25 painful and what should make the next change cheap. `Proposal:` marks things not
yet built or agreed; **[unverified]** = not checked in-session.

---

## 1. What went wrong

**A half-written migration became permanent.** Three sessions shared one checkout. One of
them rebuilt the api container while another had `0007_turns` half-written in the tree.
The API migrates on every start (`server/src/store.rs`, `Store::connect`), so the dev DB
recorded 0007 as applied at 15:37. From then on, 0007 could not be changed, only added to.
The full story is in the `CLAUDE.md` rules that came out of it (`35372e2`).

**The dev DB was the only copy of the pokedex.** Getting M-C in meant reconciling four
things that had quietly drifted apart:

- **What's in the DB depended on import history.** M-A and M-B had been imported from
  snapshots in the gitignored `ingest/out/`, made by an older exporter. Nothing in git
  could reproduce the database.
- **Showdown moved underneath us.** Master renamed its mods (`champions` went from M-B to
  M-C), deleted `championsregma`, and moved descriptions off the data objects. A naive
  re-export would have labelled M-C data as M-B and nulled 464 descriptions.
- **Ingest is one-way.** It refuses a snapshot older than one already loaded, so M-B's
  corrections (28 × Slash, Politoed's Pound) had to go in before M-C, or never.
- **Some facts lived only in the DB.** `regulation.notes` was written by hand. The M-C note
  ("no snapshot yet: resolves to M-B data carried forward") goes false the moment M-C is
  imported, and no script knows it exists.

## 2. The lesson: two kinds of data, two rules

| | Derived | Irreplaceable |
|---|---|---|
| Tables | the 16 pokedex reference tables | `video`, `battle`, `transcript`, `transcript_line`, `turn_end` |
| Source of truth | `pokedex/snapshots/` in git | only the database |
| If lost | `scripts/rebuild-pokedex.sh`, **~21 s** | gone; turn marks are hand-made |
| Schema change cost | throw it away and rebuild | needs a migration that preserves rows |

**Derived data should never need migration discipline.** If the shape of `pokemon_move`
changes, the honest fix is: edit the schema, rebuild from snapshots, and diff the CSVs. A
data-preserving migration for data we can regenerate in 21 seconds is effort spent
protecting nothing.

**Irreplaceable data is the only reason migrations are append-only.** So the question to
ask of any schema idea is: *does it touch a battle table, or a column one of them points
at?* If not, iterate freely.

As of today the derived side really is disposable. The snapshots are committed with their
Showdown source, the rebuild script is proven on a throwaway Postgres, and the
reference CSVs round-trip identically through `export-reference.sh` and
`load-reference.sh`.

## 3. Specific lessons

1. **A migration is published the moment any process runs it.** The API auto-migrates, and
   container builds take whatever is in the working tree. So "uncommitted" is no
   protection: WIP SQL can land in the dev DB from someone else's rebuild.
2. **The runner can't tell when history is rewritten.** `migrate.rs` numbers migrations by
   list position and stores the name, but never compares it. Edit an applied migration and
   the edit is silently skipped. Insert one mid-list and the DB believes it ran, while the
   last one never does. Nothing errors.
3. **Anything typed into the DB by hand is drift.** `regulation.notes` is the example, and
   it's why a fresh rebuild differs from dev by exactly two lines. If it matters, it
   belongs in a script or a snapshot.
4. **Irreplaceable rows must not point at derived rows by surrogate id.**
   `battle.regulation_id` stores a raw id. It survives a rebuild only because regulations
   happen to be inserted oldest first (1 = M-A, …). Nothing makes pokemon or move ids
   stable: `load-reference.sh` assigns fresh ones, and so does any change to insert order.
   A battle table referencing `pokemon.id` could silently mispoint after a rebuild.
5. **Natural keys make a diff a test.** The reference CSVs carry no ids, so exporting before
   and after any change shows exactly what changed as game facts. For a schema refactor
   that shouldn't change data, "rebuild, export, `git diff` is empty" is the regression
   test. It caught the notes drift above.
6. **One-way ingest makes fixing history a race, and rebuild removes the race.** Correcting
   an old regulation in place is only possible until a newer one lands. Correcting the
   snapshot and rebuilding is always possible.
7. **Pin the tools, not just the data.** Host Node was 18 but Showdown needs 22, and host
   `pg_dump` 13 refuses a Postgres 17 server. Every script now runs its toolchain in a
   pinned container.

## 4. Workflow for an ad-hoc schema change

1. **Back up.** Run `scripts/backup-battles.sh <dev-url>`. It takes seconds and is the only
   copy of turn marks.
2. **Experiment off the shared DB.** Start
   `docker run --rm -d -p 127.0.0.1:55432:5432 … postgres:17-alpine`, then use
   `rebuild-pokedex.sh` against it. Restore the latest battle backup if the change touches
   battle tables. Never point the api at it through compose (see `CLAUDE.md`).
3. **Iterate on the migration freely** while it has only touched throwaway databases: edit
   it, drop the container, go again.
4. **Prove it.** Rebuild, then `export-reference.sh` into the repo, and check that `git diff
   pokedex/data/` shows only the change you intended.
5. **Commit, then let dev apply it.** Only a committed migration should ever reach the
   shared DB.

## 5. Proposals, cheapest payoff first

1. **`scripts/scratch-db.sh`.** One command for steps 2–3: a throwaway Postgres on 55432,
   rebuilt from snapshots, with the latest battle backup restored. It's already done by
   hand twice today; roughly 30 lines.
2. **Make the runner notice rewrites.** Store a checksum per migration, refuse to start if
   an applied migration's name or checksum changed, and key by name, not position. The
   last part also removes the numbering race between branches (retro follow-up 1). A
   refusal is loud, where today's failure is silent.
3. **Treat reference-table migrations as editable before release.** They're derived, so a
   change to one can be applied by dropping the reference tables, re-running migrations,
   and rebuilding. That's awkward with one linear migration list. The simpler form is
   **periodic squashing**: while there's only a handful of battles, fold 0001–000N into
   one baseline after taking a backup. **[unverified]** The catch is that a data-only
   battle dump restores only into a schema whose battle tables have the same shape, so
   squashing is free only between battle-table changes.
4. **Reference derived data by natural key from battle tables.** For example
   `battle.regulation` holds the name, or at least a test asserts regulation ids are stable
   across rebuilds. Do it before any battle table references `pokemon` or `move` (a parsed
   transcript naming species is the obvious next one).
5. **Stop the API from migrating on start**, or gate it behind a flag the dev stack sets
   explicitly, and stamp images with the git commit and a dirty flag (retro follow-ups
   2–3). Rebuilding containers only from committed `main` closes lesson 1 without tooling.
6. **Move `regulation.notes` into `rebuild-pokedex.sh`** (or the snapshot), so the dev DB
   and a rebuild agree exactly.

Retro follow-ups 4 (reference CSVs, battle backups) and 5 (committed snapshots, bootstrap
script) are done: `f5caa73`, `2af02e7`.
