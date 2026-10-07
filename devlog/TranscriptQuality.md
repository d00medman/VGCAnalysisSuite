# Transcript quality — why lines come out unclear, and what fixed them

Where the reader goes wrong on real recordings, measured across Batch 0 (103 recordings,
7,513 lines), and what each fix bought. Speed work from the same days is in
[AnalyzerPerformance.md](AnalyzerPerformance.md). **[unverified]** = not checked in-session.

## 2026-10-06/07 — unclear lines halved; the rest is mostly names

### 1. Tools

- **`scripts/unclear-report.py [batch…] [--lines out.tsv]`** reads transcripts only (never
  the video). It splits `[unclear]` lines into two kinds:
  - **Partial:** readable text with `?`s. The report guesses each `?` from pokedex names and
    words read cleanly elsewhere, allowing only characters the atlas lacks.
  - **Unreadable:** fewer than 4 letters. Grouped into bursts per video, with timestamps,
    so they can be found in the footage.

  It also says whether the `?`s fall in a player's name, and ranks videos.
- **`scripts/transcribe-all.py --regenerate`** redoes transcripts older than the
  analyzer, atlas or lexicon, and keeps each previous version in
  `sandbox/transcripts-previous/` for diffing. Batch names limit a run to those batches.
- **Diagnosing a burst:**
  1. `analyzer harvest --unknown-only --ss … --t …` saves the message-strip crops.
  2. A frame grab with the vendored ffmpeg shows the whole screen.
  3. `analyzer read <crop>` prints the reader's segmentation glyph by glyph.

### 2. What was wrong (Batch 0, before)

**389 unclear lines (5.2%), in 71 of 103 videos.**

| Cause | Lines | Example |
|---|---|---|
| Scenery read as text between messages | ~130 of the 139 unreadable | Farigiraf's legs at the left of the strip in starchu, scrafty-htr |
| Glyphs missing from the atlas | ~40 words | `?ab` Jab, `?ap` Zap, `?-turn`, `Pok?mon`, digits |
| Player names and nicknames | 118 lines with `?` only in the name, plus most `The opposing ??? used …` | `Pou?ko`, all-`?` non-Latin names |

**Scenery, in detail.** In the command phase there is no message box, and your
Farigiraf's pale legs reach into the strip's left margin.
- **Shape:** 1–4px wide slivers, 45–60px tall (the tallest real glyph is `g`, 37px), 3–4 per
  row. Kingambit's swirl does the same at starchu 05:12.
- **Coincidences that pointed the wrong way:** the bursts correlated with harsh sunlight
  and Trick Room. But Farigiraf is the one setting Trick Room, and sun happened to be up
  in that game.

**Missing glyphs, in detail.**
- **The original atlas had 52 characters:** no `J Q U X Z j é`, and only the digits `1 6 9`.
- **"Mega Charizard?" was a bad guess:** the guesser said `X`, but the frame shows
  `Mega Charizard!`. The `!` after `d` is a shape the atlas hadn't seen.

### 3. Fixes

1. **A text row needs at least 6 glyphs** (`MIN_ROW_GLYPHS` in `text.rs`, commit `25f1277`).
   - **Why it's safe:** the shortest real message in Batch 0 is `Go! Nala!` (8 glyphs), and
     110 of the 139 unreadable lines had 3–4.
   - **It stays geometric,** per the module's rule that gates never depend on recognition,
     so non-Latin names still pass.
   - **The leg and swirl crops are now test fixtures** in `tests/fixtures/scenery/`.
2. **Atlas: 21 labelled crops** (commit `98e610a`). They add `U Z J é 0 2 3 5` and a second
   `!`.
   - **Labels came from images:** each crop was viewed and labelled from the image. Crops
     caught mid-fade (letters missing) were left out.
   - **Result:** 64 of 66 labelled crops now read exactly, up from 43. The other two were
     already skipped by `analyzer atlas` (glyph-count mismatch).
   - **The lexicon** was rebuilt from the committed M-A…M-C snapshots (+20 words).

### 4. Result (Batch 0, after `--regenerate`)

| | Before | After |
|---|---|---|
| Unclear lines | 389 (5.2%) | **186 (2.5%)** |
| Unreadable lines | 139 | **28** |
| Videos with any unclear line | 71 | **34** |

**Bonus:** many messages that used to be cut short are now whole (`Aqua et!` → `Aqua Jet!`).
Player names now read: `Pouéko`, `VGCJames`, `Juice`, `nickyhigh5ez`, `Higuy93`.

**Losses: none real.**
- Every clean line of 6+ glyphs that changed or vanished has a near-identical or fuller line
  within 4 seconds.
- Three lines got slightly worse (`Big Man` → `Big an`; two in FTDP2963), all from the
  dropout problem below.

### 5. Still open

- **Non-Latin names and nicknames** are the largest remaining group. The atlas is Latin
  only, and `text.rs` has a `TODO(cjk)`: multi-piece characters split into several blobs.
  Next piece of work.
- **Remaining leg noise:** scrafty-htr still has 14 lines of 6–9 slivers. A shape gate
  (very tall, very thin blobs) would catch them; the frames need looking at first.
- **Letters dropping out mid-message:** `A s d rm kicked up!`, `Th p sing`. Seen around
  sandstorms and overlapping sprites. Not investigated.
- **Digits `4 7 8`:** no clean example in Batch 0. Batch 1 may have some.
- **Staleness is coarse:** `--regenerate` judges by file dates. Any analyzer rebuild marks
  every transcript stale, even when output is identical (e.g. after the crop-first change).
