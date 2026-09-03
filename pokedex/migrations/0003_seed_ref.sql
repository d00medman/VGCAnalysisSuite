-- 0003_seed_ref.sql — static reference data that needs no research.
-- Natures: fixed since Gen 3. Ids match PokeAPI ordering.
-- NOTE: nature is a per-INDIVIDUAL trait, not a species trait, so nothing in this
-- schema joins to it. It is correct and unused until an individuals table exists.

INSERT INTO nature (id, name, increased_stat, decreased_stat) VALUES
  ( 1, 'hardy'    , NULL         , NULL         ),
  ( 2, 'bold'     , 'defense'    , 'attack'     ),
  ( 3, 'modest'   , 'sp_attack'  , 'attack'     ),
  ( 4, 'calm'     , 'sp_defense' , 'attack'     ),
  ( 5, 'timid'    , 'speed'      , 'attack'     ),
  ( 6, 'lonely'   , 'attack'     , 'defense'    ),
  ( 7, 'docile'   , NULL         , NULL         ),
  ( 8, 'mild'     , 'sp_attack'  , 'defense'    ),
  ( 9, 'gentle'   , 'sp_defense' , 'defense'    ),
  (10, 'hasty'    , 'speed'      , 'defense'    ),
  (11, 'adamant'  , 'attack'     , 'sp_attack'  ),
  (12, 'impish'   , 'defense'    , 'sp_attack'  ),
  (13, 'bashful'  , NULL         , NULL         ),
  (14, 'careful'  , 'sp_defense' , 'sp_attack'  ),
  (15, 'rash'     , 'sp_attack'  , 'sp_defense' ),
  (16, 'jolly'    , 'speed'      , 'sp_attack'  ),
  (17, 'naughty'  , 'attack'     , 'sp_defense' ),
  (18, 'lax'      , 'defense'    , 'sp_defense' ),
  (19, 'quirky'   , NULL         , NULL         ),
  (20, 'naive'    , 'speed'      , 'sp_defense' ),
  (21, 'brave'    , 'attack'     , 'speed'      ),
  (22, 'relaxed'  , 'defense'    , 'speed'      ),
  (23, 'quiet'    , 'sp_attack'  , 'speed'      ),
  (24, 'sassy'    , 'sp_defense' , 'speed'      ),
  (25, 'serious'  , NULL         , NULL         );

INSERT INTO variant_kind (id, name, description) VALUES
  (1, 'mega'       , 'Battle-time transformation requiring a Mega Stone'),
  (2, 'primal'     , 'Primal Reversion (Kyogre, Groudon)'),
  (3, 'regional'   , 'Regional form with different stats/typing (Alolan, Galarian, Hisuian, Paldean)'),
  (4, 'gigantamax' , 'Gigantamax form'),
  (5, 'paradox'    , 'Past/Future Paradox counterpart'),
  (6, 'form'       , 'Alternate form with distinct stats (Rotom, Deoxys, Urshifu, ...)'),
  (7, 'other'      , 'Anything not covered above');
