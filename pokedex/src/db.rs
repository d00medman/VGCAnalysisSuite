//! Connection handling.

use crate::error::Result;
use crate::migrate;
use postgres::{Client, NoTls};

pub struct Db {
    client: Client,
}

impl Db {
    /// Connect with a libpq-style URL or key=value string, e.g.
    /// `postgres://pokedex:pokedex@localhost:5432/pokedex`.
    ///
    /// Deliberately does NOT migrate. A pipeline embedding this library decides when
    /// schema changes happen; only the CLI opts in, via `POKEDEX_AUTO_MIGRATE`.
    pub fn connect(url: &str) -> Result<Self> {
        Ok(Db { client: Client::connect(url, NoTls)? })
    }

    /// Wrap an already-configured client (TLS, search_path, ...).
    pub fn from_client(client: Client) -> Self {
        Db { client }
    }

    pub fn client(&mut self) -> &mut Client {
        &mut self.client
    }

    pub fn migrate(&mut self) -> Result<Vec<&'static str>> {
        migrate::migrate(&mut self.client)
    }
    pub fn schema_version(&mut self) -> Result<i32> {
        migrate::current_version(&mut self.client)
    }
    pub fn require_current_schema(&mut self) -> Result<()> {
        migrate::require_current(&mut self.client)
    }
}
