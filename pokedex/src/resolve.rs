//! Name -> id resolution.
//!
//! Records reference types, moves, abilities and variant kinds by name; a pipeline
//! stage must never need to know surrogate keys. Lookups are cached per ingest run
//! because a 1000-pokemon snapshot resolves the same 18 type names thousands of times.

use crate::error::{Error, Result};
use postgres::GenericClient;
use std::collections::HashMap;

#[derive(Default)]
pub struct Resolver {
    types: HashMap<String, i64>,
    abilities: HashMap<String, i64>,
    moves: HashMap<String, i64>,
    variant_kinds: HashMap<String, i64>,
    pokemon: HashMap<(i64, String), i64>,
}

impl Resolver {
    pub fn new() -> Self {
        Self::default()
    }

    /// Fixed reference data: unknown names are an error, never an insert.
    pub fn type_id(&mut self, c: &mut impl GenericClient, name: &str) -> Result<i64> {
        Self::lookup(&mut self.types, c, "type", "name", name, "type")
    }

    pub fn variant_kind_id(&mut self, c: &mut impl GenericClient, name: &str) -> Result<i64> {
        Self::lookup(
            &mut self.variant_kinds,
            c,
            "variant_kind",
            "name",
            name,
            "variant kind",
        )
    }

    fn lookup(
        cache: &mut HashMap<String, i64>,
        c: &mut impl GenericClient,
        table: &str,
        col: &str,
        name: &str,
        kind: &'static str,
    ) -> Result<i64> {
        if let Some(id) = cache.get(name) {
            return Ok(*id);
        }
        // `table`/`col` are compile-time literals from this module, never input.
        let sql = format!("SELECT id FROM {table} WHERE {col} = $1");
        let id: Option<i64> = c.query_opt(&sql, &[&name])?.map(|r| r.get(0));
        let id = id.ok_or_else(|| Error::UnknownName { kind, name: name.to_string() })?;
        cache.insert(name.to_string(), id);
        Ok(id)
    }

    /// Ability rows are created on demand: abilities arrive as part of a snapshot
    /// rather than being seeded reference data.
    pub fn ability_id(
        &mut self,
        c: &mut impl GenericClient,
        name: &str,
        description: Option<&str>,
    ) -> Result<i64> {
        if let Some(id) = self.abilities.get(name) {
            return Ok(*id);
        }
        // Only overwrite a stored description when a new one is actually supplied,
        // so a bare reference from a learnset cannot blank out real prose.
        let id: i64 = c
            .query_one(
                "INSERT INTO ability (name, description) VALUES ($1, $2)
                 ON CONFLICT (name) DO UPDATE SET
                   description = COALESCE(excluded.description, ability.description)
                 RETURNING id",
                &[&name, &description],
            )?
            .get(0);
        self.abilities.insert(name.to_string(), id);
        Ok(id)
    }

    pub fn move_id(&mut self, c: &mut impl GenericClient, name: &str) -> Result<i64> {
        if let Some(id) = self.moves.get(name) {
            return Ok(*id);
        }
        c.execute(
            "INSERT INTO move (name) VALUES ($1) ON CONFLICT (name) DO NOTHING",
            &[&name],
        )?;
        let id: i64 = c.query_one("SELECT id FROM move WHERE name = $1", &[&name])?.get(0);
        self.moves.insert(name.to_string(), id);
        Ok(id)
    }

    /// Look up an existing pokemon by its natural key. Does not create.
    pub fn pokemon_id(
        &mut self,
        c: &mut impl GenericClient,
        national_dex_no: i64,
        form_slug: &str,
    ) -> Result<Option<i64>> {
        let key = (national_dex_no, form_slug.to_string());
        if let Some(id) = self.pokemon.get(&key) {
            return Ok(Some(*id));
        }
        let id: Option<i64> = c
            .query_opt(
                "SELECT id FROM pokemon WHERE national_dex_no = $1 AND form_slug = $2",
                &[&national_dex_no, &form_slug],
            )?
            .map(|r| r.get(0));
        if let Some(id) = id {
            self.pokemon.insert(key, id);
        }
        Ok(id)
    }

    pub fn cache_pokemon(&mut self, national_dex_no: i64, form_slug: &str, id: i64) {
        self.pokemon.insert((national_dex_no, form_slug.to_string()), id);
    }
}
