# Pokédex DevLog

Append-only journal for the VGC/Champions Pokédex project. Newest entry at the top.
The database itself lives in `pokedex/`. This file is for *what we learned and why we
decided things* — not for schema DDL or code, which belong in the repo.

**Conventions**
- One entry per working session, dated `YYYY-MM-DD`, with a one-line title.
- Every factual claim gets a source or a "verified by" note. Unverified claims are
  labelled **[unverified]** so they don't calcify into assumptions.
- Decisions get their own bullet prefixed `DECISION:` so they're greppable.
- Reversals are new entries, never edits to old ones.

---

## 2026-09-03 — Data sourcing research (no code committed to `pokedex/`)

Scope: *how* we source the data, not the schema. Schema specifics deferred to the
session building the Rust ingestion layer.

### 1. The level question — answered, and it's better than expected

Pokémon Champions locks **every** battle to **Level 50**. Confirmed three ways:
Serebii's preview, Bulbapedia, and the actual Showdown implementation (the Champions
mod's `standardag` ruleset hardcodes `'Adjust Level = 50'`).

More importantly, Champions **replaced IVs/EVs with Stat Points (SP)**:

- Every Pokémon is treated as having **31 IVs in all stats**. No breeding, no RNG.
- **0–32 SP per stat, 66 SP total** across all six.
- Natures still exist and still apply ×1.1 / ×0.9 / ×1.0.

Because IVs are fixed and the level is fixed, a level-50 stat collapses to a **pure
function of base stat + SP + nature**:

```
HP     = base_hp + SP_hp + 75
others = trunc((base + SP + 20) * nature_mod)      nature_mod in {0.9, 1.0, 1.1}
```

**Verified** against the real Showdown Champions simulator (instantiated a
`gen9championsvgc2026regmb` battle and compared `storedStats` to the formula):

| Set | Sim | Formula |
|---|---|---|
| Garchomp-Mega, Jolly, 0/32/0/0/2/32 | 183/222/135/126/117/158 | identical |
| Kingambit, Adamant, 32/32/0/0/1/1 | 207/205/140/72/106/71 | identical |

- `DECISION:` **do not store computed level-50 stats.** Store the six base stats on the
  central pokemon table and compute on read. There is no level table to build, and no
  IV dimension to model at all.
- SP caps (32 per stat / 66 total) are *format rules*, not Pokémon attributes — they
  belong wherever we model regulations, or as constants in the app layer.
- HOME transfer conversion, for reference: the first SP in a stat costs 4 EVs, each
  additional one costs 8. In classic-formula terms the mod substitutes
  `floor(EV/4) -> max(2*SP - 1, 0)`. Only relevant if we ever ingest legacy EV spreads.
- Nature multiplication uses 16-bit truncation in-sim (`tr(tr(stat*110, 16)/100)`).
  Irrelevant at Champions stat magnitudes, but noted so nobody "fixes" a rounding bug
  that isn't one.

### 2. Critical finding: Champions is a **rebalance**, not a re-skin

This is the finding that most affects the ingestion design. Champions does not reuse
Scarlet/Violet's numbers:

- **Move stats changed.** Beak Blast: 100 BP / 15 PP in SV → **120 BP / 5 PP** in
  Champions. Anchor Shot 90 BP, Apple Acid 90 BP, Baneful Bunker 5 PP, and many more.
- **Moves were cut.** Only **500 of 954** moves are legal; the rest are flagged
  `isNonstandard: "Past"`.
- **Movepools were trimmed.** Garchomp learns 58 moves in Champions vs 94 entries in
  PokéAPI's full record; Incineroar 77 vs 104; Whimsicott 50 vs 69.
- **Status mechanics changed.** Paralysis is a 1/8 full-paralysis chance (was 1/4);
  sleep lasts 2–3 turns (was 1–3).
- **No Terastallization.** Usage data shows `Tera Types: {nothing: 100%}`. Mega
  Evolution *is* in (74 mega formes legal). Z-Moves/Dynamax only teased.

> Consequence: **any pipeline seeded from vanilla Gen 9 data will be silently wrong.**
> Every ingest must apply the Champions overlay. This is the single easiest way for this
> project to produce a plausible-looking but incorrect damage calculator.

### 3. Source ranking

**Tier 1 — canonical: `smogon/pokemon-showdown`, `data/mods/champions/`** (MIT licensed)
- Files: `formats-data.ts` (per-species legality + tier), `learnsets.ts` (Champions
  movepools), `moves.ts` (BP/PP/legality overrides), `abilities.ts`, `items.ts`,
  `conditions.ts` (status rebalance), `rulesets.ts`, `scripts.ts` (the SP stat formula).
- Base stats, types and the type chart are inherited from the root `data/pokedex.ts` and
  `data/typechart.ts` — the mod only overrides what changed.
- Sibling mod `data/mods/championsregma/` = the **previous** regulation (Reg M-A).
- Because data is spread across an inherit chain, **do not hand-parse the `.ts` files.**
  Let the dex resolve it (see §5).

**Tier 2 — "what people actually run": Smogon usage stats**
- `https://www.smogon.com/stats/YYYY-MM/chaos/<format>-<cutoff>.json`
- Champions formats present as of 2026-08: `gen9championsvgc2026regmb`,
  `...regmbbo3`, `gen9championsou`, `gen9championsuu`, `gen9championsbssregmb`.
- Cutoffs `0 / 1500 / 1630 / 1760`. Monthly. `regmb-1760` is 12.9 MB, 277 Pokémon,
  1,269,250 battles.
- Per-Pokémon: `Moves`, `Abilities`, `Items`, `Spreads`, `Teammates`,
  `Checks and Counters`, `Tera Types`, `usage`, `Raw count`, `Viability Ceiling`.
- Values are **weighted counts, not percentages** — normalise against the sum.
- Spreads arrive already in SP units, e.g. `"Adamant:32/32/0/0/1/1"` — independent
  confirmation of the 32/66 model.
- Aug 2026 (regmb @1760) top of meta: Kingambit 47.2%, Incineroar 33.6%,
  Garchomp 32.2%, Basculegion 31.5%, Sneasler 31.4%, Charizard-Mega-Y 26.5%,
  Sinistcha 26.1%, Whimsicott 23.5%.

**Tier 3 — supplement + cross-validation: PokéAPI** (BSD-3, CSVs in-repo)
- It *does* now have Champions: `version-group/champions` (id 32) and
  `pokedex/champions` (208 species entries). Champions learnsets are tagged on
  `pokemon/<name>.moves[].version_group_details` — Garchomp returns exactly 58 moves,
  **matching Showdown**. Useful as an independent check.
- It does **not** carry the Champions move rebalances: `move/beak-blast` still reports
  100 BP / 15 PP with an empty `past_values`. So it cannot be the move-data source.
- Its champions pokedex is form-blind (208 species, no megas/regional split), so it
  cannot drive legality either.
- Best used for: sprites, flavour text, dex numbers, species metadata, egg groups —
  the cosmetic layer around our competitive core.

**Tier 4 — the official in-game ladder: Pikalytics**
- The only source that aggregates **Pokémon HOME Battle Stadium** data, i.e. the real
  in-game ranked ladder rather than the Showdown simulator. Current default format code
  `battledataregmbs3`.
- Has an explicitly AI-addressable, markdown-returning API:
  `GET /ai/pokedex/{format}` and `GET /ai/pokedex/{format}/{pokemon}`; contract
  documented at `https://www.pikalytics.com/llms-full.txt`.
- Also aggregates RK9 / Limitless / Victory Road tournament team sheets
  (`/ai/tournaments/...`) — the closest thing to "what wins events".
- Markdown-per-Pokémon means ~350 requests for a full pull. Fine occasionally, not a
  bulk pipeline. No published bulk endpoint or API ToS. **[unverified]** whether
  volume scraping is permitted — ask before automating.

**Tier 5 — do not use as primary: `otterlyclueless/pokemon-champions-data`**
- Structured JSON (base-stats, roster, moves, learnsets, abilities, items, type chart,
  natures) and superficially exactly what we want. But: last push **2026-04-16**
  (launch-era, 258 entries — the roster is now 347), and its own
  `mechanics/stat-formula.md` says the SP→stat mapping is *"still being
  community-verified"* and falls back to the classic EV formula. It is stale *and*
  wrong on the one mechanic we most need.
- Keep as a sanity-check corpus only.

**Rejected:** scraping Serebii / Bulbapedia HTML. Both are excellent human references
and were used above to establish the mechanics, but they're editorial HTML with no
stable contract. `chouten.dev`, `champdex.com`, `champslab.xyz`, `genpkm.com` etc. look
like SEO/AI-generated content farms — chouten's stat article contains numbers that
don't reproduce under any formula (claims base-100 Speed = 156 at L50). Treat all of
them as untrustworthy.

### 4. Roster and regulation churn

- Launch (Apr 2026): 269 available — 210 regular + 59 megas.
- Now (Reg M-B): **347** legal entries in the `champions` mod, incl. **74** megas.
  `championsregma` (Reg M-A) has 318. Bulbapedia counts 208 *species* + 75 mega formes.
- The counts differ because everyone counts a different thing (species vs. formes vs.
  legal-in-current-reg vs. obtainable-vs-transfer-only). Pick one and define it.
- No Legendaries or Mythicals at all. Everything is fully evolved except Pikachu.
- Showdown format names in play: `[Gen 9 Champions] VGC 2026 Reg M-B` (+ Bo3), `BSS Reg
  M-B`, `OU`, `UU`, plus the Reg M-A equivalents on the older mod.

- `DECISION:` legality is **not** a column on the pokemon table. Regulations rotate and
  a Pokémon's legality is a fact about *(pokemon, regulation)*. Whatever the final
  schema, it needs a regulation/format dimension with a join. Same for per-regulation
  item and ability legality.

### 5. Verified extraction recipe

Rather than parsing the mod's TypeScript, install the simulator and let it resolve the
inherit chain, then dump flat JSON for the Rust ingester to read. **Verified working**
against `pokemon-showdown@0.11.11` (upstream master `2f5b2739`, 2026-09-02):

```
npm install pokemon-showdown
node pokedex_data_export.js      # script sits next to this log
```

Measured output:

| File | Rows | Size |
|---|---|---|
| `species.json` | 347 | 140 KB |
| `moves.json` | 500 | 184 KB |
| `learnsets.json` | 14,192 | 1.2 MB |
| `typechart.json` | 361 | 26 KB |
| `natures.json` | 25 | 2 KB |
| `abilities.json` | 316 | 50 KB |
| `items.json` | 148 | 25 KB |

~1.6 MB total. Small enough to commit as a versioned snapshot, which is probably what we
want for reproducibility — a Pokédex that silently changes under the app is worse than
one that's a month stale.

Notes for whoever maps this to tables:

- **Type chart.** Showdown stores it *transposed*: each defending type carries a
  `damageTaken` map with codes `0 = 1x, 1 = 2x, 2 = 0.5x, 3 = immune`. The export
  flattens it to `(attacking, defending, multiplier)` — which is exactly the
  `TypeChartRule` join-row shape described for the schema. 19 types (18 + Stellar);
  361 = 19² rows, of which **120 are non-1x**. Stellar is Tera-only, so it's inert in
  Champions — keep it in the types table, expect zero Pokémon to carry it.
- **Two types max is safe.** Verified empirically across all 347 legal entries: 234
  dual-type, 113 mono-type, **0** with more than two. The `type_1 / type_2` constraint
  holds.
- **Moves map 1:1** onto the described Move table: `type`, `base_power`, `accuracy`,
  `secondary`. Two gotchas: (a) never-miss moves have `accuracy === true` in Showdown —
  the export normalises that to `null`; (b) `secondary` is a *nested object*
  (`{chance, status | boosts | volatileStatus | self | ...}`) and some moves have a
  `secondaries` *array*. **120 of 500** legal moves have one. Either a JSON column or a
  `MoveSecondary` child table — a single scalar column will not hold it.
- **Megas** appear as distinct species entries with their own types/base stats/ability,
  linked by `base_species` and gated by `required_item` (e.g. Garchomp-Mega,
  `Garchompite`, Sand Force, 108/170/115/120/95/92). They need to be rows, not a flag.
- **Learnsets** resolve cleanly: every legal base forme has an explicit Champions
  override (0 fell through to Gen 9 data), so the movepools are genuinely
  Champions-authored, not inherited.

### 6. "Most recent moveset" is two different questions

Worth splitting before it gets baked into a column name:

- **(a) What can it legally learn?** → Champions learnsets (§5 `learnsets.json`, or
  PokéAPI's champions-tagged moves as a cross-check). Static per regulation.
- **(b) What do people actually run?** → Smogon chaos JSON (§2) and/or Pikalytics (§4).
  Time-varying, format-specific, and the more useful answer for VGC prep.

- `DECISION:` model these separately. (a) is a `pokemon × move` legality join. (b) is a
  time series keyed roughly `(format, month, ladder_cutoff, species, move)` with a
  weight — **not** columns on the pokemon table. Conflating them means the DB can't
  answer "was this a good set last month" or "is this legal in Reg M-C".

### 7. Licensing

- Pokémon Showdown: **MIT**. Fine to redistribute a derived data snapshot with
  attribution.
- PokéAPI: **BSD-3-Clause**.
- Smogon usage stats: publicly published flat files, no auth.
- Pikalytics: **[unverified]** — no stated bulk-use terms. Ask before automating.
- All of it is derived from Nintendo/Game Freak IP; fine for a personal tool, worth a
  thought before anything public-facing.

### 8. Open questions

1. **Which regulation are we targeting?** Reg M-B today, and it rotates. Do we snapshot
   one, or model all of them from the start? Affects whether the regulation dimension is
   load-bearing or vestigial.
2. **Which ladder is "truth" for VGC relevance** — the official HOME Battle Stadium data
   (via Pikalytics) or the Showdown simulator ladder (via Smogon stats)? They are
   different metagames. Showdown is bulk-accessible; HOME is the real game.
3. **Per-regulation item/ability legality.** `championsregma/items.ts` is only ~2 KB of
   overrides vs `champions/items.ts` at 16 KB — needs a proper diff before assuming
   items are regulation-invariant.
4. **Snapshot vs. live fetch.** Recommend pinning an upstream commit SHA and recording
   it in the DB alongside the data, so any row can be traced to a source version.
5. Mega Evolution interacts oddly in Champions (the mod deliberately does *not* revert
   megas on fainting). Not a data-model issue yet, but flag it for the calculator.

### Sources

- [Serebii — Pokémon Champions preview](https://www.serebii.net/pokemonchampions/preview/)
- [Bulbapedia — Stat point](https://bulbapedia.bulbagarden.net/wiki/Stat_point)
- [Bulbapedia — List of Pokémon in Pokémon Champions](https://bulbapedia.bulbagarden.net/wiki/List_of_Pok%C3%A9mon_in_Pok%C3%A9mon_Champions)
- [smogon/pokemon-showdown](https://github.com/smogon/pokemon-showdown) — `data/mods/champions/`, `config/formats.ts`
- [Smogon usage stats, 2026-08](https://www.smogon.com/stats/2026-08/)
- [PokéAPI](https://pokeapi.co/) — `version-group/champions`, `pokedex/champions`
- [Pikalytics](https://www.pikalytics.com/) — [AI contract](https://www.pikalytics.com/llms-full.txt)
- [otterlyclueless/pokemon-champions-data](https://github.com/otterlyclueless/pokemon-champions-data)
- [Engadget — Pokémon Champions review](https://www.engadget.com/2245080/pokmon-champions-review/)
