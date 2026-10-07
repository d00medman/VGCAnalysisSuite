# 003: HP panel reader — species from panel icons

Status: Draft · Rev 1 · 2026-10-07
Serves: vision §5 capability 4 (derived data) and 3 (stats); §10 "Non-Latin names". It
also covers your own nicknamed Pokémon.

`Decided:` marks things agreed with the user. `Open:` marks questions still to settle.

## Problem

The transcript names Pokémon by whatever the battle text says. That's often not the species:
- **Nicknames.** Your own (`Big Man`, `Smaug`, `Merlin`, `Titania`) and opponents'. In the
  Batch 0 battle analysis, several of your Megas were counted under nicknames.
- **Non-Latin nicknames** come out as `?`. In PMBT5875, 22 lines don't say which opposing
  Pokémon acted.
- **Team preview isn't always recorded,** so it can't be the only source of species.

Every stat that matters, from usage and win rate by Pokémon to matchups, needs the species.

## What's on screen

Each active Pokémon has an HP panel in a fixed place: your two at the bottom left, the
opponent's two at the top right. Each panel shows:
- **A small 2D species icon,** pixel-identical from frame to frame in the samples so far
  (`sandbox/frames/panels.png`).
- **The name.** `Open:` nickname or species? See question 4.
- **HP:** a percentage for the opponent's Pokémon, exact HP for yours.
- **A gender symbol, and status icons** (e.g. `Zz` for asleep).

The panels are up every turn during move selection, in every recording.

## Behaviour

- **Species per slot.** For each battle, the analyzer reports which species is in each of
  the four panel slots over time, identified from the icon alone. That makes it
  independent of nickname, script and team preview.
- **Nicknames resolve to species.** Each nickname (or `?` name) in the transcript maps to
  a species, so a line like `The opposing ??? used Protect!` can be attributed, as can
  `Go! Big Man!`.
- **Unknown icons stay unknown.** An icon the library has never seen is reported as
  unknown, never as a best guess, and its crop is saved so it can be labelled.
- **The icon library is built from your recordings.** When the transcript names a
  species in readable text ("sent out Garchomp!"), the icon that appears is a labelled
  example, so most labels need no hand-labelling.

## Scope

**In:**
- Locating the four panels and cropping their icons.
- Matching icons against a template library.
- Building that library from the corpus, with transcript-derived labels plus hand labels for
  gaps.
- Mapping names to species per battle.

**Out (non-goals, for now):**
- **Reading HP, status and gender from the panels.** It's natural next work in the same
  spot; `Open:` see question 2.
- **Team preview** (who was brought but never sent out). It stays on the roadmap as a
  complement.
- **Identifying the 3D battle models.**
- **Showing names in their real script** (spec 002, parked).

## Acceptance criteria

Draft, with thresholds to settle in the plan.

1. **Accuracy.** On a hand-checked set of panel crops covering both sides, many species,
   and highlighted and plain panels, at least `Open:` 98% are identified correctly. No
   icon is ever given a wrong species with high confidence.
2. **Nickname mapping.** In Batch 0, every Pokémon you nicknamed maps to the right species,
   and so do the opposing Pokémon in PMBT5875 (the battle with non-Latin nicknames).
3. **Coverage.** The library covers every species seen in Batch 0 and Batch 1. Unknown
   icons are listed with saved crops.
4. **Unchanged transcripts.** Existing transcript text is unaffected; the species data is
   added alongside.
5. **Re-run analysis.** The Batch 0 battle analysis, re-run with species, no longer counts
   any Pokémon under a nickname.

## Data

- **The icon template library is reference data:** committed and rebuildable from labelled
  crops, like the glyph atlas.
- **Per-battle species readings** are derived and regenerable from video, so they need no
  backup. `Open:` where they're stored (plan).

## Open questions

Blocking:

1. **How much hand-checking for the accuracy set?** About 100 crops is roughly 15 minutes of
   review. I'd prepare a contact sheet with proposed labels, and you'd confirm or correct
   them.
2. **Species only first, or HP % and status too?** Species is the goal. HP and status
   share the crop and add derived data, but they need their own small glyph set (an
   italic digit font). I'd do species first, then HP and status as a follow-up.
3. **Shiny Pokémon.** A shiny's icon has different colours, which matters if matching uses
   colour. Do shinies show up in your games, and should a shiny count as its species?

Not blocking (answered in the plan, from samples):

4. **Does the panel show the nickname or the species?** If it's the species, reading the
   panel *name* with the existing text reader is a second, cheaper source to cross-check
   the icon. TJSR2480 (`Big Man` at ~04:24) will show it.
5. **Panel geometry:** exact boxes, the highlight glow during your selection, slide-in and
   slide-out frames, and what a fainted or empty slot looks like.
6. **Megas and alternate forms** (Rotom forms, regional forms) need their own templates:
   either labelled as their form, or folded into their species for stats.
7. **Where the reader runs:** inside the existing transcription pass, which decodes each
   frame once anyway (cheapest), or as a separate pass.
