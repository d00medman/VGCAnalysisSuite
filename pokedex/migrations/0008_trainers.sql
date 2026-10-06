-- 0008_trainers.sql — trainers (accounts), and an owner on every recording and battle.
--
-- Spec: specs/001-trainer-table. Transcripts, lines and turn marks hang off battle, so
-- they are owned through it. A trainer and everything it owns is battle-side data: it
-- can't be rebuilt, so scripts/backup-battles.sh dumps it.

CREATE TABLE trainer (
  id           BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
  display_name TEXT NOT NULL CHECK (length(btrim(display_name)) BETWEEN 1 AND 40),
  role         TEXT NOT NULL DEFAULT 'user' CHECK (role IN ('user','admin')),
  auth_subject TEXT UNIQUE,          -- the login provider's subject id; NULL until login exists
  created_at   TIMESTAMPTZ NOT NULL DEFAULT now()
);

-- Existing rows get an owner: the user's trainer, created only if there is data to own.
-- A fresh database starts with no trainers, so a battle backup restores into it cleanly.
INSERT INTO trainer (display_name, role)
SELECT 'Alexander', 'admin' WHERE EXISTS (SELECT 1 FROM video);

ALTER TABLE video ADD COLUMN trainer_id BIGINT REFERENCES trainer(id);
UPDATE video SET trainer_id = (SELECT min(id) FROM trainer);
ALTER TABLE video ALTER COLUMN trainer_id SET NOT NULL;
ALTER TABLE video ADD CONSTRAINT video_id_trainer_key UNIQUE (id, trainer_id);

ALTER TABLE battle ADD COLUMN trainer_id BIGINT;
UPDATE battle b SET trainer_id = v.trainer_id FROM video v WHERE v.id = b.video_id;
ALTER TABLE battle ALTER COLUMN trainer_id SET NOT NULL;
-- A battle and its recording always have the same owner.
ALTER TABLE battle ADD CONSTRAINT battle_video_owner_fkey
  FOREIGN KEY (video_id, trainer_id) REFERENCES video (id, trainer_id) ON DELETE CASCADE;

CREATE INDEX idx_video_trainer ON video (trainer_id, uploaded_at DESC);
CREATE INDEX idx_battle_trainer ON battle (trainer_id);
