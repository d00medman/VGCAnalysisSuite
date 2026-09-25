-- 0004_items.sql — held items and their per-regulation legality.

-- Identity plus unversioned facts, like `ability`. display_name is kept (unlike move and
-- ability) because battle text prints it: "Floette's Floettite is reacting ..."
CREATE TABLE item (
  id           BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
  name         TEXT NOT NULL UNIQUE,   -- 'focus-sash'
  display_name TEXT NOT NULL,          -- 'Focus Sash'
  category     TEXT NOT NULL CHECK (category IN ('mega-stone','berry','other')),
  fling_power  BIGINT CHECK (fling_power IS NULL OR fling_power >= 0),
  description  TEXT                    -- deliberately not regulation-scoped
);

-- Set membership: which items are legal in which regulation. The same half-open
-- [valid_from, valid_to) shape as pokemon_move, because legality is exactly the case
-- scalar "latest wins" rows cannot express: an item can be banned and later return.
CREATE TABLE item_legality (
  item_id                  BIGINT NOT NULL REFERENCES item(id) ON DELETE CASCADE,
  valid_from_regulation_id BIGINT NOT NULL REFERENCES regulation(id),
  valid_to_regulation_id   BIGINT          REFERENCES regulation(id),
  PRIMARY KEY (item_id, valid_from_regulation_id)
);

CREATE UNIQUE INDEX item_legality_one_open_window
    ON item_legality (item_id)
    WHERE valid_to_regulation_id IS NULL;

CREATE VIEW item_legal_effective AS
SELECT il.item_id, r.id AS regulation_id
FROM item_legality il
JOIN regulation rf ON rf.id = il.valid_from_regulation_id
LEFT JOIN regulation rt ON rt.id = il.valid_to_regulation_id
JOIN regulation r
  ON r.effective_from >= rf.effective_from
 AND (rt.effective_from IS NULL OR r.effective_from < rt.effective_from);
