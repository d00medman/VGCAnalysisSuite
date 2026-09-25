//! Read-only pokedex endpoints, for browsing and auditing the reference data.
//!
//! Each query builds its JSON response in SQL, so this module only passes documents
//! through. Every endpoint takes `?regulation=<id>` and defaults to the current (latest)
//! regulation; answers come from the `*_effective` views, i.e. exactly what the temporal
//! model resolves for that regulation.

use crate::{ApiError, ApiResult, Shared};
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::routing::get;
use axum::{Json, Router};
use serde::Deserialize;
use serde_json::Value;

pub fn routes() -> Router<Shared> {
    Router::new()
        .route("/api/pokedex/regulations", get(regulations))
        .route("/api/pokedex/pokemon", get(pokemon_list))
        .route("/api/pokedex/pokemon/:id", get(pokemon_detail))
        .route("/api/pokedex/moves", get(moves))
        .route("/api/pokedex/items", get(items))
}

#[derive(Deserialize)]
struct Reg {
    regulation: Option<i64>,
}

/// `$1` resolved to a regulation id: the one asked for, else the latest.
const REG: &str = "(SELECT coalesce($1::bigint,
                     (SELECT id FROM regulation ORDER BY effective_from DESC LIMIT 1)))";

async fn doc(s: &Shared, sql: &str, reg: Option<i64>) -> ApiResult<Json<Value>> {
    let sql = sql.replace("$REG", REG);
    Ok(Json(s.store.query_json(&sql, &[&reg]).await?.unwrap_or(Value::Null)))
}

async fn regulations(State(s): State<Shared>) -> ApiResult<Json<Value>> {
    let sql = "
        SELECT coalesce(json_agg(json_build_object(
                 'id', id, 'name', name,
                 'effective_from', effective_from::text,
                 'effective_to', effective_to::text) ORDER BY effective_from), '[]')
        FROM regulation_window";
    Ok(Json(s.store.query_json(sql, &[]).await?.unwrap_or(Value::Null)))
}

async fn pokemon_list(State(s): State<Shared>, Query(q): Query<Reg>) -> ApiResult<Json<Value>> {
    let sql = "
        WITH r AS (SELECT $REG AS id)
        SELECT coalesce(json_agg(x ORDER BY x.dex, x.form <> '', x.form), '[]') FROM (
          SELECT p.id, p.national_dex_no AS dex, p.form_slug AS form, p.name,
                 vk.name AS variant_kind,
                 s.base_hp AS hp, s.base_attack AS atk, s.base_defense AS def,
                 s.base_sp_attack AS spa, s.base_sp_defense AS spd, s.base_speed AS spe,
                 s.base_stat_total AS bst, s.sourced_from_regulation AS stats_from,
                 (SELECT json_agg(t.name ORDER BY e.slot)
                    FROM pokemon_type_effective e JOIN type t ON t.id = e.type_id
                   WHERE e.pokemon_id = p.id AND e.regulation_id = r.id) AS types,
                 (SELECT json_agg(a.name ORDER BY array_position(
                           ARRAY['primary','secondary','hidden'], e.slot))
                    FROM pokemon_ability_effective e JOIN ability a ON a.id = e.ability_id
                   WHERE e.pokemon_id = p.id AND e.regulation_id = r.id) AS abilities,
                 (SELECT count(*) FROM pokemon_move_effective m
                   WHERE m.pokemon_id = p.id AND m.regulation_id = r.id) AS moves
          FROM r
          JOIN pokemon_stats_effective s ON s.regulation_id = r.id
          JOIN pokemon p ON p.id = s.pokemon_id
          LEFT JOIN variant v ON v.pokemon_id = p.id
          LEFT JOIN variant_kind vk ON vk.id = v.variant_kind_id
        ) x";
    doc(&s, sql, q.regulation).await
}

async fn pokemon_detail(
    State(s): State<Shared>,
    Path(id): Path<i64>,
    Query(q): Query<Reg>,
) -> ApiResult<Json<Value>> {
    // $1 is the regulation, $2 the pokemon.
    let sql = "
        WITH r AS (SELECT $REG AS id)
        SELECT json_build_object(
          'id', p.id, 'dex', p.national_dex_no, 'form', p.form_slug, 'name', p.name,
          'height_dm', p.height_dm, 'weight_hg', p.weight_hg,
          'variant', (SELECT json_build_object(
                         'kind', vk.name, 'required_item', v.required_item,
                         'base', json_build_object('id', b.id, 'name', b.name))
                        FROM variant v
                        JOIN variant_kind vk ON vk.id = v.variant_kind_id
                        JOIN pokemon b ON b.id = v.base_pokemon_id
                       WHERE v.pokemon_id = p.id),
          'variants', (SELECT json_agg(json_build_object(
                          'id', c.id, 'name', c.name, 'kind', vk.name,
                          'required_item', v.required_item) ORDER BY c.form_slug)
                         FROM variant v
                         JOIN pokemon c ON c.id = v.pokemon_id
                         JOIN variant_kind vk ON vk.id = v.variant_kind_id
                        WHERE v.base_pokemon_id = p.id),
          'stats', (SELECT json_build_object(
                       'hp', s.base_hp, 'atk', s.base_attack, 'def', s.base_defense,
                       'spa', s.base_sp_attack, 'spd', s.base_sp_defense, 'spe', s.base_speed,
                       'bst', s.base_stat_total, 'from', s.sourced_from_regulation)
                      FROM pokemon_stats_effective s
                     WHERE s.pokemon_id = p.id AND s.regulation_id = r.id),
          'stat_rows', (SELECT json_agg(json_build_object(
                           'regulation', rg.name,
                           'hp', s.base_hp, 'atk', s.base_attack, 'def', s.base_defense,
                           'spa', s.base_sp_attack, 'spd', s.base_sp_defense,
                           'spe', s.base_speed, 'bst', s.base_stat_total)
                           ORDER BY rg.effective_from)
                          FROM pokemon_stats s JOIN regulation rg ON rg.id = s.regulation_id
                         WHERE s.pokemon_id = p.id),
          'types', (SELECT json_agg(t.name ORDER BY e.slot)
                      FROM pokemon_type_effective e JOIN type t ON t.id = e.type_id
                     WHERE e.pokemon_id = p.id AND e.regulation_id = r.id),
          'abilities', (SELECT json_agg(json_build_object(
                           'slot', e.slot, 'name', a.name, 'description', a.description)
                           ORDER BY array_position(ARRAY['primary','secondary','hidden'], e.slot))
                          FROM pokemon_ability_effective e JOIN ability a ON a.id = e.ability_id
                         WHERE e.pokemon_id = p.id AND e.regulation_id = r.id),
          'defense', (SELECT json_agg(json_build_object('type', t.name, 'pct', d.multiplier_pct)
                         ORDER BY t.id)
                        FROM pokemon_defense_effectiveness d JOIN type t ON t.id = d.attacking_type_id
                       WHERE d.pokemon_id = p.id AND d.regulation_id = r.id),
          'learnset', (SELECT json_agg(json_build_object(
                          'name', m.name, 'method', e.learn_method, 'level', e.level,
                          'type', t.name, 'class', md.damage_class, 'power', md.power,
                          'accuracy', md.accuracy, 'pp', md.pp, 'priority', md.priority)
                          ORDER BY m.name)
                         FROM pokemon_move_effective e
                         JOIN move m ON m.id = e.move_id
                         LEFT JOIN move_data_effective md
                           ON md.move_id = m.id AND md.regulation_id = r.id
                         LEFT JOIN type t ON t.id = md.type_id
                        WHERE e.pokemon_id = p.id AND e.regulation_id = r.id)
        )
        FROM pokemon p, r
        WHERE p.id = $2";
    let sql = sql.replace("$REG", REG);
    s.store
        .query_json(&sql, &[&q.regulation, &id])
        .await?
        .map(Json)
        .ok_or_else(|| ApiError(StatusCode::NOT_FOUND, format!("no pokemon {id}")))
}

async fn moves(State(s): State<Shared>, Query(q): Query<Reg>) -> ApiResult<Json<Value>> {
    let sql = "
        WITH r AS (SELECT $REG AS id)
        SELECT coalesce(json_agg(x ORDER BY x.name), '[]') FROM (
          SELECT m.id, m.name, t.name AS type, d.damage_class AS class, d.power, d.accuracy,
                 d.pp, d.priority, d.effect_chance, d.secondary_effect,
                 d.sourced_from_regulation AS data_from,
                 (SELECT count(DISTINCT e.pokemon_id) FROM pokemon_move_effective e
                   WHERE e.move_id = m.id AND e.regulation_id = r.id) AS learners
          FROM r
          JOIN move_data_effective d ON d.regulation_id = r.id
          JOIN move m ON m.id = d.move_id
          JOIN type t ON t.id = d.type_id
        ) x";
    doc(&s, sql, q.regulation).await
}

async fn items(State(s): State<Shared>, Query(q): Query<Reg>) -> ApiResult<Json<Value>> {
    // Every item, flagged by legality, so a wrongly-banned item is visible too.
    let sql = "
        WITH r AS (SELECT $REG AS id)
        SELECT coalesce(json_agg(x ORDER BY x.display_name), '[]') FROM (
          SELECT i.id, i.name, i.display_name, i.category, i.fling_power, i.description,
                 EXISTS (SELECT 1 FROM item_legal_effective e
                          WHERE e.item_id = i.id AND e.regulation_id = r.id) AS legal
          FROM item i, r
        ) x";
    doc(&s, sql, q.regulation).await
}
