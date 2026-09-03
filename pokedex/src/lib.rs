//! pokedex — SQLite pokedex schema and regulation-aware ingest.
//!
//! The library is the API; the CLI in `main.rs` is a thin shell over it. A pipeline
//! stage constructs domain values from `model` and hands them to `ingest`, never
//! touching JSON or surrogate keys.
//!
//! See `../devlog/SchemaAndIngestPlan.md` for design rationale, and
//! `migrations/0001_init.sql` for the authoritative schema.

pub mod db;
pub mod error;
pub mod migrate;
pub mod model;
pub mod ingest;
pub mod regulation;
pub mod resolve;

pub use db::Db;
pub use error::{Error, Result};
