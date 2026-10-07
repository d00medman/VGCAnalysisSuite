# 002: Non-Latin names and nicknames

Status: Parked · Rev 1 · 2026-10-07

**Parked 2026-10-07.** For stats, what matters is *which Pokémon* acted, and reading a
non-Latin nickname correctly still wouldn't say. Species identification from the HP-panel
icons (`003-hp-panel-reader`) solves that in any script, so it went first. This spec now only
covers showing names in their real characters.
- **What would bring it back:** wanting opponent names (e.g. stats by opponent), or
  nicknames shown properly in the GUI.
- **Starting point:** harvested samples in `sandbox/cjk/` (`sandbox/cjk-targets.tsv` lists
  30 moments across 14 Batch 0 battles), which show which scripts appear.
Serves: vision §5 capability 1 (transcripts), and §10 "Non-Latin names".

`Decided:` marks things agreed with the user. `Open:` marks questions still to settle.

## Problem

The reader's glyph atlas is Latin only. When a player's name or a Pokémon's nickname is
Japanese, Chinese or Korean, every character comes out as `?`.

In Batch 0 that's **13 of 115 battles (about 11%)** with an unreadable opponent name. In 2
more, opposing Pokémon have non-Latin nicknames, so lines like
`The opposing ? ?? ?? ? used Dazzling Gleam!` lose which Pokémon acted (22 lines in
PMBT5875 alone). These are the largest remaining group of unclear lines
(devlog/TranscriptQuality.md §5).

Two things are wrong today:
1. **No templates.** The atlas has nothing to match these characters against.
2. **Characters are split into pieces.** `text.rs` merges only vertically stacked blobs.
   Many kana, kanji and hangul are side-by-side pieces (い, 川, 사), so one character
   becomes several "glyphs", and a spurious space can appear between them (`??? ??????`).
   See `TODO(cjk)` in `text.rs`.

## Behaviour

- **Names read correctly.** A non-Latin player name or nickname appears in the transcript
  as its real characters, e.g. `たろう sent out Garchomp!`.
- **Each character is one character,** however many pieces it's drawn with.
- **Latin text is unaffected.**
- **No silent loss.** A character that still can't be read stays `?`, never dropped. This
  is the reader's existing rule.

## Scope

**In:**
- The scripts the samples show. `Open:` see question 1.
- Segmentation that groups a multi-piece character into one glyph.

**Out (non-goals):**
- **Species behind a nickname.** Reading a nickname correctly still doesn't say which
  Pokémon it is: "sent out たろう!" names the nickname, not the species. That needs team
  preview or sprite recognition (vision §10), a separate feature.
- Non-English *game* text (the game's language set to Japanese, etc.).

## Acceptance criteria

Draft, to be firmed up once the approach is chosen.

1. On a labelled set of harvested non-Latin name crops, at least `Open:` N% of names read
   exactly.
2. All Latin labelled crops still read exactly as before (64 of 66 today).
3. A player's name reads the same in every line of a battle.
4. Batch 0's unclear lines drop by roughly the name lines fixed. No clean line becomes
   unclear.

## Data

Atlas and lexicon files (committed, rebuildable). No database change.

## Open questions

Blocking:

1. **Which scripts actually appear?** The samples being harvested now (one crop per affected
   battle, `sandbox/cjk/`) will show Japanese kana, kanji, Chinese or Korean.
   - **Why it decides the approach:** kana is about 170 characters. Kanji, Chinese and
     Korean run to thousands.
2. **Approach.** It depends on 1:
   - **a. Hand-label harvested crops**, as for the Latin atlas.
     - **For:** exact templates.
     - **Against:** only covers characters someone has seen and labelled. Fine for kana,
       hopeless for thousands of kanji or hangul.
   - **b. Render templates from a font.** Generate every character's template at the game's
     size from the game's font or a close match (e.g. Noto Sans CJK).
     - **For:** covers whole scripts at once.
     - **Against:** accuracy depends on how closely the font matches the game's. We need to
       identify the font first.
   - **c. Send the unknown span to an OCR engine** (Tesseract with jpn/kor/chi models). The
     reader already knows which run of glyphs is unknown, so it would crop that span and
     ask.
     - **For:** handles any script and font, and needs no piece-grouping.
     - **Against:** an external dependency, slower (but names are rare), and less certain
       on tiny, anti-aliased text.
3. **Is a readable name the goal, or recovering the species?** For your stats, opponent
   names matter little and species matter a lot. If species is the real goal, team preview
   (roadmap stage E) may be the better next step, and this spec smaller.
