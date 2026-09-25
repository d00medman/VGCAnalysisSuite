//! Regulation management and the derived time window.
//!
//! `effective_to` is never stored — it is `LEAD(effective_from)` over the ordering.
//! That makes overlaps and gaps unrepresentable, and lets a regulation be inserted
//! retroactively between two existing ones with the chain re-linking itself.

use crate::error::{Error, Result};
use postgres::{GenericClient, Row};

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

/// Expects `effective_from` selected as `::text`: it is a DATE column, and callers
/// get back the ISO-8601 string they wrote.
fn from_row(r: &Row) -> Regulation {
    Regulation { id: r.get(0), name: r.get(1), effective_from: r.get(2), notes: r.get(3) }
}

pub fn add(
    c: &mut impl GenericClient,
    name: &str,
    effective_from: &str,
    notes: Option<&str>,
) -> Result<i64> {
    // Stricter than Postgres' own date parsing, which would also take '2023/1/1'.
    if !valid_date(effective_from) {
        return Err(Error::Validation(format!(
            "effective_from must be ISO-8601 YYYY-MM-DD, got {effective_from:?}"
        )));
    }
    let row = c.query_one(
        "INSERT INTO regulation (name, effective_from, notes)
         VALUES ($1, $2::text::date, $3) RETURNING id",
        &[&name, &effective_from, &notes],
    )?;
    Ok(row.get(0))
}

pub fn by_name(c: &mut impl GenericClient, name: &str) -> Result<Regulation> {
    c.query_opt(
        "SELECT id, name, effective_from::text, notes FROM regulation WHERE name = $1",
        &[&name],
    )?
    .map(|r| from_row(&r))
    .ok_or_else(|| Error::UnknownRegulation(name.to_string()))
}

/// Every regulation in chronological order, with derived windows.
pub fn windows(c: &mut impl GenericClient) -> Result<Vec<Window>> {
    let rows = c.query(
        "SELECT id, name, effective_from::text, effective_to::text,
                prev_regulation_id, next_regulation_id
         FROM regulation_window ORDER BY effective_from",
        &[],
    )?;
    Ok(rows
        .iter()
        .map(|r| Window {
            id: r.get(0),
            name: r.get(1),
            effective_from: r.get(2),
            effective_to: r.get(3),
            prev_regulation_id: r.get(4),
            next_regulation_id: r.get(5),
        })
        .collect())
}

/// The regulation immediately preceding `id`, if any.
///
/// Ingest needs this to resolve "what was the value before this regulation?" when
/// deciding whether a scalar fact actually changed.
pub fn previous(c: &mut impl GenericClient, id: i64) -> Result<Option<Regulation>> {
    Ok(c
        .query_opt(
            "SELECT p.id, p.name, p.effective_from::text, p.notes
             FROM regulation r
             JOIN regulation p ON p.effective_from < r.effective_from
             WHERE r.id = $1
             ORDER BY p.effective_from DESC LIMIT 1",
            &[&id],
        )?
        .map(|r| from_row(&r)))
}
