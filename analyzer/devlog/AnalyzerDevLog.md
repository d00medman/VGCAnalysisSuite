# Analyzer DevLog

Append-only journal for the Champions battle-transcript analyzer. Newest entry at the top.
The design document lives beside this file in `TranscriptExtractionPlan.md`; code will live in
`analyzer/`. This file is for *what we learned and why we decided things* — not for schemas or
code, which belong in the plan and the repo.

Sibling journal: `../../devlog/DataSourcingResearchResults.md` covers the Pokédex project.
Kept separate because that file declares itself scoped to the Pokédex, but the two are coupled:
the pokedex is the analyzer's lexicon.

**Conventions** (inherited from the Pokédex devlog)
- One entry per working session, dated `YYYY-MM-DD`, with a one-line title.
- Every factual claim gets a source or a "verified by" note. Unverified claims are
  labelled **[unverified]** so they don't calcify into assumptions.
- Decisions get their own bullet prefixed `DECISION:` so they're greppable.
- Reversals are new entries, never edits to old ones.

---

## 2026-09-03 — Source format characterised; architecture chosen; no code written

Scope: figure out what the recordings actually are and what the transcript should be. No code
committed to `analyzer/`. Reference file: `raw_recordings/sunroom_7_15_2026.MP4`.

### 1. The recordings are screen captures, not camera footage — this was the session's biggest surprise

The working assumption going in was camera-filmed footage of a screen, which would have required
screen detection, homography rectification, and glare/moiré handling. `ffprobe` says otherwise:

- Stored 1126x2436 portrait with a Display Matrix `rotation: 90` → decodes to **2436x1126
  landscape**. 1125x2436 is the iPhone X/XS/11 Pro panel resolution; the encoder rounded to 1126.
- HEVC Main, yuvj420p, bt709, 15.1 Mbps, 579.543 s, 18249 frames.

`../importer/README.md` corroborates: files are pulled off an **iPhone over PTP**. So these are
iOS screen recordings of the game, and the UI is pixel-exact and fixed-position.

**Verified by** `ffprobe -show_streams` on the reference file, plus visual inspection of six
frames sampled across the recording.

- `DECISION:` **drop all rectification work.** An entire planned subsystem — screen quad
  detection, homography, perspective correction — does not need to exist.

### 2. The stream is VFR, and this is a correctness trap

`r_frame_rate` is 60/1 but `avg_frame_rate` is 35550/1129 ≈ **31.49 fps**. iOS drops frames
while the screen is static. Frame index is therefore not proportional to time.

- `DECISION:` **all timing comes from presentation timestamps, never a frame counter.** Any
  code that multiplies a frame index by a nominal frame rate is wrong on this corpus.

Keyframes land every ~1.85–2.00 s, so fast seek-based sampling is viable for exploration tooling
even though the main pass decodes everything.

### 3. Decoding every frame is affordable

30 s of video decoded to null in **7.52 s wall at 620% CPU** on 8 cores ≈ 4x realtime; a full
pass over the 9.7-minute reference is ~145 s. No subsampling is needed, so short-lived messages
cannot be missed by a sampling policy.

No system `ffmpeg` and no `tesseract` are installed, and `sudo` needs a password. Work used a
pinned static ffmpeg 7.0.2 (johnvansickle) fetched into scratch.

- `DECISION:` **vendor the pinned static ffmpeg rather than `apt install`.** A fixed decoder
  version is part of the determinism guarantee, and the distro package here is 4.4.2 and would
  differ on another machine.

### 4. "Noise" turned out to mean iOS artifacts, not video artifacts

Initially read as compression/optical noise. It is not — it is **iOS notification banners and
leaving the app mid-recording**.

Found and measured one real banner: sunroom **t≈46.5–49.0**, a Gmail / "The Free Press" alert,
bounding box **x384–2174, y0–330**, appearing during team preview.

The geometry matters because it decides the handling:

- The banner **does** cover the opponent nameplates (y37–177), which carry opponent HP%.
- It **does not** reach the message line (y762–832).
- It **does not** reach the connection indicator (x200–268, left of the banner's x384).

- `DECISION:` **occlusion suppresses the covered ROIs, not the whole frame.** A banner then
  leaves a gap between two opponent-HP plateaus while the message line keeps working. Dropping
  frames wholesale would discard good data from ROIs the banner never touched.

**[unverified]** No app-exit example has been found. Five recordings were examined via contact
sheets; they yielded one banner and zero app-exits. The intended handling (connection-anchor
absence) is therefore **untested against a real case**.

**[unverified]** Banner variants are uncharacterised: one sample, dark mode, one app, one text
length. Light mode, stacked banners and longer bodies are unknown.

### 5. The connection indicator is a free in-app anchor

A small glyph at **x200–268, y12–55** is present in *every* frame sampled, across four different
recordings, in every UI state (team preview, command select, Battle Info overlay, turn
resolution), and across two visually unrelated arena backgrounds. Its white outline is
invariant; only the inner fill changes — green normally, **amber in `202608_a_GJUN5795`**,
so the fill encodes connection quality.

This gives a deterministic "am I in the app" test for essentially nothing, and it survives
banners because it sits outside the banner's x-range.

- `DECISION:` **match the white outline, not the fill.** The fill is a signal in its own right
  and gets logged as a side channel — a degraded connection is a plausible explanation for
  anomalous timing.

### 6. Two ROI collisions, one of which I got wrong first

**Summary popup vs the message line.** At t=180 the message ROI reads `Ability   Chlorophyll`.
I initially attributed this to a "Battle Info screen". **Corrected by the user:** `Battle Info`
is a *button*; the centre panel is a separate **Summary popup**, toggled by Show/Hide Summary,
which also appears during team preview and during Pokémon selection. It is one recurring popup
in several contexts, not a property of one screen.

- `DECISION:` **UI mode is an internal gate only and never appears in the output.** The gate is
  still required — without it every Summary popup emits a bogus message event.
- `DECISION:` popups are **treated as noise in v1**, with a TODO to read and integrate them.
  They carry real state (moves, PP, ability, held item) and are worth having later.

**Banner vs opponent nameplates**, covered in §4.

### 7. HP animates, so plateau detection is mandatory

Pyroar's HP was observed at **58 @ t=536.0** and **50 @ t=537.5** — the counter ticks down
during the drain animation. Only the final plateau is a real value. Nameplates additionally fade
in and out with alpha (observed t≈554.5), so partial-alpha frames must not be read either.

Message text types out character by character, so intermediate frames are briefly stable and
would emit prefix fragments.

- `DECISION:` **message-episode rule** — a reading that is a strict prefix-extension of the
  current one continues the episode; one that is not ends it. This handles back-to-back messages
  with no intervening blank frame, which a blank-delimited scheme would merge.
- `DECISION:` **HP settling window** (~400 ms) coalescing consecutive plateaus, emitting the
  settled value plus the pre-change value. This turns `161 → 58 → 50` into one `delta: -111`,
  and is the primary tuning knob.

### 8. Text isolation was prototyped and works

A near-white + low-saturation mask, gated on a nearby dark outline pixel, cleanly recovers
`The opposing Sylveon used Hyper Beam!` from a busy animated 3D background, and produces an
**empty** mask on frames with no message — no false positives from stage lighting. Two glyphs
dropped out under a crude fixed threshold, where the text overlapped a bright background region.

An earlier attempt to find ROIs by accumulating a near-white heatmap over 580 frames was
confounded by stage lighting; the averaged-frame ghost image was far more informative and is
what produced the ROI map. A high-frequency-energy test for the banner was also tried and
**rejected** — the arena is already smooth in the top band, so the signal is scene-dependent.

- `DECISION:` **no ML.** Fixed font, pixel-exact rendering, small glyph set → a glyph template
  atlas with nearest-neighbour matching is bit-exact reproducible, needs no weights and no GPU.
  Digits in HP/PP/timers are a 10-class problem at known size.
- `DECISION:` **pure Rust**, ffmpeg as a subprocess over a rawvideo pipe. No bindings, no
  OpenCV, no ONNX. Dropping ML is what makes this possible. The user had expected ML would be
  needed and agreed to drop it.
- Atlas bootstrapping: hand-label a set of message crops once, derive templates by alignment.

### 9. Game facts that shape the schema

- **Own side shows absolute HP** (`50/161`); **opponent shows percent** (`26%`). Two parsers,
  two event types. Absolute damage on the opposing side is **not recoverable from video**.
- **Nicknames are in use** — `Yertle` is a Torkoal, `Clodius` a Scrafty, `STAR PLTNM` an
  opponent Incineroar. Names in messages are therefore not species names, and species must come
  from nameplate icons or team preview.
- **Megas are live** (`Scraftinite`, `Pyroarite` held), so species and stats can change mid-battle.
- Fainted nameplates render dark grey at `0%` — a structural faint signal independent of text.
- Item-activation popups render **on different sides** depending on whose item fired. Not one ROI.
- Command-phase HUD carries weather with a duration counter (`Harsh Sunlight 1/8`), `MOVE TIME`,
  and two clocks (`05:01` top, `04:26` bottom).
  - **[unverified]** `1/8` read as weather turns remaining; consistent with Yertle's Heat Rock
    granting 8 turns of sun, but not confirmed.
  - **[unverified]** the two clocks read as per-player time banks.
- `Communicating...` appears as a network-wait state (t≈551.5–554.5) and must be kept out of the
  message feed.
- Arena backgrounds differ substantially between recordings (`202608_a_KCUI3658` is a gold/desert
  stage vs sunroom's blue/green), confirming text extraction must be background-independent.

### 10. Output shape

- `DECISION:` **two layers.** `observations.jsonl` (expensive, literal, once per video) and
  `transcript.jsonl` (cheap, semantic, re-derived constantly). Tuning happens entirely at
  Layer 2, so a threshold change costs seconds rather than a ~145 s re-decode per file across a
  ~100-recording corpus. Layer 1 doubles as the audit trail when a derived event is wrong.
- `DECISION:` **flat events, one per line, no turn segmentation in v1.** Weather, crits and
  status afflictions each get their own line.
- `DECISION:` **nothing is silently dropped** — unmatched text is emitted as `unparsed` with a
  confidence, so filtering stays subtractive and reviewable.
- `DECISION:` **team preview reader stubbed off**, since recordings will not reliably contain it.
- `DECISION:` **opponent HP% changes are in scope for v1**, which is what makes banner handling
  mandatory rather than optional.

### 11. Ordering change

The flagging tool was originally planned last. It moved to **Phase 2**: ROI coordinates, the
message grammar, the popup taxonomy and banner variants are all unknown, and all four resolve
from the same harvested corpus. Building it early unblocks everything downstream.

Digits are read before prose (Phase 4 before 5) so a 10-class problem at known size validates
the plateau pipeline before variable-width text is introduced.

### Next session

Phase 1: decode harness with PTS-correct iteration and the connection-anchor test.

---

Tags: analyzer, video, ocr, planning
