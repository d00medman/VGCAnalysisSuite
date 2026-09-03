//! Regulation-aware ingest: **feed dense, store sparse**.
//!
//! A caller submits a complete snapshot for one regulation. This layer diffs it against
//! what is already stored and writes only what changed:
//!
//! - **Scalar facts** (`pokemon_stats`, `move_data`) — resolve the value as of the
//!   previous regulation; write a row only if it differs. Unchanged entities cost zero
//!   rows.
//! - **Set membership** (`pokemon_type`, `pokemon_ability`, `pokemon_move`) — open an
//!   interval for anything new, close the interval of anything that disappeared, leave
//!   the rest untouched. **Close before open**: the one-open-window triggers enforce it.
//!
//! The pipeline never computes deltas. Re-submitting an identical snapshot is a no-op,
//! so idempotency holds.

use crate::error::{Error, Result};
use crate::model::*;
use crate::regulation::{self, Regulation};
use crate::resolve::Resolver;
use rusqlite::{params, Connection, OptionalExtension, Transaction};
use std::collections::HashSet;

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct IngestReport {
    pub pokemon_upserted: usize,
    pub variants_upserted: usize,
    pub stats_written: usize,
    pub stats_unchanged: usize,
    pub move_data_written: usize,
    pub move_data_unchanged: usize,
    pub types_opened: usize,
    pub types_closed: usize,
    pub abilities_opened: usize,
    pub abilities_closed: usize,
    pub learnset_opened: usize,
    pub learnset_closed: usize,
}

impl IngestReport {
    /// True when the snapshot changed nothing — the signal that an import was a no-op.
    pub fn is_noop(&self) -> bool {
        self.stats_written == 0
            && self.move_data_written == 0
            && self.types_opened + self.types_closed == 0
            && self.abilities_opened + self.abilities_closed == 0
            && self.learnset_opened + self.learnset_closed == 0
    }
}

/// Apply a snapshot. Everything happens in one transaction: a malformed record aborts
/// the whole submission rather than leaving a half-applied regulation.
pub fn apply(conn: &mut Connection, snap: &Snapshot) -> Result<IngestReport> {
    let tx = conn.transaction()?;
    let report = apply_in(&tx, snap)?;
    tx.commit()?;
    Ok(report)
}

/// Apply inside a caller-owned transaction, so `--dry-run` can roll back and a pipeline
/// can batch several snapshots into one commit.
pub fn apply_in(tx: &Transaction, snap: &Snapshot) -> Result<IngestReport> {
    let reg = regulation::by_name(tx, &snap.regulation)?;
    let prev = regulation::previous(tx, reg.id)?;
    let mut r = Resolver::new();
    let mut rep = IngestReport::default();

    for a in &snap.abilities {
        r.ability_id(tx, &a.name, a.description.as_deref())?;
    }
    for m in &snap.moves {
        upsert_move_data(tx, &mut r, m, &reg, prev.as_ref(), &mut rep)?;
    }

    // Two passes over pokemon: `variant.base_pokemon_id` is a self-FK, so every base
    // row must exist before any variant referencing it.
    for p in &snap.pokemon {
        validate(p)?;
        upsert_pokemon(tx, &mut r, p, &mut rep)?;
    }
    for p in &snap.pokemon {
        if let Some(v) = &p.variant {
            upsert_variant(tx, &mut r, p, v, &mut rep)?;
        }
    }

    for p in &snap.pokemon {
        let pid = r
            .pokemon_id(tx, p.national_dex_no, &p.form_slug)?
            .expect("inserted in the pass above");
        if let Some(s) = &p.stats {
            upsert_stats(tx, pid, s, &reg, prev.as_ref(), &mut rep)?;
        }
        if let Some(types) = &p.types {
            sync_types(tx, &mut r, pid, types, &reg, &mut rep)?;
        }
        if let Some(ab) = &p.abilities {
            sync_abilities(tx, &mut r, pid, ab, &reg, &mut rep)?;
        }
        if let Some(ls) = &p.learnset {
            sync_learnset(tx, &mut r, pid, ls, &reg, &mut rep)?;
        }
    }
    Ok(rep)
}

/// Invariants SQLite cannot declare.
fn validate(p: &PokemonRecord) -> Result<()> {
    let who = format!("#{}{}", p.national_dex_no, if p.form_slug.is_empty() { String::new() } else { format!(" ({})", p.form_slug) });
    if let Some(types) = &p.types {
        // Positional ordering means slot 2 without slot 1 cannot be expressed, but an
        // empty-then-populated list still has to be caught.
        if types.len() > 2 {
            return Err(Error::Validation(format!(
                "{who}: {} types; a pokemon has at most 2",
                types.len()
            )));
        }
        if types.iter().any(|t| t.trim().is_empty()) {
            return Err(Error::Validation(format!("{who}: empty type name")));
        }
        // Replaces the dropped UNIQUE (pokemon_id, type_id, valid_from). See devlog
        // decision 6: it was a data-quality guard, not a structural one.
        if types.len() == 2 && types[0] == types[1] {
            return Err(Error::Validation(format!(
                "{who}: duplicate type {:?} in both slots",
                types[0]
            )));
        }
    }
    if let Some(v) = &p.variant {
        if v.base_form_slug == p.form_slug {
            return Err(Error::Validation(format!(
                "{who}: variant declares itself as its own base form"
            )));
        }
    }
    if let Some(ab) = &p.abilities {
        if ab.primary.is_none() && ab.secondary.is_some() {
            return Err(Error::Validation(format!(
                "{who}: secondary ability without a primary"
            )));
        }
    }
    Ok(())
}

fn upsert_pokemon(
    tx: &Transaction,
    r: &mut Resolver,
    p: &PokemonRecord,
    rep: &mut IngestReport,
) -> Result<i64> {
    tx.execute(
        "INSERT INTO pokemon (national_dex_no, form_slug, name, genus, height_dm, weight_hg)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)
         ON CONFLICT(national_dex_no, form_slug) DO UPDATE SET
           name      = excluded.name,
           genus     = COALESCE(excluded.genus,     pokemon.genus),
           height_dm = COALESCE(excluded.height_dm, pokemon.height_dm),
           weight_hg = COALESCE(excluded.weight_hg, pokemon.weight_hg)",
        params![p.national_dex_no, p.form_slug, p.name, p.genus, p.height_dm, p.weight_hg],
    )?;
    let id: i64 = tx.query_row(
        "SELECT id FROM pokemon WHERE national_dex_no = ?1 AND form_slug = ?2",
        params![p.national_dex_no, p.form_slug],
        |row| row.get(0),
    )?;
    r.cache_pokemon(p.national_dex_no, &p.form_slug, id);
    rep.pokemon_upserted += 1;
    Ok(id)
}

fn upsert_variant(
    tx: &Transaction,
    r: &mut Resolver,
    p: &PokemonRecord,
    v: &VariantRecord,
    rep: &mut IngestReport,
) -> Result<()> {
    let pid = r.pokemon_id(tx, p.national_dex_no, &p.form_slug)?.expect("just inserted");
    let base = r
        .pokemon_id(tx, p.national_dex_no, &v.base_form_slug)?
        .ok_or_else(|| {
            Error::Validation(format!(
                "#{} ({}): base form {:?} not present in this snapshot",
                p.national_dex_no, p.form_slug, v.base_form_slug
            ))
        })?;
    let kind = r.variant_kind_id(tx, &v.kind)?;
    tx.execute(
        "INSERT INTO variant (pokemon_id, base_pokemon_id, variant_kind_id, required_item)
         VALUES (?1, ?2, ?3, ?4)
         ON CONFLICT(pokemon_id) DO UPDATE SET
           base_pokemon_id = excluded.base_pokemon_id,
           variant_kind_id = excluded.variant_kind_id,
           required_item   = COALESCE(excluded.required_item, variant.required_item)",
        params![pid, base, kind, v.required_item],
    )?;
    rep.variants_upserted += 1;
    Ok(())
}

// ------------------------------------------------------------------ scalar facts

/// Write a stat row only when the values differ from what resolves at the previous
/// regulation. This is what keeps `pokemon_stats` sparse while callers feed dense.
fn upsert_stats(
    tx: &Transaction,
    pokemon_id: i64,
    s: &Stats,
    reg: &Regulation,
    prev: Option<&Regulation>,
    rep: &mut IngestReport,
) -> Result<()> {
    let inherited: Option<Stats> = match prev {
        None => None,
        Some(p) => tx
            .query_row(
                "SELECT base_hp, base_attack, base_defense, base_sp_attack, base_sp_defense, base_speed
                 FROM pokemon_stats_effective WHERE pokemon_id = ?1 AND regulation_id = ?2",
                params![pokemon_id, p.id],
                |row| {
                    Ok(Stats {
                        hp: row.get(0)?, attack: row.get(1)?, defense: row.get(2)?,
                        sp_attack: row.get(3)?, sp_defense: row.get(4)?, speed: row.get(5)?,
                    })
                },
            )
            .optional()?,
    };

    // Already-stored row for THIS regulation: a correction must still be applied.
    let stored_here: Option<Stats> = tx
        .query_row(
            "SELECT base_hp, base_attack, base_defense, base_sp_attack, base_sp_defense, base_speed
             FROM pokemon_stats WHERE pokemon_id = ?1 AND regulation_id = ?2",
            params![pokemon_id, reg.id],
            |row| {
                Ok(Stats {
                    hp: row.get(0)?, attack: row.get(1)?, defense: row.get(2)?,
                    sp_attack: row.get(3)?, sp_defense: row.get(4)?, speed: row.get(5)?,
                })
            },
        )
        .optional()?;

    if stored_here.is_none() && inherited.as_ref() == Some(s) {
        rep.stats_unchanged += 1;
        return Ok(());
    }
    if stored_here.as_ref() == Some(s) {
        rep.stats_unchanged += 1;
        return Ok(());
    }

    tx.execute(
        "INSERT INTO pokemon_stats
           (pokemon_id, regulation_id, base_hp, base_attack, base_defense,
            base_sp_attack, base_sp_defense, base_speed)
         VALUES (?1,?2,?3,?4,?5,?6,?7,?8)
         ON CONFLICT(pokemon_id, regulation_id) DO UPDATE SET
           base_hp=excluded.base_hp, base_attack=excluded.base_attack,
           base_defense=excluded.base_defense, base_sp_attack=excluded.base_sp_attack,
           base_sp_defense=excluded.base_sp_defense, base_speed=excluded.base_speed",
        params![pokemon_id, reg.id, s.hp, s.attack, s.defense, s.sp_attack, s.sp_defense, s.speed],
    )?;
    rep.stats_written += 1;
    Ok(())
}

/// Comparable projection of a move's versioned data.
type MoveTuple = (i64, String, Option<i64>, Option<i64>, Option<i64>, i64, Option<String>, Option<i64>);

fn upsert_move_data(
    tx: &Transaction,
    r: &mut Resolver,
    m: &MoveRecord,
    reg: &Regulation,
    prev: Option<&Regulation>,
    rep: &mut IngestReport,
) -> Result<()> {
    let move_id = r.move_id(tx, &m.name)?;
    let type_id = r.type_id(tx, &m.type_name)?;
    let incoming: MoveTuple = (
        type_id,
        m.damage_class.as_str().to_string(),
        m.power, m.accuracy, m.pp, m.priority,
        m.secondary_effect.clone(), m.effect_chance,
    );

    let read = |sql: &str, a: i64, b: i64| -> Result<Option<MoveTuple>> {
        Ok(tx
            .query_row(sql, params![a, b], |row| {
                Ok((
                    row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?,
                    row.get(4)?, row.get(5)?, row.get(6)?, row.get(7)?,
                ))
            })
            .optional()?)
    };

    let inherited = match prev {
        None => None,
        Some(p) => read(
            "SELECT type_id, damage_class, power, accuracy, pp, priority, secondary_effect, effect_chance
             FROM move_data_effective WHERE move_id = ?1 AND regulation_id = ?2",
            move_id, p.id,
        )?,
    };
    let stored_here = read(
        "SELECT type_id, damage_class, power, accuracy, pp, priority, secondary_effect, effect_chance
         FROM move_data WHERE move_id = ?1 AND regulation_id = ?2",
        move_id, reg.id,
    )?;

    if (stored_here.is_none() && inherited.as_ref() == Some(&incoming))
        || stored_here.as_ref() == Some(&incoming)
    {
        rep.move_data_unchanged += 1;
        return Ok(());
    }

    tx.execute(
        "INSERT INTO move_data
           (move_id, regulation_id, type_id, damage_class, power, accuracy, pp, priority,
            secondary_effect, effect_chance)
         VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10)
         ON CONFLICT(move_id, regulation_id) DO UPDATE SET
           type_id=excluded.type_id, damage_class=excluded.damage_class,
           power=excluded.power, accuracy=excluded.accuracy, pp=excluded.pp,
           priority=excluded.priority, secondary_effect=excluded.secondary_effect,
           effect_chance=excluded.effect_chance",
        params![
            move_id, reg.id, type_id, m.damage_class.as_str(), m.power, m.accuracy,
            m.pp, m.priority, m.secondary_effect, m.effect_chance
        ],
    )?;
    rep.move_data_written += 1;
    Ok(())
}

// ------------------------------------------------------------- set membership
//
// All three follow the same shape: read the currently-open set, close what is gone,
// open what is new, leave matches untouched. Closing happens first because the
// one-open-window triggers reject a second open row for a key.

fn close_open(
    tx: &Transaction,
    table: &str,
    key_sql: &str,
    params: &[&dyn rusqlite::ToSql],
    reg_id: i64,
) -> Result<usize> {
    let sql = format!(
        "UPDATE {table} SET valid_to_regulation_id = ?1
         WHERE valid_to_regulation_id IS NULL AND {key_sql}"
    );
    let mut all: Vec<&dyn rusqlite::ToSql> = vec![&reg_id];
    all.extend_from_slice(params);
    Ok(tx.execute(&sql, all.as_slice())?)
}

fn sync_types(
    tx: &Transaction,
    r: &mut Resolver,
    pokemon_id: i64,
    types: &[String],
    reg: &Regulation,
    rep: &mut IngestReport,
) -> Result<()> {
    let mut want: Vec<(i64, i64)> = Vec::new(); // (slot, type_id)
    for (i, name) in types.iter().enumerate() {
        want.push((i as i64 + 1, r.type_id(tx, name)?));
    }

    let mut stmt = tx.prepare(
        "SELECT slot, type_id FROM pokemon_type
         WHERE pokemon_id = ?1 AND valid_to_regulation_id IS NULL",
    )?;
    let have: Vec<(i64, i64)> = stmt
        .query_map([pokemon_id], |row| Ok((row.get(0)?, row.get(1)?)))?
        .collect::<rusqlite::Result<_>>()?;
    drop(stmt);

    for (slot, tid) in &have {
        if !want.contains(&(*slot, *tid)) {
            rep.types_closed += close_open(
                tx, "pokemon_type", "pokemon_id = ?2 AND slot = ?3",
                &[&pokemon_id, slot], reg.id,
            )?;
        }
    }
    for (slot, tid) in &want {
        if !have.contains(&(*slot, *tid)) {
            tx.execute(
                "INSERT INTO pokemon_type (pokemon_id, slot, type_id, valid_from_regulation_id)
                 VALUES (?1,?2,?3,?4)
                 ON CONFLICT(pokemon_id, slot, valid_from_regulation_id) DO UPDATE SET
                   type_id = excluded.type_id, valid_to_regulation_id = NULL",
                params![pokemon_id, slot, tid, reg.id],
            )?;
            rep.types_opened += 1;
        }
    }
    Ok(())
}

fn sync_abilities(
    tx: &Transaction,
    r: &mut Resolver,
    pokemon_id: i64,
    ab: &Abilities,
    reg: &Regulation,
    rep: &mut IngestReport,
) -> Result<()> {
    let mut want: Vec<(String, i64)> = Vec::new();
    for (slot, name) in ab.occupied() {
        want.push((slot.to_string(), r.ability_id(tx, name, None)?));
    }

    let mut stmt = tx.prepare(
        "SELECT slot, ability_id FROM pokemon_ability
         WHERE pokemon_id = ?1 AND valid_to_regulation_id IS NULL",
    )?;
    let have: Vec<(String, i64)> = stmt
        .query_map([pokemon_id], |row| Ok((row.get(0)?, row.get(1)?)))?
        .collect::<rusqlite::Result<_>>()?;
    drop(stmt);

    for (slot, aid) in &have {
        if !want.contains(&(slot.clone(), *aid)) {
            rep.abilities_closed += close_open(
                tx, "pokemon_ability", "pokemon_id = ?2 AND slot = ?3",
                &[&pokemon_id, slot], reg.id,
            )?;
        }
    }
    for (slot, aid) in &want {
        if !have.contains(&(slot.clone(), *aid)) {
            tx.execute(
                "INSERT INTO pokemon_ability (pokemon_id, slot, ability_id, valid_from_regulation_id)
                 VALUES (?1,?2,?3,?4)
                 ON CONFLICT(pokemon_id, slot, valid_from_regulation_id) DO UPDATE SET
                   ability_id = excluded.ability_id, valid_to_regulation_id = NULL",
                params![pokemon_id, slot, aid, reg.id],
            )?;
            rep.abilities_opened += 1;
        }
    }
    Ok(())
}

fn sync_learnset(
    tx: &Transaction,
    r: &mut Resolver,
    pokemon_id: i64,
    entries: &[LearnsetEntry],
    reg: &Regulation,
    rep: &mut IngestReport,
) -> Result<()> {
    let mut want: HashSet<(i64, String, i64)> = HashSet::new();
    for e in entries {
        let mid = r.move_id(tx, &e.move_name)?;
        let level = if e.method == LearnMethod::LevelUp { e.level.unwrap_or(0) } else { 0 };
        want.insert((mid, e.method.as_str().to_string(), level));
    }

    let mut stmt = tx.prepare(
        "SELECT move_id, learn_method, level FROM pokemon_move
         WHERE pokemon_id = ?1 AND valid_to_regulation_id IS NULL",
    )?;
    let have: HashSet<(i64, String, i64)> = stmt
        .query_map([pokemon_id], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))?
        .collect::<rusqlite::Result<_>>()?;
    drop(stmt);

    // Only entries that actually disappeared are touched — a one-move change must not
    // rewrite the rest of the learnset.
    for (mid, method, level) in have.difference(&want) {
        rep.learnset_closed += close_open(
            tx, "pokemon_move",
            "pokemon_id = ?2 AND move_id = ?3 AND learn_method = ?4 AND level = ?5",
            &[&pokemon_id, mid, method, level], reg.id,
        )?;
    }
    for (mid, method, level) in want.difference(&have) {
        tx.execute(
            "INSERT INTO pokemon_move
               (pokemon_id, move_id, learn_method, level, valid_from_regulation_id)
             VALUES (?1,?2,?3,?4,?5)
             ON CONFLICT(pokemon_id, move_id, learn_method, level, valid_from_regulation_id)
             DO UPDATE SET valid_to_regulation_id = NULL",
            params![pokemon_id, mid, method, level, reg.id],
        )?;
        rep.learnset_opened += 1;
    }
    Ok(())
}
