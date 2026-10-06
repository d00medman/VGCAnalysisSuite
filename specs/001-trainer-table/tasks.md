# 001: Trainer table · Tasks

Small steps, in order. Each one ends in something that can be checked. Tick each box as
its step is done. Tasks 1–10 were done on branch `trainer` in a worktree, then merged.

- [x] 1. **`scripts/scratch-db.sh`.** Throwaway Postgres on 55432, pokedex rebuilt, latest
  battle backup restored.
  Check: run it, and the counts match the backup.
- [x] 2. **User step: take a fresh battle backup of dev** with
  `scripts/backup-battles.sh`.
- [x] 3. **Migration `0008_trainers.sql`** and its line in `migrate.rs`. Iterate against the
  scratch DB.
  Check: criterion 1, video and battle counts per trainer.
- [x] 4. **Server test harness.** Schema-per-test, and a `Router` built by a function.
  Check: one smoke test passes against `/api/health`.
- [x] 5. **Store:** trainer queries, `ensure_dev_trainer`, and `trainer_id` on every
  battle-side method.
  Check: it compiles, and the existing behaviour is unchanged for one trainer.
- [x] 6. **`auth.rs`:** the `Trainer` extractor, the `DEV_AUTH` setting, `/api/me` and
  `/api/dev/trainers`.
  Check: tests for criteria 8 and 9.
- [x] 7. **Handlers take `Trainer`.**
  Check: tests for criteria 3, 4 and 5.
- [x] 8. **`backup-battles.sh` includes `trainer`.**
  Check: criterion 6. Back up, restore into a fresh scratch DB, compare counts.
- [x] 9. **Frontend:** api.ts additions and `TrainerSwitcher`.
  Check: `npm run build` passes.
- [x] 10. **compose `DEV_AUTH`, Guide section, DeploymentPlan §6 note.**
- [x] 11. **Converge.** Go through criteria 1–9 against the code, and report any gaps.
  See *Converge* below.
- [x] 12. **Merge to `main` and rebuild the stack from the repo root.** Merged
  2026-10-06 (fast-forward). The user rebuilt the stack; dev applied 0008. The user
  checked criteria 2 and 7 in the GUI and confirmed the switcher works.
- [x] 13. **Close.** Set the spec's status to Done and tick the TODO item.

## Converge (2026-10-06)

| # | Criterion | Result | Evidence |
|---|---|---|---|
| 1 | Migration on existing data | ✅ | Scratch DB from the 2026-10-06 backup, then live dev: 1 video and 1 battle, both owned by Alexander (admin). No row lacks an owner. |
| 2 | GUI unchanged locally | ✅ | Live stack with no cookie: list, detail (16 lines, 1 turn mark) and preview range request (206) all work. The browser walkthrough is the user's. |
| 3 | New uploads owned | ✅ | `tests::uploads_belong_to_the_uploader` |
| 4 | Isolation | ✅ | `tests::trainers_cannot_see_or_touch_each_others_battles`. It covers every battle route. Removing the owner filter from `Store::get` makes it fail. |
| 5 | Pokedex public | ✅ | Two tests, plus the live stack with a stale cookie (200). |
| 6 | Backups include trainers | ✅ | A schema-8 backup restores into a fresh scratch DB with the same counts, and the id sequences carry over. |
| 7 | Switching by hand | ✅ | The user's GUI walkthrough, 2026-10-06. API parts are tested. |
| 8 | Stub can be switched off | ✅ | `tests::without_dev_auth_battle_routes_refuse_and_pokedex_is_public` |
| 9 | Roles stored | ✅ | Pokedex test `trainers_have_a_role_and_battles_share_their_recordings_owner`, and the server's dev-stub test |

**Gap found and fixed:** a `dev_trainer` cookie naming a trainer that no longer exists made
`/api/me` fail. The switcher then hid itself, leaving no way to pick a valid trainer. It now
still shows, with "Pick a trainer" and an error explaining why.

**Not covered by any check:** the frontend has no automated tests, so criteria 2 and 7 rest
on the user's walkthrough.
