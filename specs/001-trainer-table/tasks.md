# 001: Trainer table · Tasks

Small steps, in order. Each one ends in something that can be checked. Tick each box as
its step is done. Work happens on branch `trainer`, in `../pokemon_recordings-trainer`.

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
- [ ] 9. **Frontend:** api.ts additions and `TrainerSwitcher`.
  Check: `npm run build` passes.
- [ ] 10. **compose `DEV_AUTH`, Guide section, DeploymentPlan §6 note.**
- [ ] 11. **Converge.** Go through criteria 1–9 against the code, and report any gaps.
- [ ] 12. **User step: merge to `main`, rebuild the stack from the repo root.**
  Check: criteria 2 and 7 in the GUI.
- [ ] 13. **Close.** Set the spec's status to Done and tick the TODO item.
