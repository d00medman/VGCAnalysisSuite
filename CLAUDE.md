# Working in this repo

## Specs
- Features are spec-driven. Read `specs/README.md` for the loop, and `specs/vision.md` for
  what the product is.
- Non-trivial work needs an approved `specs/NNN-name/spec.md` before implementation starts.
- If the code has to diverge from a spec, update the spec first and tell the user.

## Git workflow
- **One session (the usual case):** work and commit directly on `main` in the repo root. No
  branches or worktrees. Before committing, check `ListAgents` to confirm no other session
  is working on this project.
- **Several sessions at once:** each feature gets its own branch in its own worktree beside
  the repo: `git worktree add ../pokemon_recordings-<feature> -b <feature>`. The repo root
  stays on `main`, the integration branch, because switching branches there switches it for
  everyone. Merge finished work into `main` with a fast-forward where possible, send the
  other session the commit hash, and remove the worktree.
- **Explain worktrees.** The user doesn't use them day to day. Before creating one, say why
  it's needed and where it will live.
- Use `SendMessage` to tell other sessions which files you're touching. Push only when the
  user says to.

## Shared state outside git
- **Migrations:** the runner in `pokedex/src/migrate.rs` treats list position as the
  version number. Tell other sessions the number you're taking. Never edit a migration the
  dev database has already applied; add a new one instead.
- **Untested migrations stay out of the dev database.** The API applies pending migrations
  when it starts, so a migration reaches dev the next time the stack is rebuilt from the
  repo root. Try it first against `scripts/scratch-db.sh` (a throwaway copy of the data),
  and take a backup (`scripts/backup-battles.sh`) before the rebuild.
- **Docker:** for now, run `docker compose` only from the repo root. Compose names the
  project after the folder and the ports are fixed, so a stack started in a worktree clashes
  on ports 8080 and 5432 and has an empty database. Per-branch stacks are on the roadmap
  (`specs/vision.md` §9).
- **Container rebuilds** pick up whatever is in the repo root's files at that moment,
  including another session's unfinished work.
