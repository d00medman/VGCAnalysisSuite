//! Migration runner.
//!
//! Hand-rolled rather than refinery/sqlx: ~40 lines, stays synchronous, and adds no
//! dependency. Each migration is embedded at compile time, so the runtime container
//! ships one binary with no `migrations/` directory to mount and no chance of the
//! image drifting from the SQL on disk.

use crate::error::{Error, Result};
use rusqlite::Connection;

/// Ordered, 1-indexed. `user_version` on the database stores how many have been applied.
const MIGRATIONS: &[(&str, &str)] = &[
    ("0001_init", include_str!("../migrations/0001_init.sql")),
    ("0002_seed_types", include_str!("../migrations/0002_seed_types.sql")),
    ("0003_seed_ref", include_str!("../migrations/0003_seed_ref.sql")),
];

pub const LATEST_VERSION: i32 = MIGRATIONS.len() as i32;

pub fn current_version(conn: &Connection) -> Result<i32> {
    Ok(conn.query_row("PRAGMA user_version", [], |r| r.get(0))?)
}

/// Applied migration names, in the order they ran.
pub fn pending(conn: &Connection) -> Result<Vec<&'static str>> {
    let v = current_version(conn)?;
    check_not_newer(v)?;
    Ok(MIGRATIONS[v as usize..].iter().map(|(n, _)| *n).collect())
}

fn check_not_newer(found: i32) -> Result<()> {
    if found > LATEST_VERSION {
        return Err(Error::SchemaTooNew { found, known: LATEST_VERSION });
    }
    Ok(())
}

/// Apply every migration the database has not yet seen. Returns the names applied.
///
/// Each migration runs in its own transaction together with the `user_version` bump,
/// so a failure part-way leaves the database at the last fully-applied version rather
/// than in a half-migrated state.
pub fn migrate(conn: &mut Connection) -> Result<Vec<&'static str>> {
    let from = current_version(conn)?;
    check_not_newer(from)?;

    let mut applied = Vec::new();
    for (i, (name, sql)) in MIGRATIONS.iter().enumerate().skip(from as usize) {
        let version = i as i32 + 1;
        let tx = conn.transaction()?;
        tx.execute_batch(sql)?;
        // PRAGMA does not accept bound parameters, and `version` is derived from a
        // compile-time array index, never from input.
        tx.pragma_update(None, "user_version", version)?;
        tx.commit()?;
        applied.push(*name);
    }
    Ok(applied)
}

/// Error unless the database is fully migrated. Used by commands that read or write
/// data, so a caller never silently operates against a stale schema.
pub fn require_current(conn: &Connection) -> Result<()> {
    let found = current_version(conn)?;
    check_not_newer(found)?;
    if found != LATEST_VERSION {
        return Err(Error::SchemaOutOfDate { found, expected: LATEST_VERSION });
    }
    Ok(())
}
