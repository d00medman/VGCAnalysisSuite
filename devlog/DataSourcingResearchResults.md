# Data Sourcing — Research & Results

How data reaches the DB. Schema rationale: `SchemaAndIngestPlan.md`. `DECISION:` is
greppable; **[unverified]** = not checked in-session; reversals get a new entry.

---

## 2026-09-03 (rev 2) — sourcing settled, exporter written, schema fit assessed

### 1. Stats: no level or IV dimension

Every battle is Level 50, IVs are fixed at 31, and EVs are replaced by **Stat Points**
(0–32 per stat, 66 total). Natures still apply ×1.1/×0.9:

```
HP     = base_hp + SP_hp + 75
others = trunc((base + SP + 20) * nature_mod)
```

Verified against `storedStats` in a live `gen9championsvgc2026regmb` battle; Smogon spreads
arrive in SP units, confirming the caps independently.

- `DECISION:` store base stats, compute L50 on read. No level table, no IV modelling. SP
  caps are format rules, not columns. (Legacy EVs, if ever needed: `floor(EV/4) -> 2*SP-1`.)

### 2. Champions is a rebalance, not a re-skin

**500 of 954 moves are legal**; the rest are `isNonstandard: "Past"`. Move stats changed
(Beak Blast 100 BP/15 PP → 120/5), movepools were trimmed (Garchomp 58 vs PokéAPI's 94),
paralysis is 1/8. No Tera; Megas are in.

- `DECISION:` never seed from vanilla Gen 9 — every ingest applies the Champions overlay.
  This is the easiest way for the project to end up plausible-looking and wrong.

### 3. Sources, ranked

1. **`smogon/pokemon-showdown`, `data/mods/champions/`** (MIT) — canonical, our only ingest
   input. Sibling `championsregma` is the previous regulation.
2. **Smogon usage stats** — `stats/YYYY-MM/chaos/<format>-<cutoff>.json`, live since
   2026-08. Values are **weighted counts, not percentages**.
3. **PokéAPI** (BSD-3) — confirms Garchomp's 58 moves, but has **none** of the rebalances
   and a form-blind dex. Cosmetic layer only: sprites, genus, flavour.
4. **Pikalytics** — only source for the in-game HOME ladder and tournament sheets; one
   request per Pokémon. **[unverified]** whether bulk use is allowed.
5. **`otterlyclueless/pokemon-champions-data`** — stale, admits its SP formula is unverified.

**Rejected:** Serebii/Bulbapedia HTML (no stable contract) and the `chouten.dev` SEO farms,
whose numbers reproduce under no formula.

Licensing: Showdown MIT, PokéAPI BSD-3, Smogon public, Pikalytics unstated — all downstream
of Nintendo IP.

### 4. "Most recent moveset" is two questions

*Legally learnable* → Champions learnsets, static per regulation, ingested today. *Actually
played* → Smogon/Pikalytics, keyed `(format, month, cutoff, species, move)` with a weight.

- `DECISION:` separate tables. Conflated, the DB can't answer "was this good last month".
  The second is not built.

### 5. The exporter — `pokedex/ingest/`

**Why JavaScript:** no HTTP API exists. The data is TypeScript inside the Showdown repo, and
the mod is an *overlay* — base stats and the type chart inherit from root Gen 9; moves and
learnsets are partial overrides. Parsing the `.ts` means reimplementing Showdown's inherit
chain and getting it subtly wrong. The npm package *is* the interface: run it once, hand Rust
flat JSON.

```
cd pokedex/ingest && npm install
node showdown-snapshot.js champions   # -> out/snapshot.regulation-m-b.json
pokedex import --file out/snapshot.regulation-m-b.json
```

It emits `model::Snapshot` directly, not a raw dump — one translation layer, one file,
Showdown-isms commented where they happen. `typechart-check.json` is verification, not input.

- `DECISION:` one mod = one regulation (`champions` → M-B, `championsregma` → M-A). Loading
  both is what makes the temporal schema do work.
- `DECISION:` `out/` is gitignored; pinning `pokemon-showdown` to 0.11.11 is what buys
  reproducibility. (Reverses rev 1's "commit the snapshot".)

### 6. Schema fit — verified by importing both regulations

**Holds.** 347 entries, 0 collisions on `(national_dex_no, form_slug)`; 0 species over two
types; base stats 15–230, inside `CHECK 1..255`; heights/weights convert to exact integer
dm/hg; migration 0002's 324-row chart matches Showdown with **0 mismatches**, so it needs no
ingest path; `variant`/`variant_kind` absorbs 76 megas + 15 regionals + 49 other formes with
no special-casing.

**Sparseness works.** M-A wrote 309 stat and 500 move rows; M-B added **38 stat rows and 0
move rows** — its only change is 38 new species. Re-import is a clean no-op. Views resolve
309 species for M-A, 347 for M-B; Garchomp-Mega 4× to Ice, BST 700 `sourced_from M-A`.

**Two bugs the schema caught in rev 1's script,** both fixed. Abilities were read
positionally via `Object.values()`, but Showdown keys them `0/1/H/S` and **95 of 347**
species have a hidden ability and no second normal one, so hidden landed in `secondary`.
Megas were found by `forme.startsWith('Mega')`, missing Meowstic-M/F-Mega; `item.megaStone`
gives the correct **76**.

**Gaps, by cost:**

1. **No legality interval table.** Sparse scalar tables cannot express *removal*, so a
   delisted species would resolve stale stats forever. Untested rather than broken — M-A→M-B
   removed nothing — but rosters rotate. Wants intervals, ditto the 454 cut moves.
2. **`secondary_effect` is one TEXT column**, but 120 moves have a structured secondary and
   4 have two. Stuffed in as JSON for now; wants a `move_secondary` table before the
   calculator is trustworthy.
3. **No `display_name` on `move`/`ability`** — `type` has one. Names are kebab slugs
   (`beak-blast`) to match `type.name` and join PokéAPI later, so "Beak Blast" is lost.
4. **Ability slot `S`** has nowhere to go: Greninja's Battle Bond, dropped with a warning.
5. **`learn_method`/`level` carry no information** — all 14,192 entries are `9M`; Champions
   has no level-up learning. Don't read meaning into `machine`.
6. **Illegal base formes still need identity rows.** Floette is illegal, Floette-Eternal
   and -Mega are not, and `base_pokemon_id` is a hard FK — so a stats-less row is emitted,
   appearing in 0 regulations. Works only because identity and regulation-scoped facts are
   separate tables. Keep it that way.
7. **No item table** — `required_item` is free text; 148 items have nowhere to live.
8. **Stellar excluded** — Tera-only, inert here.

### 7. Open questions

1. Model every regulation or snapshot one? Currently both, which keeps the dimension
   load-bearing.
2. Which ladder is truth — HOME (Pikalytics) or Showdown (Smogon)? Different metagames.
3. Per-regulation item/ability legality: `championsregma/items.ts` is 2 KB of overrides vs
   16 KB. Needs a diff.
4. Record the upstream Showdown SHA in the DB so rows trace to a source version.
5. Megas do not revert on fainting here. Calculator concern, not a data-model one.

### Sources

[pokemon-showdown](https://github.com/smogon/pokemon-showdown) ·
[Smogon stats](https://www.smogon.com/stats/2026-08/) ·
[PokéAPI](https://pokeapi.co/) ·
[Pikalytics](https://www.pikalytics.com/llms-full.txt) ·
[Serebii](https://www.serebii.net/pokemonchampions/preview/) ·
[Bulbapedia](https://bulbapedia.bulbagarden.net/wiki/Stat_point) ·
[pokemon-champions-data](https://github.com/otterlyclueless/pokemon-champions-data)
