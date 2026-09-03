-- 0001_init.sql — pokedex core schema
--
-- Applied by src/migrate.rs under PRAGMA user_version, inside a transaction.
-- Do not edit after release; add a new numbered file instead.
--
-- NOTE: `PRAGMA foreign_keys = ON` is deliberately NOT here. It is a no-op inside a
-- transaction, so it must be set per-connection in Db::open instead.

-- ============================================================ static reference

CREATE TABLE type (
  id           INTEGER PRIMARY KEY,
  name         TEXT NOT NULL UNIQUE,   -- 'fire'
  display_name TEXT NOT NULL           -- 'Fire'
);

-- Dense: all 18x18 = 324 pairs, including neutral. See devlog decision 1 —
-- a plain JOIN everywhere beats LEFT JOIN + COALESCE(...,100), where a forgotten
-- COALESCE silently drops neutral matchups instead of erroring.
-- Modern (Gen 6+) chart only, not scoped by generation or regulation. Decision 2.
CREATE TABLE type_chart_rule (
  attacking_type_id INTEGER NOT NULL REFERENCES type(id) ON DELETE CASCADE,
  defending_type_id INTEGER NOT NULL REFERENCES type(id) ON DELETE CASCADE,
  multiplier_pct    INTEGER NOT NULL CHECK (multiplier_pct IN (0,50,100,200)),
  PRIMARY KEY (attacking_type_id, defending_type_id)
) WITHOUT ROWID;

-- Per-individual trait, not a species trait: nothing in this schema joins to it.
-- Correct and unused until an individuals/caught-pokemon table exists.
CREATE TABLE nature (
  id             INTEGER PRIMARY KEY,
  name           TEXT NOT NULL UNIQUE,
  increased_stat TEXT CHECK (increased_stat IN ('attack','defense','sp_attack','sp_defense','speed')),
  decreased_stat TEXT CHECK (decreased_stat IN ('attack','defense','sp_attack','sp_defense','speed')),
  CHECK ((increased_stat IS NULL) = (decreased_stat IS NULL))   -- the 5 neutral natures
);

-- Named variant_kind, NOT variant_type: `type` already means elemental type here,
-- and variant.type_id beside pokemon_type.type_id is a live footgun in queries.
CREATE TABLE variant_kind (
  id          INTEGER PRIMARY KEY,
  name        TEXT NOT NULL UNIQUE,
  description TEXT
);

CREATE TABLE ability (
  id          INTEGER PRIMARY KEY,
  name        TEXT NOT NULL UNIQUE,
  description TEXT            -- deliberately not regulation-scoped
);

-- ============================================================ temporal spine

-- NOT seeded: names and dates are research output. `pokedex regulation add` is
-- required plumbing, not a convenience — the DB is unusable with zero regulations.
CREATE TABLE regulation (
  id             INTEGER PRIMARY KEY,
  name           TEXT NOT NULL UNIQUE,               -- 'Regulation G'
  effective_from TEXT NOT NULL UNIQUE                -- ISO-8601 'YYYY-MM-DD'
                 CHECK (effective_from LIKE '____-__-__'),
  notes          TEXT
);

-- effective_to is DERIVED, never stored. Storing both ends forces every insert to
-- patch its predecessor; one missed patch silently creates an overlap or a gap.
-- LEAD() makes those unrepresentable, and lets a regulation be inserted retroactively
-- between two existing ones with the chain re-linking itself — no data migration.
-- UNIQUE(effective_from) is what makes the ordering total. Do not drop it.
-- ISO-8601 TEXT sorts lexicographically = chronologically; no date functions anywhere.
CREATE VIEW regulation_window AS
SELECT id,
       name,
       effective_from,
       LEAD(effective_from) OVER w AS effective_to,          -- NULL = current
       LEAD(id)             OVER w AS next_regulation_id,
       LAG(id)              OVER w AS prev_regulation_id
FROM regulation
WINDOW w AS (ORDER BY effective_from);

-- ============================================================ entities

CREATE TABLE pokemon (
  id              INTEGER PRIMARY KEY,
  national_dex_no INTEGER NOT NULL,
  -- '' = base form, else 'alolan', 'mega-x', ... In the natural key so variants do
  -- not collide on dex number. Empty string not NULL: SQLite UNIQUE treats NULLs as
  -- distinct, so NULL would permit base Raichu twice.
  form_slug       TEXT NOT NULL DEFAULT '',
  name            TEXT NOT NULL,
  genus           TEXT,
  height_dm       INTEGER,
  weight_hg       INTEGER,
  UNIQUE (national_dex_no, form_slug)
);

-- A variant is still its own `pokemon` row, so stats/types/abilities/learnsets all
-- work through the ordinary path with zero special-casing. Presence in this table is
-- what makes something a variant; absence means base form — the bad state is
-- unrepresentable rather than merely CHECKed.
-- Self-FK => base rows must be inserted before their variants (ingest two-pass).
CREATE TABLE variant (
  pokemon_id      INTEGER PRIMARY KEY REFERENCES pokemon(id) ON DELETE CASCADE,
  base_pokemon_id INTEGER NOT NULL    REFERENCES pokemon(id) ON DELETE RESTRICT,
  variant_kind_id INTEGER NOT NULL    REFERENCES variant_kind(id),
  required_item   TEXT,               -- 'Charizardite X'; NULL for regional
  CHECK (pokemon_id <> base_pokemon_id)
);

CREATE TABLE move (
  id   INTEGER PRIMARY KEY,
  name TEXT NOT NULL UNIQUE
);

-- ============================================================ scalar facts (sparse)
-- Rows exist ONLY where values change. Resolution = latest row at or before the
-- target regulation. Cannot express removal — that is what interval tables are for.

CREATE TABLE pokemon_stats (
  id              INTEGER PRIMARY KEY,
  pokemon_id      INTEGER NOT NULL REFERENCES pokemon(id) ON DELETE CASCADE,
  regulation_id   INTEGER NOT NULL REFERENCES regulation(id) ON DELETE RESTRICT,
  base_hp         INTEGER NOT NULL CHECK (base_hp         BETWEEN 1 AND 255),
  base_attack     INTEGER NOT NULL CHECK (base_attack     BETWEEN 1 AND 255),
  base_defense    INTEGER NOT NULL CHECK (base_defense    BETWEEN 1 AND 255),
  base_sp_attack  INTEGER NOT NULL CHECK (base_sp_attack  BETWEEN 1 AND 255),
  base_sp_defense INTEGER NOT NULL CHECK (base_sp_defense BETWEEN 1 AND 255),
  base_speed      INTEGER NOT NULL CHECK (base_speed      BETWEEN 1 AND 255),
  base_stat_total INTEGER GENERATED ALWAYS AS
      (base_hp + base_attack + base_defense + base_sp_attack + base_sp_defense + base_speed) STORED,
  UNIQUE (pokemon_id, regulation_id)
);

-- type_id lives on the versioned side on purpose: move types change
-- (Bite went Normal -> Dark). Only `name` is identity.
-- NULL accuracy = never misses (Swift, Aerial Ace). NULL power = status/variable.
-- Both distinct from 0.
CREATE TABLE move_data (
  id               INTEGER PRIMARY KEY,
  move_id          INTEGER NOT NULL REFERENCES move(id) ON DELETE CASCADE,
  regulation_id    INTEGER NOT NULL REFERENCES regulation(id) ON DELETE RESTRICT,
  type_id          INTEGER NOT NULL REFERENCES type(id),
  damage_class     TEXT NOT NULL CHECK (damage_class IN ('physical','special','status')),
  power            INTEGER CHECK (power         IS NULL OR power >= 0),
  accuracy         INTEGER CHECK (accuracy      IS NULL OR accuracy      BETWEEN 0 AND 100),
  pp               INTEGER CHECK (pp            IS NULL OR pp > 0),
  priority         INTEGER NOT NULL DEFAULT 0,
  secondary_effect TEXT,
  effect_chance    INTEGER CHECK (effect_chance IS NULL OR effect_chance BETWEEN 0 AND 100),
  UNIQUE (move_id, regulation_id)
);

-- ============================================================ set membership (intervals)
-- Half-open [valid_from, valid_to). valid_to NULL = still open.
-- valid_from is in the PK so successive windows coexist.
-- Invariant on all three: AT MOST ONE OPEN WINDOW PER LOGICAL KEY (triggers below).

CREATE TABLE pokemon_type (
  pokemon_id               INTEGER NOT NULL REFERENCES pokemon(id) ON DELETE CASCADE,
  slot                     INTEGER NOT NULL CHECK (slot IN (1,2)),
  type_id                  INTEGER NOT NULL REFERENCES type(id),
  valid_from_regulation_id INTEGER NOT NULL REFERENCES regulation(id),
  valid_to_regulation_id   INTEGER          REFERENCES regulation(id),
  PRIMARY KEY (pokemon_id, slot, valid_from_regulation_id)
) WITHOUT ROWID;

CREATE TABLE pokemon_ability (
  pokemon_id               INTEGER NOT NULL REFERENCES pokemon(id) ON DELETE CASCADE,
  slot                     TEXT NOT NULL CHECK (slot IN ('primary','secondary','hidden')),
  ability_id               INTEGER NOT NULL REFERENCES ability(id),
  valid_from_regulation_id INTEGER NOT NULL REFERENCES regulation(id),
  valid_to_regulation_id   INTEGER          REFERENCES regulation(id),
  PRIMARY KEY (pokemon_id, slot, valid_from_regulation_id)
) WITHOUT ROWID;

CREATE TABLE pokemon_move (
  pokemon_id               INTEGER NOT NULL REFERENCES pokemon(id) ON DELETE CASCADE,
  move_id                  INTEGER NOT NULL REFERENCES move(id) ON DELETE CASCADE,
  learn_method             TEXT NOT NULL CHECK (learn_method IN ('level-up','machine','egg','tutor','other')),
  level                    INTEGER NOT NULL DEFAULT 0,   -- 0 = n/a
  valid_from_regulation_id INTEGER NOT NULL REFERENCES regulation(id),
  valid_to_regulation_id   INTEGER          REFERENCES regulation(id),
  PRIMARY KEY (pokemon_id, move_id, learn_method, level, valid_from_regulation_id)
) WITHOUT ROWID;

-- ------------------------------------------------ one-open-window triggers
-- Two open rows for the same key is not a data-quality issue, it is BROKEN
-- RESOLUTION: the effectiveness view would return two slot-1 rows for one
-- regulation and silently duplicate results. Also enforces close-before-open
-- ordering in ingest as a side effect.
-- Known limit: only open rows are inspected, so two overlapping CLOSED windows are
-- not caught. Unreachable via normal ingest (you only ever close the open row and
-- open a new one at the current regulation).

CREATE TRIGGER pokemon_type_one_open_ins
BEFORE INSERT ON pokemon_type
WHEN NEW.valid_to_regulation_id IS NULL
 AND EXISTS (SELECT 1 FROM pokemon_type
             WHERE pokemon_id = NEW.pokemon_id
               AND slot       = NEW.slot
               AND valid_to_regulation_id IS NULL)
BEGIN SELECT RAISE(ABORT, 'pokemon_type: slot already has an open window'); END;

CREATE TRIGGER pokemon_type_one_open_upd
BEFORE UPDATE ON pokemon_type
WHEN NEW.valid_to_regulation_id IS NULL
 AND OLD.valid_to_regulation_id IS NOT NULL
 AND EXISTS (SELECT 1 FROM pokemon_type
             WHERE pokemon_id = NEW.pokemon_id
               AND slot       = NEW.slot
               AND valid_to_regulation_id IS NULL
               AND valid_from_regulation_id <> NEW.valid_from_regulation_id)
BEGIN SELECT RAISE(ABORT, 'pokemon_type: reopening would create a second open window'); END;

CREATE TRIGGER pokemon_ability_one_open_ins
BEFORE INSERT ON pokemon_ability
WHEN NEW.valid_to_regulation_id IS NULL
 AND EXISTS (SELECT 1 FROM pokemon_ability
             WHERE pokemon_id = NEW.pokemon_id
               AND slot       = NEW.slot
               AND valid_to_regulation_id IS NULL)
BEGIN SELECT RAISE(ABORT, 'pokemon_ability: slot already has an open window'); END;

CREATE TRIGGER pokemon_ability_one_open_upd
BEFORE UPDATE ON pokemon_ability
WHEN NEW.valid_to_regulation_id IS NULL
 AND OLD.valid_to_regulation_id IS NOT NULL
 AND EXISTS (SELECT 1 FROM pokemon_ability
             WHERE pokemon_id = NEW.pokemon_id
               AND slot       = NEW.slot
               AND valid_to_regulation_id IS NULL
               AND valid_from_regulation_id <> NEW.valid_from_regulation_id)
BEGIN SELECT RAISE(ABORT, 'pokemon_ability: reopening would create a second open window'); END;

CREATE TRIGGER pokemon_move_one_open_ins
BEFORE INSERT ON pokemon_move
WHEN NEW.valid_to_regulation_id IS NULL
 AND EXISTS (SELECT 1 FROM pokemon_move
             WHERE pokemon_id   = NEW.pokemon_id
               AND move_id      = NEW.move_id
               AND learn_method = NEW.learn_method
               AND level        = NEW.level
               AND valid_to_regulation_id IS NULL)
BEGIN SELECT RAISE(ABORT, 'pokemon_move: entry already has an open window'); END;

CREATE TRIGGER pokemon_move_one_open_upd
BEFORE UPDATE ON pokemon_move
WHEN NEW.valid_to_regulation_id IS NULL
 AND OLD.valid_to_regulation_id IS NOT NULL
 AND EXISTS (SELECT 1 FROM pokemon_move
             WHERE pokemon_id   = NEW.pokemon_id
               AND move_id      = NEW.move_id
               AND learn_method = NEW.learn_method
               AND level        = NEW.level
               AND valid_to_regulation_id IS NULL
               AND valid_from_regulation_id <> NEW.valid_from_regulation_id)
BEGIN SELECT RAISE(ABORT, 'pokemon_move: reopening would create a second open window'); END;

-- ============================================================ resolution views
-- PERFORMANCE: these CROSS JOIN against `regulation` and are meant to be queried
-- WITH A FILTER (regulation_id and/or pokemon_id). An unfiltered scan of
-- pokemon_stats_effective is O(pokemon x regulations) correlated subqueries.

-- Inner JOIN (not LEFT) is load-bearing: a pokemon with no stat row at or before R
-- does not appear in R at all, so species introduced later correctly vanish from
-- earlier regulations for free.
CREATE VIEW pokemon_stats_effective AS
SELECT p.id AS pokemon_id,
       r.id AS regulation_id,
       s.base_hp, s.base_attack, s.base_defense,
       s.base_sp_attack, s.base_sp_defense, s.base_speed, s.base_stat_total,
       sr.name AS sourced_from_regulation      -- "was this rebalanced, and when?"
FROM pokemon p
CROSS JOIN regulation r
JOIN pokemon_stats s ON s.id = (
    SELECT s2.id
    FROM pokemon_stats s2
    JOIN regulation r2 ON r2.id = s2.regulation_id
    WHERE s2.pokemon_id = p.id
      AND r2.effective_from <= r.effective_from
    ORDER BY r2.effective_from DESC
    LIMIT 1)
JOIN regulation sr ON sr.id = s.regulation_id;

CREATE VIEW move_data_effective AS
SELECT m.id AS move_id,
       r.id AS regulation_id,
       d.type_id, d.damage_class, d.power, d.accuracy, d.pp, d.priority,
       d.secondary_effect, d.effect_chance,
       sr.name AS sourced_from_regulation
FROM move m
CROSS JOIN regulation r
JOIN move_data d ON d.id = (
    SELECT d2.id
    FROM move_data d2
    JOIN regulation r2 ON r2.id = d2.regulation_id
    WHERE d2.move_id = m.id
      AND r2.effective_from <= r.effective_from
    ORDER BY r2.effective_from DESC
    LIMIT 1)
JOIN regulation sr ON sr.id = d.regulation_id;

-- Interval membership predicate, shared by the three views below:
--   rf.effective_from <= r.effective_from
--   AND (rt.effective_from IS NULL OR r.effective_from < rt.effective_from)

CREATE VIEW pokemon_type_effective AS
SELECT pt.pokemon_id, pt.slot, pt.type_id, r.id AS regulation_id
FROM pokemon_type pt
JOIN regulation rf ON rf.id = pt.valid_from_regulation_id
LEFT JOIN regulation rt ON rt.id = pt.valid_to_regulation_id
JOIN regulation r
  ON r.effective_from >= rf.effective_from
 AND (rt.effective_from IS NULL OR r.effective_from < rt.effective_from);

CREATE VIEW pokemon_ability_effective AS
SELECT pa.pokemon_id, pa.slot, pa.ability_id, r.id AS regulation_id
FROM pokemon_ability pa
JOIN regulation rf ON rf.id = pa.valid_from_regulation_id
LEFT JOIN regulation rt ON rt.id = pa.valid_to_regulation_id
JOIN regulation r
  ON r.effective_from >= rf.effective_from
 AND (rt.effective_from IS NULL OR r.effective_from < rt.effective_from);

CREATE VIEW pokemon_move_effective AS
SELECT pm.pokemon_id, pm.move_id, pm.learn_method, pm.level, r.id AS regulation_id
FROM pokemon_move pm
JOIN regulation rf ON rf.id = pm.valid_from_regulation_id
LEFT JOIN regulation rt ON rt.id = pm.valid_to_regulation_id
JOIN regulation r
  ON r.effective_from >= rf.effective_from
 AND (rt.effective_from IS NULL OR r.effective_from < rt.effective_from);

-- Exact integer math: (a*b)/100 yields 400/200/100/50/25/0 with no float comparison
-- and no log-product hackery, because the slot cap is 2.
CREATE VIEW pokemon_defense_effectiveness AS
SELECT s1.pokemon_id,
       s1.regulation_id,
       t.id AS attacking_type_id,
       (r1.multiplier_pct * COALESCE(r2.multiplier_pct, 100)) / 100 AS multiplier_pct
FROM pokemon_type_effective s1
CROSS JOIN type t
JOIN type_chart_rule r1
  ON r1.attacking_type_id = t.id AND r1.defending_type_id = s1.type_id
LEFT JOIN pokemon_type_effective s2
  ON s2.pokemon_id    = s1.pokemon_id
 AND s2.regulation_id = s1.regulation_id
 AND s2.slot          = 2
LEFT JOIN type_chart_rule r2
  ON r2.attacking_type_id = t.id AND r2.defending_type_id = s2.type_id
WHERE s1.slot = 1;

-- ============================================================ indexes
-- PK/UNIQUE already cover forward lookups; these are the reverse directions.
CREATE INDEX idx_pokemon_move_move    ON pokemon_move(move_id);
CREATE INDEX idx_pokemon_type_type    ON pokemon_type(type_id);
CREATE INDEX idx_pokemon_ability_abil ON pokemon_ability(ability_id);
CREATE INDEX idx_move_data_type       ON move_data(type_id);
CREATE INDEX idx_variant_base         ON variant(base_pokemon_id);
