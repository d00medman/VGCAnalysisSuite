-- 0005_battles.sql — recorded battles and their transcripts.
--
-- Written by the web server (server/src/store.rs), not by ingest. Lives in the same
-- migration chain as the pokedex so that later tables can link transcript lines to
-- pokemon, moves and items with real foreign keys.

-- One uploaded recording. The file itself lives on the server's video volume.
CREATE TABLE video (
  id          TEXT PRIMARY KEY,          -- server-generated; also the id in page URLs
  name        TEXT NOT NULL,             -- original filename as uploaded
  file        TEXT NOT NULL,             -- file name on the video volume
  size_bytes  BIGINT NOT NULL CHECK (size_bytes > 0),
  uploaded_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

-- One match. Today exactly one per video, hence UNIQUE; drop it if a recording ever
-- holds several battles. regulation_id is not known at upload and stays NULL until
-- something determines it.
CREATE TABLE battle (
  id            BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
  video_id      TEXT NOT NULL UNIQUE REFERENCES video(id) ON DELETE CASCADE,
  regulation_id BIGINT REFERENCES regulation(id),
  created_at    TIMESTAMPTZ NOT NULL DEFAULT now()
);

-- One transcription run. Transcribing again adds a row instead of replacing one, so
-- links made against an older run's lines stay valid. The newest run is current.
CREATE TABLE transcript (
  id          BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
  battle_id   BIGINT NOT NULL REFERENCES battle(id) ON DELETE CASCADE,
  status      TEXT NOT NULL DEFAULT 'queued'
              CHECK (status IN ('queued','transcribing','done','failed')),
  queued_at   TIMESTAMPTZ NOT NULL DEFAULT now(),
  started_at  TIMESTAMPTZ,
  finished_at TIMESTAMPTZ,
  error       TEXT,
  CHECK ((status = 'failed') = (error IS NOT NULL))
);

CREATE INDEX idx_transcript_battle ON transcript (battle_id, id);

-- At most one run in flight per battle.
CREATE UNIQUE INDEX transcript_one_active
    ON transcript (battle_id)
    WHERE status IN ('queued','transcribing');

-- One message line, in on-screen order. Each line is its own row with a stable id, so
-- later tables can link events (a move used, a pokemon sent out) to the exact line
-- they were read from, and `seq` gives the battle's timeline.
CREATE TABLE transcript_line (
  id            BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
  transcript_id BIGINT NOT NULL REFERENCES transcript(id) ON DELETE CASCADE,
  seq           INTEGER NOT NULL CHECK (seq >= 0),    -- 0-based order in the transcript
  t0            DOUBLE PRECISION NOT NULL,            -- video seconds: line appeared
  t1            DOUBLE PRECISION NOT NULL,            -- video seconds: line gone
  text          TEXT NOT NULL,
  conf          REAL NOT NULL,                        -- reader confidence, 0..1
  clean         BOOLEAN NOT NULL,                     -- false: unread glyphs, text has '?'
  UNIQUE (transcript_id, seq),
  CHECK (t1 >= t0)
);
