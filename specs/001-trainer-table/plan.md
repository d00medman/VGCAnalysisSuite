# 001: Trainer table · Plan

Rev 1 · 2026-10-06 · Implements `spec.md` rev 2

## Approach

- **A `trainer` table, and an owner column on `video` and `battle`.** Transcripts, lines and
  turn marks hang off `battle`, so they're owned through it and need no column of their own.
- **One request extractor resolves the current trainer.** It's an axum `FromRequestParts`
  type called `Trainer`.
  - Every battle-side handler takes a `Trainer`.
  - Every store query that reads or changes battle-side rows filters on `trainer_id`.
  - Today the only resolver is the dev stub. Phase 2 adds the OIDC resolver to the same
    extractor, and the handlers don't change.
- **The dev stub is off unless `DEV_AUTH=1`.**
  - **Selection by cookie, not header.** The current trainer comes from a `dev_trainer=<id>`
    cookie. The browser fetches `<video>` and `<img>` sources itself and can't add custom
    headers to them, so a header would break the player and the live snapshot.
  - **No cookie:** the stub uses the oldest `admin` trainer.
  - **Off:** the extractor answers 401, and the `/api/dev/*` routes aren't mounted at all.
- **Display names aren't unique.** Public sign-up will want duplicates, and the switcher
  shows ids anyway. This settles the spec's non-blocking question.

Rejected:

- **Owner on `battle` only.** Stitching (stage D) will let a recording exist before it's
  assigned to a battle, and later span several battles' worth of clips. So the upload needs
  its own owner.
- **A header for the dev trainer.** It breaks media requests, as explained above.
- **Postgres row-level security.** Useful later as a safety net, but this change doesn't
  need it. Owner-filtered queries plus the isolation test cover it.

## Data model

**Migration:** `0008_trainers.sql`. This is the next position in `pokedex/src/migrate.rs`.
No other session in this repo is active, so no number clash.

```sql
CREATE TABLE trainer (
  id           BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
  display_name TEXT NOT NULL CHECK (length(btrim(display_name)) BETWEEN 1 AND 40),
  role         TEXT NOT NULL DEFAULT 'user' CHECK (role IN ('user','admin')),
  auth_subject TEXT UNIQUE,          -- OIDC `sub`, set in phase 2; NULL until then
  created_at   TIMESTAMPTZ NOT NULL DEFAULT now()
);

-- Existing rows get an owner: the user's trainer, created only if there is data to own.
-- A fresh database starts with no trainers, so a backup restores into it without clashing.
INSERT INTO trainer (display_name, role)
SELECT 'Alexander', 'admin' WHERE EXISTS (SELECT 1 FROM video);

ALTER TABLE video  ADD COLUMN trainer_id BIGINT REFERENCES trainer(id);
UPDATE video  SET trainer_id = (SELECT min(id) FROM trainer);
ALTER TABLE video  ALTER COLUMN trainer_id SET NOT NULL;
ALTER TABLE video  ADD UNIQUE (id, trainer_id);

ALTER TABLE battle ADD COLUMN trainer_id BIGINT;
UPDATE battle b SET trainer_id = v.trainer_id FROM video v WHERE v.id = b.video_id;
ALTER TABLE battle ALTER COLUMN trainer_id SET NOT NULL;
-- A battle and its recording always have the same owner.
ALTER TABLE battle ADD FOREIGN KEY (video_id, trainer_id)
  REFERENCES video (id, trainer_id) ON DELETE CASCADE;

CREATE INDEX idx_video_trainer  ON video  (trainer_id, uploaded_at DESC);
CREATE INDEX idx_battle_trainer ON battle (trainer_id);
```

- **Battle tables:** this change touches them, so it follows `devlog/SchemaIteration.md` §4.
  Back up, iterate on a scratch DB, commit, and only then let dev apply the migration.
- **Seeded name:** `'Alexander'`, taken from the git author. Change it here if you'd rather
  have something else, since there's no rename feature yet.
- **Fresh databases:** with `DEV_AUTH=1`, the server creates an `admin` trainer called
  `Dev` at startup if the table is empty. Without that, a fresh local stack would have no
  one to act as.
- **Old backups:** backups taken before 0008 have no `trainer_id`, so they only restore into
  a 0007 schema, which you then migrate. This is the existing data-only dump rule from
  SchemaIteration §5.3, nothing new.

## API

| Route | Trainer | Change |
|---|---|---|
| `GET/POST /api/videos`, `/api/videos/:id/**` | required | Scoped to the current trainer. Another trainer's id returns 404. |
| `/api/pokedex/**`, `/api/health` | none | Unchanged and public. |
| `GET /api/me` | required | New. Returns `{id, display_name, role}`. |
| `GET /api/dev/trainers` | dev only | New. Returns every trainer. |
| `POST /api/dev/trainers` | dev only | New. Takes `{display_name, role?}` and returns the new trainer. |

- **Selecting a trainer:** the frontend sets the `dev_trainer` cookie itself
  (`SameSite=Strict`, `Path=/`). There's no endpoint for it.
- **Unknown trainer id:** a cookie naming a trainer that doesn't exist gets a 401, so a
  stale cookie can never act as someone else.

## Components touched

- **pokedex:**
  - `migrations/0008_trainers.sql`
  - `src/migrate.rs` gets one new line in `MIGRATIONS`
- **server:**
  - `src/auth.rs`, new: the `Trainer` extractor, the dev stub, and the `/api/dev` routes
  - `src/store.rs`:
    - `trainer_id` parameters on `create`, `get`, `list`, `queue`, `transcript` and
      `set_turn_end`
    - trainer queries
    - `ensure_dev_trainer`
  - `src/main.rs`: handlers take `Trainer`, plus router assembly and the `DEV_AUTH` setting
  - The `Router` is built by a function, so tests can drive it without a socket
- **frontend:**
  - `src/api.ts`: `me`, `devTrainers`, `createDevTrainer` and `selectDevTrainer`, which
    sets the cookie
  - `src/App.tsx`: a `TrainerSwitcher` in the header, shown only when `/api/dev/trainers`
    answers. Switching clears the selected battle and reloads the history.
- **compose.yaml:** add `DEV_AUTH: "1"` to `api`
- **scripts:**
  - `backup-battles.sh`: add `-t trainer`
  - `scratch-db.sh`, new: SchemaIteration proposal 5.1, a throwaway Postgres on 55432 with
    the pokedex rebuilt and the latest battle backup restored. Criterion 1 needs it, and it
    was already done by hand twice.
- **devlog/Guide.md:** a short section on trainers and the switcher

Deferred: SchemaIteration proposals 5.2 (migration checksums, keyed by name) and 5.5 (stop
the API migrating on start). They're worth doing, but this change doesn't depend on them.
They're listed under Risks below.

## Testing

| Criterion | How |
|---|---|
| 1. Migration on existing data | User step: `scratch-db.sh` with the latest backup, then migrate, then compare video and battle counts per trainer against the backup. Claude gives the commands. |
| 2. GUI unchanged locally | User step: rebuild the stack, then upload, transcribe, browse history, play and mark turns. |
| 3. New uploads owned | Automated: upload through the router as trainer A, then check `video.trainer_id` and `battle.trainer_id`. |
| 4. Isolation | Automated: two trainers, each with a battle. As A, every battle-side route on B's video returns 404, and B's video is missing from A's list. |
| 5. Pokedex public | Automated: `/api/pokedex/regulations` returns 200 with no cookie and with `DEV_AUTH` off. |
| 6. Backups | User step: back up, restore into a fresh scratch DB, then compare counts. |
| 7. Switcher by hand | User step in the GUI, following the spec's steps. |
| 8. Stub off | Automated: with `DEV_AUTH` off, `/api/videos` and `/api/me` return 401, and `/api/dev/trainers` returns 404. |
| 9. Roles | Automated: inserting role `'owner'` fails, a new trainer defaults to `user`, and the seeded trainer is `admin`. |

**Server test harness (new):** the server has no tests today. They'll follow the pokedex
pattern: each test gets a fresh Postgres schema, using `options=-c search_path=…` in the
connection URL, migrates it, and drives the `Router` with `tower::ServiceExt::oneshot`.
Tests insert rows through the store, so no video files or ffmpeg are needed. They need the
compose Postgres running, as the pokedex tests already do.

## Risks

- **A forgotten filter leaks data.** If a store query misses its `trainer_id` filter, one
  trainer can see another's battle.
  - **Mitigation:** every store method that touches battle-side rows takes `trainer_id` as a
    required parameter, so a missed one won't compile.
  - **Mitigation:** the isolation test walks every battle-side route.
- **The dev stub ships by accident.** It's off unless `DEV_AUTH=1`, and criterion 8 tests
  the off state. The phase 2 deploy config must not set it. That gets added to
  `DeploymentPlan.md` §6.
- **The dev DB applies 0008 too early.** The API migrates on start, so rebuilding the stack
  from the repo root after merging applies it immediately. Merge only after the scratch-DB
  check and a fresh backup. Proposal 5.5 would remove this risk, and it's deferred.
- **In-flight live state:** the `live` map is keyed by video id and only read after an
  ownership-checked lookup, so it can't leak another trainer's progress.
