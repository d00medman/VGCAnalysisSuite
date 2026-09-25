//! Migration runner.
//!
//! Hand-rolled rather than refinery/sqlx: ~50 lines, stays synchronous, and adds no
//! dependency. Each migration is embedded at compile time, so the runtime container
//! ships one binary with no `migrations/` directory to mount and no chance of the
//! image drifting from the SQL on disk.

use crate::error::{Error, Result};
use postgres::{Client, GenericClient};

/// Ordered, 1-indexed. `schema_migrations` records which have been applied.
const MIGRATIONS: &[(&str, &str)] = &[
    ("0001_init", include_str!("../migrations/0001_init.sql")),
    ("0002_seed_types", include_str!("../migrations/0002_seed_types.sql")),
    ("0003_seed_ref", include_str!("../migrations/0003_seed_ref.sql")),
    ("0004_items", include_str!("../migrations/0004_items.sql")),
    ("0005_battles", include_str!("../migrations/0005_battles.sql")),
    ("0006_display_names", include_str!("../migrations/0006_display_names.sql")),
    ("0007_turns", include_str!("../migrations/0007_turns.sql")),
];

pub const LATEST_VERSION: i32 = MIGRATIONS.len() as i32;

/// Advisory lock key serializing migrators, so two containers starting at once cannot
/// both apply the same migration. Arbitrary; ASCII "pokedex".
const LOCK_KEY: i64 = 0x0070_6f6b_6564_6578;

pub fn current_version(c: &mut impl GenericClient) -> Result<i32> {
    // Resolved through search_path, like every other table here.
    let exists: bool = c
        .query_one("SELECT to_regclass('schema_migrations') IS NOT NULL", &[])?
        .get(0);
    if !exists {
        return Ok(0);
    }
    Ok(c.query_one("SELECT coalesce(max(version), 0) FROM schema_migrations", &[])?.get(0))
}

/// Names of migrations not yet applied, in the order they would run.
pub fn pending(c: &mut impl GenericClient) -> Result<Vec<&'static str>> {
    let v = current_version(c)?;
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
/// Postgres DDL is transactional, so all pending migrations run in ONE transaction:
/// a failure part-way leaves the database exactly as it was.
pub fn migrate(client: &mut Client) -> Result<Vec<&'static str>> {
    let mut tx = client.transaction()?;
    tx.execute("SELECT pg_advisory_xact_lock($1)", &[&LOCK_KEY])?;
    // Created under the lock: concurrent CREATE TABLE IF NOT EXISTS can still race.
    tx.batch_execute(
        "CREATE TABLE IF NOT EXISTS schema_migrations (
           version    INTEGER PRIMARY KEY,
           name       TEXT NOT NULL,
           applied_at TIMESTAMPTZ NOT NULL DEFAULT now()
         )",
    )?;
    let from = current_version(&mut tx)?;
    check_not_newer(from)?;

    let mut applied = Vec::new();
    for (i, (name, sql)) in MIGRATIONS.iter().enumerate().skip(from as usize) {
        let version = i as i32 + 1;
        tx.batch_execute(sql)?;
        tx.execute(
            "INSERT INTO schema_migrations (version, name) VALUES ($1, $2)",
            &[&version, name],
        )?;
        applied.push(*name);
    }
    tx.commit()?;
    Ok(applied)
}

/// Error unless the database is fully migrated. Used by commands that read or write
/// data, so a caller never silently operates against a stale schema.
pub fn require_current(c: &mut impl GenericClient) -> Result<()> {
    let found = current_version(c)?;
    check_not_newer(found)?;
    if found != LATEST_VERSION {
        return Err(Error::SchemaOutOfDate { found, expected: LATEST_VERSION });
    }
    Ok(())
}
