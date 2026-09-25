-- 0006_display_names.sql — names as the game prints them, for moves and abilities.
--
-- The slug in `name` cannot be turned back into the display name: 15 moves alone differ
-- ("kings-shield" is King's Shield, "u-turn" is U-turn, "double-edge" is Double-Edge).
-- Battle text prints display names, so matching transcript lines needs them. Nullable:
-- rows created by a bare reference (a learnset entry) get one when their record arrives.
ALTER TABLE move    ADD COLUMN display_name TEXT;
ALTER TABLE ability ADD COLUMN display_name TEXT;
