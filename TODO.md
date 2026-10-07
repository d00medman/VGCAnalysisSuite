# TODO

Running list of things we want to do. Details live in the linked devlogs.

- [ ] **Per-branch Docker stacks.** A script, e.g. `scripts/stack.sh up <branch>`, that
  runs a branch's own compose stack beside the main one:
  - its own project name (`docker compose -p`) and free ports
  - its own database, seeded the way `scripts/scratch-db.sh` does (pokedex rebuilt, latest
    battle backup restored)
  - prints the stack's URL

  Then parallel sessions, or the user, can run and compare several branches, and a branch's
  migrations never touch the dev database. Lifts the "Docker only from the repo root" rule
  in `CLAUDE.md`. Related: stamping images with their git commit (parallel-sessions retro).
- [ ] **GPU video decode.** Try NVDEC hardware decode to speed up video processing. See
  [AnalyzerPerformance.md §4d](devlog/AnalyzerPerformance.md) ("Server GPU decode (NVDEC)"),
  and measure CPU cost per video first as described in §5.
- [ ] **Richer GUI log text from the pokedex.** Use the `pokedex/` database to add details
  (names, types, moves, etc.) to the transcript lines shown in the web GUI's logs.
- [x] **Trainer table (the user table).** Done 2026-10-06: `specs/001-trainer-table`. Next big core-schema change. Battle data will point
  at it, so it's irreplaceable-side work: follow the workflow in
  [SchemaIteration.md](devlog/SchemaIteration.md) §4 (back up battles, iterate on a
  throwaway DB, commit before dev applies it), and consider the runner and scratch-DB
  proposals in §5 first.
- [ ] **Mark a battle's result by hand.** Let the user set won/lost at the end of a battle in
  the GUI, like turn marks. Needed because a recording sometimes ends right after a forfeit,
  before the defeat message reaches the screen, so the transcript never shows the result.
  Hand-set results live only in the DB, so they're battle data (include them in
  `scripts/backup-battles.sh`).
  - [ ] **Then: detect the forfeit from the video.** Recognise the user pressing the
    forfeit button in the UI, and set the result automatically.
- [ ] **Damage calculator page.** A page in the web GUI that works out a move's damage from
  attacker to defender, using the pokedex's stats, types, moves and type chart for the
  selected regulation. Level-50 stats come from base stat + SP + nature (IVs are fixed at
  31 in Champions). Once teams are logged (below), let it fill in the user's own Pokémon.
- [ ] **Sprite data.** Add sprites to the pokedex so the GUI can show them. The pokedex's
  names are PokeAPI-style slugs, so it can join to PokeAPI's sprites without a lookup
  table (see the naming note in `pokedex/ingest/showdown-snapshot.js`).
- [ ] **Identify Pokémon on screen from pixels.** Started as `specs/003-hp-panel-reader`: the
  2D species icon in each HP panel, which is far easier than the 3D models. Optional, and possibly a lot of work.
  Recognise which Pokémon are on screen from the video frames, not just from the message
  text. Would likely build on the sprite data and the analyzer's approach of matching
  images against stored templates.
- [ ] **Trainers' Pokémon: log teams.** A table of the trainer's own Pokémon, so a trainer can
  log their teams: each Pokémon's moves and stat distribution (SP spread and nature), plus
  ability and item. Depends on the trainer table above. It's user-entered data that can't be
  rebuilt, so it belongs with the battle data in `scripts/backup-battles.sh`.
- [ ] **Store data inferred from transcripts.** Work out structured facts from transcript lines
  (which Pokémon appeared, which moves they used, and what that reveals, such as an
  opponent's moves or item), and store them in the database, linked to the battle.
