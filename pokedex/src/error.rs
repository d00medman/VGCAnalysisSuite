use std::fmt;

/// Errors the library can surface to a pipeline caller.
///
/// Ingest validation errors are separated from storage errors on purpose: a caller
/// batching thousands of records needs to distinguish "this record is malformed"
/// (skippable, report it) from "the database is unavailable" (abort the batch).
#[derive(Debug)]
pub enum Error {
    Sqlite(rusqlite::Error),
    Json(serde_json::Error),
    Io(std::io::Error),

    /// Schema version on disk is newer than this binary knows about.
    SchemaTooNew { found: i32, known: i32 },
    /// Caller asked for work on a database that has not been migrated.
    SchemaOutOfDate { found: i32, expected: i32 },

    /// A name in a record did not resolve to a row (unknown type, move, ability, ...).
    UnknownName { kind: &'static str, name: String },
    /// Referenced regulation does not exist.
    UnknownRegulation(String),

    /// Record violates an invariant SQLite cannot declare.
    Validation(String),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Sqlite(e) => write!(f, "sqlite: {e}"),
            Error::Json(e) => write!(f, "json: {e}"),
            Error::Io(e) => write!(f, "io: {e}"),
            Error::SchemaTooNew { found, known } => write!(
                f,
                "database schema version {found} is newer than this binary understands \
                 (max {known}); upgrade pokedex"
            ),
            Error::SchemaOutOfDate { found, expected } => write!(
                f,
                "database schema version {found}, expected {expected}; run `pokedex migrate`"
            ),
            Error::UnknownName { kind, name } => write!(f, "unknown {kind}: {name:?}"),
            Error::UnknownRegulation(n) => write!(f, "unknown regulation: {n:?}"),
            Error::Validation(m) => write!(f, "validation: {m}"),
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Error::Sqlite(e) => Some(e),
            Error::Json(e) => Some(e),
            Error::Io(e) => Some(e),
            _ => None,
        }
    }
}

impl From<rusqlite::Error> for Error {
    fn from(e: rusqlite::Error) -> Self {
        Error::Sqlite(e)
    }
}
impl From<serde_json::Error> for Error {
    fn from(e: serde_json::Error) -> Self {
        Error::Json(e)
    }
}
impl From<std::io::Error> for Error {
    fn from(e: std::io::Error) -> Self {
        Error::Io(e)
    }
}

pub type Result<T> = std::result::Result<T, Error>;
