# 001: Trainer table

Status: Approved · Rev 2 · 2026-10-06
Serves: vision §2 ("multi-user from day one") and §4 (Trainer). This is the foundation for
capabilities 2–5.

`Decided:` marks things agreed with the user. `Open:` marks questions still to settle.

## Problem

Nothing in the database belongs to anyone. Videos, battles, transcripts and turn marks are
global, and the API shows every row to every caller.

The vision commits to multi-user from day one. Every later feature hangs data off a
trainer: results, My Pokémon, teams, stats, and inferred data. If we add ownership now, the
schema has an owner while it's small. If we retrofit it later, every table those features
add has to be migrated too.

## Behaviour

- **There is a trainer table.** A trainer is an account.
  - **Decided:** a trainer has a display name, and nothing else user-facing for now.
  - **Decided:** a trainer has a role, either `user` or `admin`. Nothing is admin-only yet,
    because pokedex imports run through the CLI, not the API. The role exists so later
    admin features have something to check.
- **Every recording and battle belongs to exactly one trainer.** So does everything that
  hangs off them: transcripts, lines and turn marks.
- **The API acts as one trainer per request,** and only shows or changes that trainer's
  data. Another trainer's battle looks the same as one that doesn't exist (404).
- **Pokedex data stays public.** It isn't owned by anyone.
- **Existing data belongs to the user's own trainer.** It's assigned by the migration, so
  nothing is lost. That trainer is an `admin`.
- **Locally, without real login,** there's a dev auth stub.
  - **Decided:** the GUI has a trainer switcher, so the user can try multi-user behaviour
    by hand. It lists the trainers, can create a new one with a display name and role,
    and switches which trainer the page acts as.
  - The default is the user's own trainer, so the GUI otherwise works as it does today.
  - The stub and the switcher are enabled by an explicit dev setting. With it off, they
    don't exist, so they can never ship to a deployed site by accident.

## Scope

**In:**
- The trainer table and owner links on the battle-side data.
- A dev-only auth stub that resolves the current trainer for each request.
- A dev-only trainer switcher in the GUI.
- A role on each trainer.
- Scoping every battle-side API route to the current trainer.
- Moving existing rows to the user's trainer.
- Adding the new table to `scripts/backup-battles.sh`.

**Out (non-goals):**
- Real login and an auth provider. That's Phase 2 (`devlog/DeploymentPlan.md` §6). The table
  should leave room for it, such as a column for the provider's subject id, but nothing
  validates tokens yet.
- Sign-up, profile and account-settings pages.
- Sharing battles between trainers.
- Admin tooling, and any route gated on the admin role.
- Profile fields beyond a display name, such as in-game name or player id.

## Acceptance criteria

1. **The migration works on existing data.** It applies cleanly to a copy of the dev
   database restored from a battle backup. Afterwards, every existing video and battle
   belongs to the user's trainer, and the counts match the backup.
2. **The GUI is unchanged locally.** With the stub, upload, transcribe, history, the player
   and turn marks all behave as they did before.
3. **New data gets an owner.** A recording uploaded locally belongs to the dev trainer.
4. **Trainers can't see each other's data.** An automated test creates two trainers, each
   with a battle. Trainer A:
   - can't list trainer B's battle
   - can't fetch it, transcribe it, or play its preview
   - can't mark turns on it

   Every one of those requests returns 404.
5. **The pokedex stays public.** Its routes answer the same with any trainer, or none.
6. **Backups include trainers.** `scripts/backup-battles.sh` includes the trainer table,
   and a backup restores into a freshly migrated database.
7. **Switching trainers works by hand.** With the dev setting on, the user can:
   - create a second trainer in the GUI
   - switch to it, and see an empty history
   - upload a recording as that trainer
   - switch back, and see only their own battles
8. **The stub can be switched off.** With the dev setting off, the stub and the switcher
   are absent from the API and the GUI. Battle-side requests are refused (401) rather than
   served as a default trainer.
9. **Roles are stored.** Every trainer has a role of `user` or `admin`, and the database
   rejects any other value. The user's migrated trainer is `admin`, and new trainers default
   to `user`.

## Data

The trainer table is **irreplaceable**, so it goes into the battle backups. Ownership links
are irreplaceable as well, which means they live on battle-side tables and never on the
pokedex.

## Open questions

Blocking: none. Answered 2026-10-06:

1. **What a trainer holds:** a display name is enough.
2. **Local dev:** the GUI gets a trainer switcher.
3. **Roles:** add them now.

Not blocking (answered in `plan.md`):

- **Where the owner link lives.** On `video`, on `battle`, or both? Stitching will let one
  battle span several videos, so the link probably belongs on both, with a check that they
  agree.
- **Schema groundwork first.** `devlog/SchemaIteration.md` §5 suggests some groundwork
  before the next battle-table change:
  - a scratch-DB script
  - checksums on migrations, keyed by name
  - stopping the API from migrating on start

  Plan proposes which of these to do first.
- **Migration number.** It will claim `0008`.
- **Display names.** Must they be unique? Unique names make the switcher unambiguous, but
  public sign-up later may want duplicates allowed.
