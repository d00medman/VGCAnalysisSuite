# TODO

Running list of things we want to do. Details live in the linked devlogs.

- [ ] **GPU video decode.** Try NVDEC hardware decode to speed up video processing. See
  [AnalyzerPerformance.md §4d](devlog/AnalyzerPerformance.md) ("Server GPU decode (NVDEC)"),
  and measure CPU cost per video first as described in §5.
- [ ] **Richer GUI log text from the pokedex.** Use the `pokedex/` database to add details
  (names, types, moves, etc.) to the transcript lines shown in the web GUI's logs.
- [ ] **Give formes their base form's moves.** 124 formes have empty learnsets in every
  regulation: 82 Megas, 7 in-battle forms (Aegislash-Blade, Mimikyu-Busted, …) and 35
  cosmetic forms (Vivillon, Alcremie, …). Showdown stores their moves on the base form.
  Plan: in `pokedex/ingest/showdown-snapshot.js`, fall back to `changesFrom` / `battleOnly`
  / `baseSpecies` when a form has no learnset; re-run `export.sh`; then rebuild dev's
  pokedex (M-A and M-B are locked now that M-C is in): `scripts/backup-battles.sh`,
  recreate the DB, `scripts/rebuild-pokedex.sh`, restore the backup, re-export
  `pokedex/data/`. See [SchemaIteration.md](devlog/SchemaIteration.md) §4.
- [ ] **Trainer table (the user table).** Next big core-schema change. Battle data will point
  at it, so it's irreplaceable-side work: follow the workflow in
  [SchemaIteration.md](devlog/SchemaIteration.md) §4 (back up battles, iterate on a
  throwaway DB, commit before dev applies it), and consider the runner and scratch-DB
  proposals in §5 first.
