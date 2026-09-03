//! Connection handling and per-connection pragmas.

use crate::error::Result;
use crate::migrate;
use rusqlite::Connection;
use std::path::Path;

/// Journal mode. WAL by default; `DELETE` exists as an escape hatch because WAL needs
/// shared memory and working POSIX locking, which some container filesystem shims
/// (Docker Desktop on macOS/Windows) do not provide. Set `POKEDEX_JOURNAL_MODE` to
/// change it without rebuilding.
fn journal_mode() -> String {
    std::env::var("POKEDEX_JOURNAL_MODE").unwrap_or_else(|_| "WAL".to_string())
}

pub struct Db {
    conn: Connection,
}

impl Db {
    /// Open (creating if absent) and apply per-connection pragmas.
    ///
    /// Deliberately does NOT migrate. A pipeline embedding this library decides when
    /// schema changes happen; only the CLI opts in, via `POKEDEX_AUTO_MIGRATE`.
    pub fn open<P: AsRef<Path>>(path: P) -> Result<Self> {
        let conn = Connection::open(path)?;
        Self::configure(&conn)?;
        Ok(Db { conn })
    }

    pub fn open_in_memory() -> Result<Self> {
        let conn = Connection::open_in_memory()?;
        Self::configure(&conn)?;
        Ok(Db { conn })
    }

    fn configure(conn: &Connection) -> Result<()> {
        // MUST be per-connection: `PRAGMA foreign_keys` is a no-op inside a
        // transaction, so it cannot live in a migration file.
        conn.pragma_update(None, "foreign_keys", "ON")?;
        conn.pragma_update(None, "journal_mode", journal_mode())?;
        conn.pragma_update(None, "busy_timeout", 5_000)?;
        conn.pragma_update(None, "synchronous", "NORMAL")?;
        Ok(())
    }

    pub fn conn(&self) -> &Connection {
        &self.conn
    }
    pub fn conn_mut(&mut self) -> &mut Connection {
        &mut self.conn
    }

    pub fn migrate(&mut self) -> Result<Vec<&'static str>> {
        migrate::migrate(&mut self.conn)
    }
    pub fn schema_version(&self) -> Result<i32> {
        migrate::current_version(&self.conn)
    }
    pub fn require_current_schema(&self) -> Result<()> {
        migrate::require_current(&self.conn)
    }

    /// Convenience for tests and for `--dry-run`: a fully migrated in-memory database.
    pub fn open_migrated_in_memory() -> Result<Self> {
        let mut db = Self::open_in_memory()?;
        db.migrate()?;
        Ok(db)
    }
}
