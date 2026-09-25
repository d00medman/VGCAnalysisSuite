-- 0007_turns.sql — where each turn of a battle ends.
--
-- A turn is the run of transcript lines up to and including a line marked here, so
-- turns are derived, not stored: turn N holds the lines after the (N-1)th mark through
-- the Nth. Lines after the last mark are the turn still open. Marks point at lines of
-- one transcript run; a re-transcription starts with none.
--
-- Today marks are placed by hand in the web GUI. `source` leaves room for marks a
-- later pass imputes, so hand-placed ones can be told apart and kept.
CREATE TABLE turn_end (
  line_id   BIGINT PRIMARY KEY REFERENCES transcript_line(id) ON DELETE CASCADE,
  source    TEXT NOT NULL DEFAULT 'manual' CHECK (source IN ('manual','imputed')),
  marked_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
