//! Regulation management and the derived time window.
//!
//! `effective_to` is never stored — it is `LEAD(effective_from)` over the ordering.
//! That makes overlaps and gaps unrepresentable, and lets a regulation be inserted
//! retroactively between two existing ones with the chain re-linking itself.

use crate::error::{Error, Result};
use rusqlite::{Connection, OptionalExtension};

#[derive(Debug, Clone)]
pub struct Regulation {
    pub id: i64,
    pub name: String,
    pub effective_from: String,
    pub notes: Option<String>,
}

#[derive(Debug, Clone)]
pub struct Window {
    pub id: i64,
    pub name: String,
    pub effective_from: String,
    /// `None` = current.
    pub effective_to: Option<String>,
    pub prev_regulation_id: Option<i64>,
    pub next_regulation_id: Option<i64>,
}

fn valid_date(s: &str) -> bool {
    let b = s.as_bytes();
    b.len() == 10
        && b[4] == b'-'
        && b[7] == b'-'
        && b.iter().enumerate().all(|(i, c)| {
            matches!(i, 4 | 7) || c.is_ascii_digit()
        })
}

pub fn add(conn: &Connection, name: &str, effective_from: &str, notes: Option<&str>) -> Result<i64> {
    if !valid_date(effective_from) {
        return Err(Error::Validation(format!(
            "effective_from must be ISO-8601 YYYY-MM-DD, got {effective_from:?}"
        )));
    }
    conn.execute(
        "INSERT INTO regulation (name, effective_from, notes) VALUES (?1, ?2, ?3)",
        rusqlite::params![name, effective_from, notes],
    )?;
    Ok(conn.last_insert_rowid())
}

pub fn by_name(conn: &Connection, name: &str) -> Result<Regulation> {
    conn.query_row(
        "SELECT id, name, effective_from, notes FROM regulation WHERE name = ?1",
        [name],
        |r| {
            Ok(Regulation {
                id: r.get(0)?,
                name: r.get(1)?,
                effective_from: r.get(2)?,
                notes: r.get(3)?,
            })
        },
    )
    .optional()?
    .ok_or_else(|| Error::UnknownRegulation(name.to_string()))
}

/// Every regulation in chronological order, with derived windows.
pub fn windows(conn: &Connection) -> Result<Vec<Window>> {
    let mut stmt = conn.prepare(
        "SELECT id, name, effective_from, effective_to, prev_regulation_id, next_regulation_id
         FROM regulation_window ORDER BY effective_from",
    )?;
    let rows = stmt.query_map([], |r| {
        Ok(Window {
            id: r.get(0)?,
            name: r.get(1)?,
            effective_from: r.get(2)?,
            effective_to: r.get(3)?,
            prev_regulation_id: r.get(4)?,
            next_regulation_id: r.get(5)?,
        })
    })?;
    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
}

/// The regulation immediately preceding `id`, if any.
///
/// Ingest needs this to resolve "what was the value before this regulation?" when
/// deciding whether a scalar fact actually changed.
pub fn previous(conn: &Connection, id: i64) -> Result<Option<Regulation>> {
    Ok(conn
        .query_row(
            "SELECT p.id, p.name, p.effective_from, p.notes
             FROM regulation r
             JOIN regulation p ON p.effective_from < r.effective_from
             WHERE r.id = ?1
             ORDER BY p.effective_from DESC LIMIT 1",
            [id],
            |r| {
                Ok(Regulation {
                    id: r.get(0)?,
                    name: r.get(1)?,
                    effective_from: r.get(2)?,
                    notes: r.get(3)?,
                })
            },
        )
        .optional()?)
}
