# Analyzer — Performance & Deployment Scaling

Parked for later. How fast `analyzer transcript` runs, what bounds it, and how that shapes a
public cloud deployment. `DECISION:` is greppable; **[unverified]** = not checked in-session;
reversals get a new entry.

---

## 2026-09-23 — pipeline parallelised; now decode-bound; scaling options parked

### 1. Where it stands

Reference video: `sunroom_7_15_2026.MP4` — 9:39, 18,250 frames, 77 messages.

| | wall | notes |
|---|---|---|
| Before | 4:32 | single thread: decode → segment → read, serial per frame |
| After | **2:24** | same 77 messages / timestamps (by eye against the earlier run) |

Final `stages:` line of the new run:
`decode wait 139.0s · segment 38.7s · read 16.7s (segment/read summed over 4 workers)`

**The analyzer is now bound by ffmpeg's HEVC decode** (139s of 144s wall). All analysis
together is ~55 CPU-seconds — under half a core on average. Further work on segmentation or
glyph matching will not move wall time.

- [unverified] Frame count went 18,249 → 18,250. Nothing in the change should alter how many
  frames ffmpeg emits. A byte-level diff of `--jsonl` output (old binary vs new) was never
  run; do that before relying on "output unchanged".

### 2. What changed (all intended to be output-preserving)

- **Pipeline** (`transcribe.rs`): reader thread → worker pool → in-order reassembly into the
  `Tracker`. Workers = half the cores, max 6; `ANALYZER_JOBS=N` overrides.
- **Glyph cache** (`atlas::GlyphCache`): per-worker memo of `classify` keyed on
  `(top, w, h, bitmap)`; cleared at 100k entries.
- **Bit-packed scoring** (`atlas::score_packed`): rows as `u128`, IoU via popcount; per-pixel
  path kept as fallback for >127px-wide bitmaps. Test asserts bit-identical scores on 2,000
  random pairs.
- **Margin early exit** (`text::text_rows`): no ink in columns 30–42 → no rows possible → skip
  segmentation.
- **Oversize blobs** (`text::blobs`): stop collecting pixels once a blob outgrows a glyph
  (still flooded, so it can't seed sub-blobs).
- **ffmpeg**: `showinfo=checksum=0`.
- **Release profile** (analyzer + server): `opt-level = 3`, thin LTO, `codegen-units = 1`.
- **CLI**: transcript lines no longer print onto the tail of the progress line; `stages:`
  timing line at the end.

Deliberately skipped: per-frame buffer reuse (negligible), larger pipe buffer (reader thread
now drains continuously), `target-cpu=native` (breaks portability of deployed binaries).

### 3. Input facts that constrain everything

From `ffprobe` headers, no decode:

- HEVC Main 8-bit, `yuvj420p`, **B-frames present** (`has_b_frames=2`)
- Stored **portrait 1126×2436 with a 90° rotation flag**
- VFR: 60 fps nominal, ~31 fps average
- Keyframe every ~2s (32 in the first 60s)
- **1.1 GB for 9:39** (15 Mbps)

### 4. Options, cheapest first

**a. Crop before rotate** — cheap, likely output-identical. ffmpeg autorotate transposes the
full 2436×1126 frame *before* our crop, moving ~4MB/frame to keep 9% of it. With
`-noautorotate`, crop the equivalent strip in portrait space and transpose only that. Crop
offsets are even, so 4:2:0 chroma stays aligned. [unverified] estimate: 5–15% of decode-side
time. Needs a transcript diff to confirm identical output.

**b. `-skip_frame nonref`** — may skip decoding non-reference B-frames. Gain depends on the
iOS GOP structure [unverified]. **Changes output** (fewer frames, coarser timing); messages
are on screen for seconds so probably tolerable, but needs a transcript comparison.

**c. Keyframe-chunked parallel decode** — split at keyframes (~2s apart), decode chunks on
separate processes/machines, stitch tracker output with a few seconds of overlap + dedupe.
**Cuts latency, not cost** (same total CPU-seconds). Right shape for queue workers or
serverless fan-out.

**d. Server GPU decode (NVDEC)** — T4/L4-class cloud GPUs decode HEVC far faster than CPU.
Worth it only once volume is steady and per-video CPU cost is measured. Locally, the pinned
ffmpeg 7.0.2 static build exposes only `vdpau` (usable via the GTX 1050 Mobile, not the Intel
UHD 630); VAAPI/NVDEC would need a different build, which conflicts with the pinned-decoder
determinism rationale in `vendor/fetch-ffmpeg.sh`. Local test, never run:

```
time vendor/ffmpeg -hide_banner -ss 120 -t 60 -i <video> -map 0:v:0 -an -f null -
time vendor/ffmpeg -hide_banner -hwaccel vdpau -ss 120 -t 60 -i <video> -map 0:v:0 -an -f null -
```

**e. Client-side decode in the browser** — the big lever for public use.
- **Upload dominates UX:** 1.1 GB on a ~20 Mbps uplink is ~7 min, longer than processing.
  No server-side speedup fixes that.
- `text`, `atlas`, `episode` are pure Rust (serde only) → compile to WASM. ffmpeg lives only in
  `decode.rs`.
- Flow: demux MP4 in JS (e.g. mp4box.js) → WebCodecs `VideoDecoder` (user's hardware) →
  copy only the ROI strip → WASM analyzer → upload the transcript (KB).
- Per-video compute cost to us ≈ 0; server just stores transcripts.
- Catches: HEVC in WebCodecs is uneven across browsers [unverified at time of writing: Safari
  yes, Chrome hardware-dependent, Firefox largely no] → server path stays as fallback.
  Browser decode + YUV→RGB won't be bit-exact with pinned ffmpeg; thresholds are coarse, but
  needs a corpus comparing browser vs server transcripts.

### 5. Leaning (not decided)

1. Make the core frame-source-agnostic: a `FrameSource` trait, ffmpeg impl now, WebCodecs
   impl later. Cheap, keeps both paths open.
2. Measure **CPU-seconds per video-minute** (`/usr/bin/time -v`, user+sys over a full run)
   before choosing server hardware; price CPU vs GPU vs serverless from that number.
3. Server path: queue, resumable direct-to-object-storage uploads, keyframe-chunked CPU
   workers; GPU workers only if the numbers justify them.
4. Browser path as the primary route for public users, server as fallback — after a spike
   confirming WebCodecs HEVC reach and browser/server transcript agreement.

### 6. When picking this back up

- [ ] Diff `--jsonl` old vs new on the full reference video (explains 18,249 vs 18,250?)
- [ ] Measure CPU-seconds per video-minute
- [ ] Try 4a (`-noautorotate` + portrait crop), diff transcripts
- [ ] Optional: vdpau timing test (4d)
- [ ] Spike: WASM build of `text`/`atlas`/`episode`; WebCodecs HEVC decode of a real recording
