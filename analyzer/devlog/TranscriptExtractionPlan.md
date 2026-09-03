# Champions Battle Transcript — Extraction Plan

Date: 2026-09-03 (rev 1)
Status: **planned, not implemented.** `analyzer/` contains only this devlog as of writing.
Location: `/home/barbaroja/Videos/pokemon_recordings/analyzer/`
Reference recording: `raw_recordings/sunroom_7_15_2026.MP4`

## Context

Produce a deterministic, tunable transcript of a Pokémon Champions battle from a screen
recording. The transcript must capture game actions and state changes as they occur, and must
be robust to iOS artifacts in the capture (notification banners, leaving the app).

Sibling projects set house style: `../importer/` (Rust, edition 2021, clap-derive + anyhow) and
`../pokedex/` (SQLite reference data). The pokedex is the analyzer's **lexicon**: species, move,
ability and item names are the vocabulary that OCR output gets snapped to.

## Source format — measured, not assumed

`ffprobe` on the reference recording:

| Property | Value |
| --- | --- |
| Container / codec | mov,mp4 / HEVC Main, yuvj420p, bt709 |
| Stored resolution | 1126x2436 (portrait) |
| Display Matrix | rotation 90 → **2436x1126 landscape** when decoded |
| Frame rate | `r_frame_rate` 60/1, `avg_frame_rate` 35550/1129 ≈ **31.49 fps** |
| Frame count / duration | 18249 frames / 579.543 s |
| Bitrate / size | 15.1 Mbps / 1.1 GB |
| Keyframe interval | ~1.85–2.00 s |
| Audio | AAC LC, 44.1 kHz, stereo (unused) |

Three consequences, each load-bearing:

1. **This is an iOS screen recording, not camera footage of a screen.** 1125x2436 is the
   iPhone X/XS/11 Pro panel; the encoder rounded to 1126. The UI is therefore pixel-exact and
   fixed-position. **No rectification, homography, glare or moiré handling is needed** — an
   entire class of work that camera capture would have forced on us does not exist here.
2. **The stream is VFR.** iOS drops frames while the screen is static, so `avg` (31.5) is far
   below `r_frame_rate` (60). Frame index is *not* proportional to time. **All timing must come
   from presentation timestamps, never from a frame counter.**
3. **Decode is cheap.** 30 s of video decoded in 7.52 s wall at 620% CPU (8 cores) ≈ 4x
   realtime; a full pass over the reference recording is ~145 s. Decoding every frame is
   affordable, so we never need to subsample and risk missing a short-lived message.

Environment: Ubuntu 22.04.5, 8 cores, 15 GB RAM, GTX 1050 (2 GB). cargo/rustc 1.94.0.
**No system ffmpeg and no tesseract installed**; `sudo` requires a password. Work so far used a
pinned static ffmpeg 7.0.2 (johnvansickle build) fetched into scratch.

- `DECISION:` **vendor the pinned static ffmpeg binary rather than `apt install ffmpeg`.** A
  fixed decoder version is part of the determinism guarantee, and it removes a root-privilege
  install from the setup path. Distro ffmpeg would be 4.4.2 here and different elsewhere.

## The reference battle

Ranked Battles / **Double Battle**, bring-6-pick-4. Recorder is **Molly** (Master Ball Tier
Rank 2, rating 1,810.096, 5-win streak); opponent is **Porks619** (Rank 2, rating 1,770.383).

Molly's six, from team preview: Yertle (Heat Rock), Venusaur (Life Orb), Farigiraf (Sitrus
Berry), Clodius (Scraftinite), Mamoswine (Focus Sash), Pyroar (Pyroarite). Picks 1/2/3 were
Pyroar / Yertle / Farigiraf.

Two facts from this that generalise and shape the design:

- **Pokémon carry nicknames.** `Yertle` is a Torkoal, `Clodius` a Scrafty, `STAR PLTNM` an
  opponent Incineroar. Names appearing in messages are therefore **not** drawn from the species
  lexicon. Species must come from the nameplate icon or from team preview, not from the name.
- **Mega evolution is live** (`Scraftinite`, `Pyroarite`), so a Pokémon's species and stats can
  change mid-battle.

## What each UI state offers

| State | Yields |
| --- | --- |
| Team preview | Own 6 with nicknames + held items + gender; opponent 6 as species icons + revealed types; pick order |
| Command select | Persistent HUD: 2 ally nameplates, opponent active nameplate, weather + duration, MOVE TIME, both clocks, remaining-Pokémon dots |
| Move menu | Move names, PP, and a type-effectiveness hint per move |
| Battle Info overlay | Full-state snapshot: own 6 absolute HP, opponent 6 percent HP, revealed types |
| Summary popup | Active Pokémon's types, 4 moves with PP, ability, held item |
| Turn resolution | Message line, transient spotlight nameplates, item-activation popups |

**`Battle Info` is a button, not a screen.** The center panel is a separate **Summary popup**
toggled by Show/Hide Summary, reachable from team preview, from the Battle Info overlay, and
from Pokémon selection. It is one recurring popup appearing in several contexts. This
distinction was got wrong once already; see the collision below for why it matters.

## ROI map

All coordinates in **original decoded pixels (2436x1126 landscape)**.

> **These are approximate.** Every figure below was measured off a handful of frames of one
> recording. They are a starting point for calibration, **not** config values. Pinning them
> against a fixture set is Phase 2.

| ROI | x | y | Notes |
| --- | --- | --- | --- |
| `anchor.conn` | 200–268 | 12–55 | Connection indicator. White outline invariant; fill encodes quality |
| `message` | 530 → ~1700 | 762–832 | **Left-aligned at x≈530**, not centered |
| `ally.1` | 219–621 | 950–1060 | Absolute HP (`127/157`) |
| `ally.2` | 621–1023 | 950–1060 | Absolute HP |
| `opp.1` | 1462–1790 | 37–177 | **Percent** HP (`26%`) |
| `opp.2` | 1888–2229 | 37–177 | Percent HP |
| `popup.summary.ability` | 926–1279 | 767–828 | **Overlaps `message`** |
| `banner` | 384–2174 | 0–330 | iOS notification; **overlaps `opp.1`/`opp.2`** |

### The two collisions

1. **Summary popup vs `message`.** The popup's Ability row renders white text at y767–828,
   inside the message band. Verified: at t=180 the message ROI reads `Ability   Chlorophyll`.
   Without gating this emits a bogus message event every time the popup is opened.
2. **iOS banner vs opponent nameplates.** The banner occupies y0–330; the opponent nameplates
   sit at y37–177, fully inside it. The banner does **not** reach the message line (y762+) and
   does **not** reach `anchor.conn` (x200–268, left of the banner's x384).

The second collision is why banner handling is not optional: opponent HP% is explicitly in
scope, and it is the one ROI directly in the blast radius.

### Asymmetries

- **Own side shows absolute HP** (`50/161`); **opponent shows percent** (`28%`). Two different
  parsers, two different event types. Damage on the opponent side is only ever recoverable as a
  percentage delta.
- **Item-activation popups appear on different sides** depending on whose item fired
  (`Farigiraf's Sitrus Berry` rendered left of centre; `STAR PLTNM's Sitrus Berry` rendered
  right of centre). This is **not** a single ROI and must be mapped before it can be read.

## Gating

One mechanism covers all three noise classes, deterministically, at a few hundred pixel reads
per frame. No ML, no classifier training.

- **In-app test.** Match the `anchor.conn` white mask against a stored template. Verified
  present at identical coordinates in every frame sampled across four recordings, in every UI
  state, across two different arena backgrounds. Absent → skip the frame entirely.
- **Mode gate.** A handful of fixed probe pixels tested against expected UI colours decides
  which ROIs are readable this frame. This is the "index heavily on fixed structure" idea made
  concrete.
- **Occlusion.** In-app, in a known mode, but that mode's expected anchors are missing →
  something is covering them.

- `DECISION:` **occlusion suppresses the affected ROIs, not the frame.** A 3-second banner then
  produces a gap between two opponent-HP plateaus, both still read correctly, while the message
  line keeps working throughout. Dropping whole frames would discard good data from ROIs the
  occluder never touched.
- `DECISION:` **UI mode is an internal gate and never appears in the output.** The transcript
  is a flat event stream.

The connection indicator's fill colour (green = good, amber = degraded, observed amber in
`202608_a_GJUN5795`) is logged as a side channel — a degraded connection is a plausible
explanation for anomalous timing, so it is worth keeping.

## Noise taxonomy

| Class | Evidence | Handling |
| --- | --- | --- |
| iOS notification banner | Verified, sunroom t≈46.5–49.0 (Gmail / "The Free Press") | Detect, suppress covered ROIs |
| Leaving the app | **No example found yet** | `anchor.conn` absent → skip frame |
| Summary popup over message ROI | Verified, sunroom t=180 | Mode gate; TODO to read it properly |
| `Communicating...` network wait | Verified, sunroom t≈551.5–554.5 | Excluded from the message feed |
| Nameplate alpha fade | Verified, sunroom t≈554.5 | Plateau detection rejects unstable frames |
| Typewriter message prefixes | Inherent to the text renderer | Message-episode rule, below |
| HP drain animation | Verified: Pyroar 58 @536.0 → 50 @537.5 | HP settling window, below |

## Extraction algorithms

**Message episodes.** Text types out character by character, so intermediate frames are briefly
stable and would emit prefix fragments. A new reading that is a strict **prefix-extension** of
the current one continues the same episode; one that is not ends the episode and starts a new
one. Emit on episode end. This handles back-to-back messages with no intervening blank frame,
which a blank-delimited scheme would merge.

**HP settling.** Coalesce consecutive plateaus of the same nameplate until the value stops
changing for `hp_settle_ms` (~400 ms), then emit one observation carrying both the settled value
and the pre-change value. This is the primary noise knob, and it is what turns the observed
`161 → 58 → 50` into a single `delta: -111`.

**Plateau detection.** Per ROI per frame, compute a cheap hash. A run of identical hashes
exceeding a minimum duration is a plateau. Emit at run end with `t0`/`t1` from presentation
timestamps.

## Output

Two artifacts, and the split is the most important decision in this document.

### Layer 1 — `observations.jsonl`

Expensive, produced once per video. Literal transcription of what was on screen, with no game
semantics. One record per stable plateau.

```json
{"t0":530.02,"t1":532.55,"roi":"message","v":"The opposing Sylveon used Hyper Beam!","conf":0.99}
{"t0":537.44,"t1":543.31,"roi":"ally.1","v":{"name":"Pyroar","hp":50,"max":161}}
{"t0":561.80,"t1":566.40,"roi":"opp.2","v":{"name":"STAR PLTNM","pct":28,"status":"slp"}}
{"t0":46.50,"t1":49.02,"roi":"_suppressed","cause":"banner","rois":["opp.1","opp.2"]}
```

### Layer 2 — `transcript.jsonl`

Cheap, re-derived constantly. Flat events, each on its own line. **No turn segmentation in v1.**

```json
{"t":530.02,"ev":"move","side":"opp","actor":"Sylveon","move":"Hyper Beam"}
{"t":537.44,"ev":"hp","side":"ally","name":"Pyroar","hp":50,"max":161,"prev":161,"delta":-111}
{"t":540.50,"ev":"effectiveness","value":"not_very","target":"Pyroar"}
{"t":556.00,"ev":"move","side":"ally","actor":"Venusaur","move":"Sludge Bomb"}
{"t":561.80,"ev":"hp_pct","side":"opp","name":"STAR PLTNM","pct":28,"prev":54,"delta":-26}
{"t":592.10,"ev":"unparsed","raw":"...","conf":0.62}
```

- `DECISION:` **keep both layers.** Tuning happens entirely at Layer 2, so a threshold change
  costs a re-derive (seconds) rather than a re-decode (~2.5 min per video, and far worse across
  the ~100-recording corpus). Layer 1 is also the audit trail when a derived event is wrong.
- `DECISION:` **nothing is ever silently dropped.** Text matching no template is emitted as
  `unparsed` with its confidence. Every record retains `raw` and `conf`, so filtering is
  subtractive and reviewable.
- `DECISION:` **weather, crits, status and every other message get their own event line.** No
  grouping, no nesting, in v1.
- `DECISION:` **team preview is stubbed off and never fires in v1**, because recordings will not
  reliably contain it.

## Implementation

- `DECISION:` **no ML.** The font is fixed, the rendering is pixel-exact, and the glyph set is
  small. A **glyph template atlas** with nearest-neighbour matching is bit-exact reproducible,
  needs no weights, no GPU and no Python. Digits in HP / PP / timers are a 10-class problem at
  known size.
- `DECISION:` **pure Rust**, with `ffmpeg` as a subprocess streaming `rawvideo` over a pipe. No
  bindings, no OpenCV, no ONNX runtime. Dropping ML is what makes this possible.
- Atlas bootstrapping: label a set of message crops by hand once; derive glyph templates by
  alignment. This removes the only remaining argument for an OCR dependency.
- **Lexicon snap.** The output space is nearly enumerable — known message templates crossed with
  known names — so readings snap to the nearest legal string with deterministic tie-breaking.
  `../pokedex/` supplies move, ability and item names. Nicknames must be learned per battle from
  nameplates and team preview.

Text isolation was prototyped: a near-white + low-saturation mask gated on a nearby dark outline
pixel cleanly recovers `The opposing Sylveon used Hyper Beam!` from a busy 3D background, and
produces an empty mask on frames with no message (no false positives). Two glyphs dropped out
under a crude fixed threshold; an adaptive threshold using the outline as the primary cue rather
than as a gate is the obvious fix.

## Phases

| # | Phase | Deliverable |
| --- | --- | --- |
| 1 | Decode harness | Rust + pinned ffmpeg subprocess; PTS-correct frame iteration over a VFR stream; `anchor.conn` in-app test |
| 2 | **Flagging tool** | Harvests candidates across the corpus: missing anchors, unknown popup signatures, unmatched message strings, banner candidates |
| 3 | ROI map + mode gate | Coordinates pinned against the Phase 2 fixture set; probe-pixel mode gate |
| 4 | Plateau + digits | Plateau detector, HP/PP/timer digit reader, `observations.jsonl`. **No prose text yet** |
| 5 | Glyph atlas | Message reader |
| 6 | Layer 2 | Event derivation, lexicon snap, `transcript.jsonl` |
| 7 | Banner detector | Built against the harvested fixtures |

Two ordering notes. **The flagging tool moved from last to second**: ROI coordinates, the
message grammar, the popup taxonomy and banner variants are all unknown, and all four resolve
from the same harvested corpus. Building it early unblocks the rest. **Digits precede prose**
in Phase 4 because a 10-class problem at known size validates the whole plateau pipeline before
variable-width text is introduced.

## Verification targets

- Determinism: run the full pipeline twice over the same file, byte-identical `observations.jsonl`
- VFR timing: emitted timestamps match `ffprobe` PTS for the same events, no frame-counter drift
- Message episodes: typewriter frames produce one event, not a chain of prefixes
- HP settling: the Pyroar `161 → 58 → 50` sequence yields exactly one event with `delta: -111`
- Banner: sunroom t≈46.5–49.0 suppresses `opp.*` only, and `message` events in that window still emit
- Popup: sunroom t=180 emits **no** message event
- `Communicating...` never reaches the transcript
- Idempotency: re-deriving Layer 2 from an unchanged Layer 1 is a no-op

## Open decisions

1. **ROI coordinates are approximate** and measured off one recording. Blocking Phase 3.
2. **The message template grammar is unenumerated.** Eight strings observed. Layer 2's parser is
   only as good as this list and its size is unknown.
3. **Popup taxonomy is a TODO.** Summary popup, target select, `Communicating...`, and an unknown
   number of others. Treated as noise in v1; they should eventually be read and integrated.
4. **No app-exit example exists.** Five recordings examined, one banner and zero app-exits found.
   The `anchor.conn` test is the intended handling but is **untested against a real case**.
5. **Banner variants unknown.** One sample: one app, dark mode, one length. Light mode, stacked
   banners and longer text are unexamined.
6. **Item-popup side mapping** must be determined before item events can be read.

## Known gaps / deliberate non-goals

- **Turn segmentation is out of scope for v1**, by choice. `MOVE TIME` and the command-phase HUD
  make it recoverable later.
- **Opponent damage is only ever a percentage.** Absolute damage on the opposing side is not
  displayed and cannot be recovered from video alone.
- **Opponent nicknames are learnable only from nameplates**, which appear transiently, so the
  opponent lexicon is built incrementally and may be incomplete early in a battle.
- **Audio is unused.** It may carry usable cues (cries, hit sounds) but is not in scope.
- Team preview reader is written but disabled.
- Replay/validation against a Champions Showdown mod is not in scope, though the flat event
  schema is deliberately close to the Showdown protocol to keep that option open.

---

Tags: analyzer, video, ocr, planning
