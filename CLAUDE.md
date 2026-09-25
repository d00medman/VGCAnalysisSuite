# Working in this repo

## Parallel Claude sessions
- Before running any git command that changes the branch or commits, check `ListAgents` and
  `git status` for other sessions. The repo root is one shared working tree, so switching
  branches there switches it for everyone.
- Each parallel feature gets its own branch in its own worktree beside the repo:
  `git worktree add ../pokemon_recordings-<feature> -b <feature>`.
- The repo root stays on `main`, the integration branch. Merge finished work there so both
  features can be tested in one build, then send the other session the commit hash.
- Use `SendMessage` to tell other sessions which files you're touching. Push only when the
  user says to.

## Shared state outside git
- **Migrations:** the runner in `pokedex/src/migrate.rs` treats list position as the
  version number. Tell other sessions the number you're taking. Never edit a migration the
  dev database has already applied; add a new one instead.
- **Docker:** run `docker compose` only from the repo root. Compose names the project after
  the folder, so running it in a worktree starts a second stack with an empty database that
  clashes on ports 8080 and 5432.
- **Container rebuilds** pick up whatever is in the repo root's files at that moment,
  including another session's unfinished work.
