#!/usr/bin/env bash
# Load pokedex/data/<table>.csv (written by export-reference.sh) into a freshly migrated,
# empty database, in one transaction.
#
#   pokedex migrate          # with DATABASE_URL set; creates the schema and its seeds
#   scripts/load-reference.sh postgres://pokedex:pokedex@127.0.0.1:55432/pokedex [dataDir]
#
# The CSVs carry natural keys, so every foreign key is looked up again here. Surrogate ids
# come out fresh, except regulation ids, which follow effective_from order (1 = M-A, ...)
# and so match any database built oldest first -- battle backups rely on that, since
# battle.regulation_id stores the raw id.
#
# The migration-seeded tables (type, type_chart_rule, nature, variant_kind) are not loaded
# but compared: a mismatch means the CSVs came from a different schema version.
set -euo pipefail

url=${1:?usage: load-reference.sh <postgres-url> [dataDir]}
repo=$(cd "$(dirname "$0")/.." && pwd)
data=$(cd "${2:-$repo/pokedex/data}" && pwd)

tables=(regulation type type_chart_rule nature variant_kind pokemon variant ability move
        item pokemon_stats move_data pokemon_type pokemon_ability pokemon_move item_legality)

{
  echo 'BEGIN;'
  cat <<'SQL'
DO $$ BEGIN
  IF EXISTS (SELECT 1 FROM regulation) OR EXISTS (SELECT 1 FROM pokemon) THEN
    RAISE EXCEPTION 'refusing: database already has pokedex data; load needs an empty one';
  END IF;
END $$;
SQL
  # Stage every CSV as all-text columns named by its header.
  for t in "${tables[@]}"; do
    cols=$(head -n1 "$data/$t.csv" | sed 's/,/ text, /g')
    echo "CREATE TEMP TABLE s_$t ($cols text);"
    echo "\\copy s_$t FROM '$t.csv' WITH (FORMAT csv, HEADER)"
  done
  cat <<'SQL'

-- Seeded tables must match exactly.
DO $$ BEGIN
  IF EXISTS ((SELECT id::bigint, name, display_name FROM s_type
              EXCEPT SELECT id, name, display_name FROM type)
             UNION ALL (SELECT id, name, display_name FROM type
              EXCEPT SELECT id::bigint, name, display_name FROM s_type))
  OR EXISTS ((SELECT id::bigint, name, increased_stat, decreased_stat FROM s_nature
              EXCEPT SELECT id, name, increased_stat, decreased_stat FROM nature)
             UNION ALL (SELECT id, name, increased_stat, decreased_stat FROM nature
              EXCEPT SELECT id::bigint, name, increased_stat, decreased_stat FROM s_nature))
  OR EXISTS ((SELECT id::bigint, name, description FROM s_variant_kind
              EXCEPT SELECT id, name, description FROM variant_kind)
             UNION ALL (SELECT id, name, description FROM variant_kind
              EXCEPT SELECT id::bigint, name, description FROM s_variant_kind))
  OR EXISTS ((SELECT attacking, defending, multiplier_pct::bigint FROM s_type_chart_rule
              EXCEPT SELECT a.name, d.name, r.multiplier_pct FROM type_chart_rule r
                     JOIN type a ON a.id = r.attacking_type_id JOIN type d ON d.id = r.defending_type_id)
             UNION ALL (SELECT a.name, d.name, r.multiplier_pct FROM type_chart_rule r
                     JOIN type a ON a.id = r.attacking_type_id JOIN type d ON d.id = r.defending_type_id
              EXCEPT SELECT attacking, defending, multiplier_pct::bigint FROM s_type_chart_rule))
  THEN
    RAISE EXCEPTION 'seeded tables differ from the CSVs; were they exported from another schema version?';
  END IF;
END $$;

INSERT INTO regulation (name, effective_from, notes)
SELECT name, effective_from::date, notes FROM s_regulation ORDER BY effective_from::date;

INSERT INTO pokemon (national_dex_no, form_slug, name, genus, height_dm, weight_hg)
SELECT national_dex_no::bigint, form_slug, name, genus, height_dm::bigint, weight_hg::bigint
FROM s_pokemon ORDER BY national_dex_no::bigint, form_slug COLLATE "C";

INSERT INTO variant (pokemon_id, base_pokemon_id, variant_kind_id, required_item)
SELECT p.id, b.id, k.id, s.required_item
FROM s_variant s
JOIN pokemon p ON p.national_dex_no = s.national_dex_no::bigint AND p.form_slug = s.form_slug
JOIN pokemon b ON b.national_dex_no = s.base_national_dex_no::bigint AND b.form_slug = s.base_form_slug
JOIN variant_kind k ON k.name = s.variant_kind;

INSERT INTO ability (name, display_name, description)
SELECT name, display_name, description FROM s_ability ORDER BY name COLLATE "C";

INSERT INTO move (name, display_name)
SELECT name, display_name FROM s_move ORDER BY name COLLATE "C";

INSERT INTO item (name, display_name, category, fling_power, description)
SELECT name, display_name, category, fling_power::bigint, description
FROM s_item ORDER BY name COLLATE "C";

INSERT INTO pokemon_stats (pokemon_id, regulation_id, base_hp, base_attack, base_defense,
                           base_sp_attack, base_sp_defense, base_speed)
SELECT p.id, r.id, s.base_hp::bigint, s.base_attack::bigint, s.base_defense::bigint,
       s.base_sp_attack::bigint, s.base_sp_defense::bigint, s.base_speed::bigint
FROM s_pokemon_stats s
JOIN pokemon p ON p.national_dex_no = s.national_dex_no::bigint AND p.form_slug = s.form_slug
JOIN regulation r ON r.name = s.regulation;

INSERT INTO move_data (move_id, regulation_id, type_id, damage_class, power, accuracy, pp,
                       priority, secondary_effect, effect_chance)
SELECT m.id, r.id, t.id, s.damage_class, s.power::bigint, s.accuracy::bigint, s.pp::bigint,
       s.priority::bigint, s.secondary_effect, s.effect_chance::bigint
FROM s_move_data s
JOIN move m ON m.name = s.move
JOIN regulation r ON r.name = s.regulation
JOIN type t ON t.name = s.type;

INSERT INTO pokemon_type (pokemon_id, slot, type_id, valid_from_regulation_id, valid_to_regulation_id)
SELECT p.id, s.slot::bigint, t.id, f.id, u.id
FROM s_pokemon_type s
JOIN pokemon p ON p.national_dex_no = s.national_dex_no::bigint AND p.form_slug = s.form_slug
JOIN type t ON t.name = s.type
JOIN regulation f ON f.name = s.valid_from
LEFT JOIN regulation u ON u.name = s.valid_to;

INSERT INTO pokemon_ability (pokemon_id, slot, ability_id, valid_from_regulation_id, valid_to_regulation_id)
SELECT p.id, s.slot, a.id, f.id, u.id
FROM s_pokemon_ability s
JOIN pokemon p ON p.national_dex_no = s.national_dex_no::bigint AND p.form_slug = s.form_slug
JOIN ability a ON a.name = s.ability
JOIN regulation f ON f.name = s.valid_from
LEFT JOIN regulation u ON u.name = s.valid_to;

INSERT INTO pokemon_move (pokemon_id, move_id, learn_method, level, valid_from_regulation_id, valid_to_regulation_id)
SELECT p.id, m.id, s.learn_method, s.level::bigint, f.id, u.id
FROM s_pokemon_move s
JOIN pokemon p ON p.national_dex_no = s.national_dex_no::bigint AND p.form_slug = s.form_slug
JOIN move m ON m.name = s.move
JOIN regulation f ON f.name = s.valid_from
LEFT JOIN regulation u ON u.name = s.valid_to;

INSERT INTO item_legality (item_id, valid_from_regulation_id, valid_to_regulation_id)
SELECT i.id, f.id, u.id
FROM s_item_legality s
JOIN item i ON i.name = s.item
JOIN regulation f ON f.name = s.valid_from
LEFT JOIN regulation u ON u.name = s.valid_to;

-- An inner join that silently dropped a row would otherwise go unnoticed.
DO $$
DECLARE t text; staged bigint; loaded bigint;
BEGIN
  FOREACH t IN ARRAY ARRAY['regulation','pokemon','variant','ability','move','item',
                           'pokemon_stats','move_data','pokemon_type','pokemon_ability',
                           'pokemon_move','item_legality'] LOOP
    EXECUTE format('SELECT count(*) FROM s_%I', t) INTO staged;
    EXECUTE format('SELECT count(*) FROM %I', t) INTO loaded;
    IF staged <> loaded THEN
      RAISE EXCEPTION '%: % rows in the CSV but % loaded', t, staged, loaded;
    END IF;
  END LOOP;
END $$;

COMMIT;
SQL
} | docker run --rm -i --network host --user "$(id -u):$(id -g)" -e HOME=/tmp \
      -v "$data:/data:ro" -w /data \
      postgres:17-alpine psql "$url" -X -q -v ON_ERROR_STOP=1

echo "loaded reference data from $data"
