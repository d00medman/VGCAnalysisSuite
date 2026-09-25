# Pokémon Champions battle recordings

A WIP solution to extract battle data from Pokémon Champions.

Tools for turning iPhone screen recordings of Pokémon Champions battles into transcripts.

| Directory | What it is |
| --- | --- |
| `frontend/` | Web page: upload a recording, transcribe it, browse past transcripts (React) |
| `server/` | API behind the page: stores uploads, runs transcriptions, saves results to Postgres (Rust) |
| `analyzer/` | The transcription engine and its command-line tool (Rust) |
| `importer/` | Copies recordings off the iPhone over USB (Rust) |
| `pokedex/` | Game reference data (species, moves, items, regulations) and the database schema (Rust + Postgres) |
| `raw_recordings/` | The recordings themselves (gitignored) |

Every command below uses absolute paths, so it works from any directory. Set `REPO` once
per shell, or add the line to `~/.zshrc`:

```sh
export REPO=/home/barbaroja/Videos/pokemon_recordings
```

## Web app

Three containers, defined in `compose.yaml`:
- `web`: the page, served by nginx.
- `api`: the transcription server. On startup it applies any pending database migrations.
- `postgres`: uploads, battles, and transcripts, plus the pokedex (see [Pokedex](#pokedex)
  and [Battles and transcripts](#battles-and-transcripts)).

### Start

Docker must be running. If it isn't:

```sh
sudo systemctl start docker
```

Then:

```sh
docker compose -f "$REPO/compose.yaml" up --build -d
```

Open <http://localhost:8080>. The first build takes a few minutes because it compiles the
server and downloads the pinned ffmpeg. Later starts reuse the build.

To use a different port, set `WEB_PORT`:

```sh
WEB_PORT=9000 docker compose -f "$REPO/compose.yaml" up --build -d
```

### Use

1. **Upload a recording:** choose the video file and press **Upload**. A progress bar shows
   the upload.
2. Press **Transcribe**. A full battle takes about 3 minutes. The page shows progress, an
   estimate of time left, and messages as they are found. Only one transcription runs at a
   time; others wait as *queued*.
3. When it finishes, the video's name and transcript are saved. Every upload appears under
   **History**, newest first; click one to see its transcript. Lines the reader could not
   fully identify are greyed out and marked *unclear*. The page address includes the
   video's id (`#<id>`), so reloading or sharing the link reopens it.

Uploaded videos are kept, about 1 GB each, so any of them can be transcribed again with
**Transcribe again**, for example after the reader improves.

### The player

The player sits above the transcript.

- **While transcribing**, it follows the transcriber: it shows the frame being read,
  updated twice a second, with a yellow box around the message line the transcriber reads.
  If a playable copy already exists, **Watch video** switches to normal playback.
- **After transcribing**, it plays the whole recording. Click any timestamp in the
  transcript to jump the video to that moment. The line for what is on screen is
  highlighted as it plays.
- **Before the first transcription**, most desktop browsers show a note instead of the
  video. The iPhone records in HEVC, which Chrome and Firefox on Linux generally can't
  play. Each transcription also writes a browser-playable H.264 copy (1280 px wide, about
  130 MB per 10 minutes) from the same decode, so once a video has been transcribed it
  plays anywhere. Browsers that can play HEVC (Safari, some GPU setups) play the original
  immediately. Videos transcribed before the player existed get their copy the next time
  you press **Transcribe again**.

### Stop, logs, rebuild

```sh
docker compose -f "$REPO/compose.yaml" logs -f api     # follow the server's log
docker compose -f "$REPO/compose.yaml" down            # stop; saved data is kept
docker compose -f "$REPO/compose.yaml" up --build -d   # rebuild after code changes
```

### Where the data lives

Data is kept in two Docker volumes, so it survives `down` and rebuilds:
- `pokemon_recordings_videos`: uploaded recordings.
- `pokemon_recordings_postgres-data`: the database (history, transcripts, pokedex).

`docker compose -f "$REPO/compose.yaml" down -v` **deletes both**.

If a transcription is running when the server stops, that video is marked *failed*
("interrupted"). Press **Transcribe again** to redo it.

## Command-line transcription

The same engine the web app uses, without the containers.

```sh
# one-time: fetch the pinned ffmpeg (checksum-verified) and build
"$REPO/analyzer/vendor/fetch-ffmpeg.sh"
cargo build --release --manifest-path "$REPO/analyzer/Cargo.toml"

# transcribe a recording; progress goes to stderr, the transcript to stdout
"$REPO/analyzer/target/release/analyzer" transcript "$REPO/raw_recordings/sunroom_7_15_2026.MP4"

# save to a file (progress stays on the terminal)
"$REPO/analyzer/target/release/analyzer" transcript "$REPO/raw_recordings/sunroom_7_15_2026.MP4" > sunroom.txt

# a slice: start at 190s, 25s long
"$REPO/analyzer/target/release/analyzer" transcript "$REPO/raw_recordings/sunroom_7_15_2026.MP4" --ss 190 --t 25
```

Flags:
- `--jsonl`: JSON lines output, with start and end times and confidence.
- `-q`: no progress output.

The binary finds its ffmpeg and glyph atlas by absolute path, so it runs from anywhere.

## Pokedex

Game reference data (species, stats, moves, learnsets, held items, per regulation) lives in
the `postgres` service. The `pokedex` CLI loads it. It runs as a one-off container that
applies pending migrations before each command:

```sh
docker compose -f "$REPO/compose.yaml" up -d postgres
docker compose -f "$REPO/compose.yaml" run --rm pokedex regulation add \
    --name "Regulation M-A" --effective-from 2026-04-08
docker compose -f "$REPO/compose.yaml" run --rm -T pokedex import \
    < "$REPO/pokedex/ingest/out/snapshot.regulation-m-a.json"
docker compose -f "$REPO/compose.yaml" run --rm pokedex status
```

Postgres listens on `127.0.0.1:5432` (set `POSTGRES_PORT` to change it). The password
defaults to `pokedex`; set `POSTGRES_PASSWORD` to change it before the volume is first
created. To open a SQL shell:

```sh
psql postgres://pokedex:pokedex@127.0.0.1:5432/pokedex
```

The CLI reads the database from `DATABASE_URL`. The tests need the `postgres` service
running. Each test works in its own temporary schema:

```sh
cargo test --manifest-path "$REPO/pokedex/Cargo.toml"
```

Design and schema notes: [devlog/SchemaAndIngestPlan.md](devlog/SchemaAndIngestPlan.md).

## Battles and transcripts

Each upload is stored as a `video` holding one `battle`. Every transcription run adds a
`transcript` row with its status. **Transcribe again** adds a new run and keeps the old ones;
the page shows the newest finished run. Each message line is its own `transcript_line` row,
with `seq` for its order and `t0`/`t1` for its time in the video. Later tables can link
battle events to these rows by id.

```sql
-- the newest finished transcript of the most recent upload
SELECT l.seq, l.t0, l.text
FROM video v
JOIN battle b ON b.video_id = v.id
JOIN LATERAL (SELECT id FROM transcript
              WHERE battle_id = b.id AND status = 'done'
              ORDER BY id DESC LIMIT 1) t ON true
JOIN transcript_line l ON l.transcript_id = t.id
WHERE v.id = (SELECT id FROM video ORDER BY uploaded_at DESC LIMIT 1)
ORDER BY l.seq;
```

## Local development without Docker

Run the server and the page directly, with the compose Postgres:

```sh
docker compose -f "$REPO/compose.yaml" up -d postgres

# terminal 1: API on :8080. Uploads go to $REPO/server/data/videos (gitignored)
cargo run --release --manifest-path "$REPO/server/Cargo.toml"

# terminal 2: page on http://localhost:5173, with live reload; /api is proxied to :8080
npm --prefix "$REPO/frontend" install
npm --prefix "$REPO/frontend" run dev
```

Server settings (environment variables):

| Variable | Controls | Default |
| --- | --- | --- |
| `BIND` | Address the server listens on | `0.0.0.0:8080` |
| `DATABASE_URL` | Postgres connection | `postgres://pokedex:pokedex@127.0.0.1:5432/pokedex` |
| `VIDEO_DIR` | Where uploads are stored | `server/data/videos` |
| `ANALYZER_FFMPEG` | ffmpeg binary | the analyzer's pinned ffmpeg |
| `ANALYZER_ATLAS` | Glyph atlas file | the analyzer's committed atlas |

Front end: `API_URL` points the dev proxy somewhere other than `:8080`.
