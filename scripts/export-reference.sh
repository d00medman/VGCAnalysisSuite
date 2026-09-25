#!/usr/bin/env bash
# Write every pokedex reference table to pokedex/data/<table>.csv, sorted by its key.
#
#   scripts/export-reference.sh postgres://pokedex:pokedex@127.0.0.1:5432/pokedex [outDir]
#
# Rows are written with natural keys (national dex number + form, move name, regulation
# name), never surrogate ids, so the git diff between two exports reads as game changes:
# "Garchomp learns Slash from Regulation M-B", not "row 88213 changed". Child tables add
# a `pokemon_name` column for the same reason; scripts/load-reference.sh ignores it.
#
# psql runs from postgres:17-alpine because host clients may be older than the server.
set -euo pipefail

url=${1:?usage: export-reference.sh <postgres-url> [outDir]}
repo=$(cd "$(dirname "$0")/.." && pwd)
out=$(mkdir -p "${2:-$repo/pokedex/data}" && cd "${2:-$repo/pokedex/data}" && pwd)

psql() {
  docker run --rm -i --network host --user "$(id -u):$(id -g)" -e HOME=/tmp \
    postgres:17-alpine psql "$url" -X -q -v ON_ERROR_STOP=1 "$@"
}

# Byte order, not locale order, so exports from different machines diff cleanly.
C='COLLATE "C"'

declare -A query=(
  [regulation]="SELECT name, effective_from, notes FROM regulation ORDER BY effective_from"

  # Seeded by migrations, with fixed ids that are themselves the key.
  [type]="SELECT id, name, display_name FROM type ORDER BY id"
  [nature]="SELECT id, name, increased_stat, decreased_stat FROM nature ORDER BY id"
  [variant_kind]="SELECT id, name, description FROM variant_kind ORDER BY id"
  [type_chart_rule]="
    SELECT a.name AS attacking, d.name AS defending, r.multiplier_pct
    FROM type_chart_rule r
    JOIN type a ON a.id = r.attacking_type_id
    JOIN type d ON d.id = r.defending_type_id
    ORDER BY a.id, d.id"

  [pokemon]="
    SELECT national_dex_no, form_slug, name, genus, height_dm, weight_hg
    FROM pokemon ORDER BY national_dex_no, form_slug $C"
  [variant]="
    SELECT p.national_dex_no, p.form_slug, p.name AS pokemon_name,
           b.national_dex_no AS base_national_dex_no, b.form_slug AS base_form_slug,
           k.name AS variant_kind, v.required_item
    FROM variant v
    JOIN pokemon p ON p.id = v.pokemon_id
    JOIN pokemon b ON b.id = v.base_pokemon_id
    JOIN variant_kind k ON k.id = v.variant_kind_id
    ORDER BY p.national_dex_no, p.form_slug $C"
  [ability]="SELECT name, display_name, description FROM ability ORDER BY name $C"
  [move]="SELECT name, display_name FROM move ORDER BY name $C"
  [item]="
    SELECT name, display_name, category, fling_power, description
    FROM item ORDER BY name $C"

  [pokemon_stats]="
    SELECT p.national_dex_no, p.form_slug, p.name AS pokemon_name, r.name AS regulation,
           s.base_hp, s.base_attack, s.base_defense, s.base_sp_attack, s.base_sp_defense,
           s.base_speed
    FROM pokemon_stats s
    JOIN pokemon p ON p.id = s.pokemon_id
    JOIN regulation r ON r.id = s.regulation_id
    ORDER BY p.national_dex_no, p.form_slug $C, r.effective_from"
  [move_data]="
    SELECT m.name AS move, r.name AS regulation, t.name AS type, d.damage_class, d.power,
           d.accuracy, d.pp, d.priority, d.secondary_effect, d.effect_chance
    FROM move_data d
    JOIN move m ON m.id = d.move_id
    JOIN regulation r ON r.id = d.regulation_id
    JOIN type t ON t.id = d.type_id
    ORDER BY m.name $C, r.effective_from"
  [pokemon_type]="
    SELECT p.national_dex_no, p.form_slug, p.name AS pokemon_name, x.slot, t.name AS type,
           f.name AS valid_from, u.name AS valid_to
    FROM pokemon_type x
    JOIN pokemon p ON p.id = x.pokemon_id
    JOIN type t ON t.id = x.type_id
    JOIN regulation f ON f.id = x.valid_from_regulation_id
    LEFT JOIN regulation u ON u.id = x.valid_to_regulation_id
    ORDER BY p.national_dex_no, p.form_slug $C, x.slot, f.effective_from"
  [pokemon_ability]="
    SELECT p.national_dex_no, p.form_slug, p.name AS pokemon_name, x.slot, a.name AS ability,
           f.name AS valid_from, u.name AS valid_to
    FROM pokemon_ability x
    JOIN pokemon p ON p.id = x.pokemon_id
    JOIN ability a ON a.id = x.ability_id
    JOIN regulation f ON f.id = x.valid_from_regulation_id
    LEFT JOIN regulation u ON u.id = x.valid_to_regulation_id
    ORDER BY p.national_dex_no, p.form_slug $C, x.slot $C, f.effective_from"
  [pokemon_move]="
    SELECT p.national_dex_no, p.form_slug, p.name AS pokemon_name, m.name AS move,
           x.learn_method, x.level, f.name AS valid_from, u.name AS valid_to
    FROM pokemon_move x
    JOIN pokemon p ON p.id = x.pokemon_id
    JOIN move m ON m.id = x.move_id
    JOIN regulation f ON f.id = x.valid_from_regulation_id
    LEFT JOIN regulation u ON u.id = x.valid_to_regulation_id
    ORDER BY p.national_dex_no, p.form_slug $C, m.name $C, x.learn_method $C,
             x.level NULLS FIRST, f.effective_from"
  [item_legality]="
    SELECT i.name AS item, f.name AS valid_from, u.name AS valid_to
    FROM item_legality x
    JOIN item i ON i.id = x.item_id
    JOIN regulation f ON f.id = x.valid_from_regulation_id
    LEFT JOIN regulation u ON u.id = x.valid_to_regulation_id
    ORDER BY i.name $C, f.effective_from"
)

for table in $(printf '%s\n' "${!query[@]}" | sort); do
  # Unquoted empty = NULL, "" = empty string: form_slug '' survives the round trip.
  psql -c "COPY (${query[$table]}) TO STDOUT WITH (FORMAT csv, HEADER)" >"$out/$table.csv.tmp"
  mv "$out/$table.csv.tmp" "$out/$table.csv"
  printf '%-16s %6d rows\n' "$table" $(($(wc -l <"$out/$table.csv") - 1))
done
