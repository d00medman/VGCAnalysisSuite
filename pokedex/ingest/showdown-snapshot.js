#!/usr/bin/env node
'use strict';

/**
 * Showdown -> pokedex snapshot exporter.
 *
 *   npm install                       # pins pokemon-showdown 0.11.11
 *   node showdown-snapshot.js champions        > writes snapshot.regulation-m-b.json
 *   node showdown-snapshot.js championsregma   > writes snapshot.regulation-m-a.json
 *
 * WHY THIS IS JAVASCRIPT
 * ----------------------
 * Pokemon Champions data has no HTTP API. It ships as TypeScript source inside
 * `smogon/pokemon-showdown`, under `data/mods/champions/`, and it is an *overlay*:
 * base stats and the type chart are inherited from the root Gen 9 files, while moves,
 * learnsets and legality are partial overrides. Reading the .ts files directly means
 * reimplementing Showdown's inherit chain, and getting it subtly wrong. The npm
 * package IS the interface, so we run it once here to flatten the overlay and hand
 * Rust something boring.
 *
 * WHAT IT EMITS
 * -------------
 * `snapshot.<regulation>.json`, matching `pokedex::model::Snapshot` field for field.
 * It is a *complete* snapshot of one regulation: every legal species, every legal
 * move, whole learnsets. Ingest diffs it against the previous regulation and stores
 * only what changed (see devlog: "feed dense, store sparse").
 *
 * `typechart-check.json` is NOT ingest input. Migration 0002 already seeds the 324-row
 * chart; this file exists so we can assert the seed still agrees with Showdown.
 *
 * WHAT IT DELIBERATELY DOES NOT EMIT
 * ----------------------------------
 * - Level-50 stats. They are a pure function of base stat + SP + nature, computed on
 *   read. Champions fixes IVs at 31 and the level at 50, so there is nothing to store.
 * - Natures and the type chart. Natures are seeded (0003); the chart is seeded (0002)
 *   and only checked here.
 * - `genus`. Showdown has no flavour text. That comes from PokeAPI later.
 */

const fs = require('fs');
const path = require('path');
const { Dex } = require('pokemon-showdown');

// Each Showdown mod is a frozen snapshot of one regulation. `champions` tracks the
// current one; siblings hold the previous ones. Loading both is how the temporal
// schema earns its keep.
const REGULATION_OF_MOD = {
  champions: 'Regulation M-B',
  championsregma: 'Regulation M-A',
};

// Formes whose prefix marks a regional variant. Everything else with a mega stone is
// 'mega', and the long tail (Rotom appliances, Vivillon patterns, Alcremie creams,
// Meowstic gender formes...) is plain 'form'. Matches variant_kind seeded in 0003.
const REGIONAL_PREFIXES = ['Alola', 'Galar', 'Hisui', 'Paldea'];

// ---------------------------------------------------------------- naming

/**
 * Canonical name form used everywhere in the database: lowercase, kebab-cased,
 * punctuation dropped. "Beak Blast" -> "beak-blast", "King's Shield" -> "kings-shield".
 *
 * This matches the convention already seeded for `type.name` and is the same shape
 * PokeAPI uses, so the cosmetic layer (sprites, genus, dex flavour) can join on it
 * later without a translation table. The cost is that display names are lost --
 * `move` and `ability` have no display_name column yet, unlike `type`.
 */
function slug(name) {
  return name
    .toLowerCase()
    .replace(/['".]/g, '')
    .replace(/[^a-z0-9]+/g, '-')
    .replace(/^-+|-+$/g, '');
}

/** `pokemon.form_slug`: '' for a base forme, else the slugified forme. */
function formSlug(species) {
  return species.forme ? slug(species.forme) : '';
}

// ---------------------------------------------------------------- species

/**
 * Legal in this regulation. Showdown marks everything else either `Illegal` (cut from
 * Champions) or `isNonstandard: 'Past'` (exists in Gen 9, not in this game).
 */
function legalSpecies(dex) {
  return dex.species.all().filter((s) => s.tier !== 'Illegal' && !s.isNonstandard);
}

/**
 * Variant classification, or null for a base forme.
 *
 * Mega detection goes through the item, not the forme string: Meowstic's megas are
 * named "M-Mega"/"F-Mega", so a `forme.startsWith('Mega')` test silently misses two
 * of the 76 mega formes.
 */
function variantOf(dex, species) {
  if (!species.forme) return null;

  const requiredItem = species.requiredItem || null;
  const isMega = requiredItem !== null && dex.items.get(requiredItem).megaStone;
  const isRegional = REGIONAL_PREFIXES.some((p) => species.forme.startsWith(p));

  return {
    // Every variant hangs off its base forme, which always has form_slug ''.
    base_form_slug: '',
    kind: isMega ? 'mega' : isRegional ? 'regional' : 'form',
    required_item: requiredItem,
  };
}

/**
 * Abilities keyed by schema slot.
 *
 * Showdown keys these '0' / '1' / 'H' / 'S'. They must be read by key, never by
 * position: 95 of 347 species have a hidden ability and no second normal ability, so
 * `Object.values()` writes their hidden ability into the secondary slot.
 *
 * 'S' is the signature-only slot (Greninja's Battle Bond, the sole case). The schema's
 * slot CHECK has no room for it, so it is dropped and reported.
 */
function abilitiesBySlot(species, warnings) {
  if (species.abilities.S) {
    warnings.push(`dropped signature ability ${species.abilities.S} on ${species.name} (no schema slot)`);
  }
  return {
    primary: species.abilities['0'] ? slug(species.abilities['0']) : null,
    secondary: species.abilities['1'] ? slug(species.abilities['1']) : null,
    hidden: species.abilities.H ? slug(species.abilities.H) : null,
  };
}

/**
 * Learnset as (move, method) pairs.
 *
 * Champions has no level-up learning at all -- every one of the 14,192 entries is
 * tagged `9M` (machine). `learn_method` and `level` therefore carry no information
 * here; they stay in the record because the schema is not Champions-specific.
 */
function learnsetOf(dex, species) {
  const learnset = dex.species.getLearnsetData(species.id).learnset || {};
  const entries = [];

  for (const moveId of Object.keys(learnset)) {
    const move = dex.moves.get(moveId);
    if (!move.exists || move.isNonstandard) continue; // learnable in Gen 9, cut from Champions
    entries.push({ move: slug(move.name), method: 'machine' });
  }
  return entries;
}

function speciesRecord(dex, species, warnings) {
  return {
    national_dex_no: species.num,
    form_slug: formSlug(species),
    name: species.name,
    genus: null, // Showdown has none; PokeAPI supplies it later
    height_dm: Math.round(species.heightm * 10), // metres -> decimetres
    weight_hg: Math.round(species.weightkg * 10), // kilograms -> hectograms
    variant: variantOf(dex, species),
    stats: {
      hp: species.baseStats.hp,
      attack: species.baseStats.atk,
      defense: species.baseStats.def,
      sp_attack: species.baseStats.spa,
      sp_defense: species.baseStats.spd,
      speed: species.baseStats.spe,
    },
    types: species.types.map(slug), // ordered: index 0 is slot 1
    abilities: abilitiesBySlot(species, warnings),
    learnset: learnsetOf(dex, species),
  };
}

/**
 * Identity-only rows for base formes that are themselves illegal.
 *
 * Floette is not legal in Champions (not fully evolved), but Floette-Eternal and
 * Floette-Mega are, and `variant.base_pokemon_id` is a hard foreign key. So the base
 * gets a `pokemon` row carrying nothing but its natural key. With no stats, types or
 * abilities it never appears in any regulation's resolution views -- the identity
 * table and the regulation-scoped fact tables are separate for exactly this reason.
 *
 * Note the asymmetry with a normal record: omitting `types`/`abilities`/`learnset`
 * means "leave untouched", where an empty array would mean "close every interval".
 */
function baseFormePlaceholder(species) {
  return {
    national_dex_no: species.num,
    form_slug: '',
    name: species.name,
    variant: null,
  };
}

// ---------------------------------------------------------------- moves

function legalMoves(dex) {
  return dex.moves.all().filter((m) => !m.isNonstandard);
}

function moveRecord(move) {
  return {
    name: slug(move.name),
    type: slug(move.type),
    damage_class: move.category.toLowerCase(),

    // Showdown uses 0 for "no fixed power", covering both status moves and the 25
    // variable-power moves (Seismic Toss, Gyro Ball, ...). The schema spells that
    // NULL and reserves 0 for a genuine zero.
    power: move.basePower === 0 ? null : move.basePower,

    // `accuracy === true` is Showdown for "never misses" (132 moves).
    accuracy: move.accuracy === true ? null : move.accuracy,

    pp: move.pp,
    priority: move.priority,

    // STOPGAP. `secondary_effect` is a single TEXT column, but 120 moves carry a
    // structured secondary and 4 (the Fangs, Triple Arrows) carry two. Storing the
    // array as JSON keeps it lossless until there is a move_secondary child table;
    // effect_chance mirrors the first entry so simple queries still work.
    secondary_effect: secondariesOf(move),
    effect_chance: firstSecondaryChance(move),
  };
}

function secondariesOf(move) {
  const list = move.secondaries && move.secondaries.length ? move.secondaries : null;
  return list ? JSON.stringify(list) : null;
}

function firstSecondaryChance(move) {
  const list = move.secondaries;
  if (!list || !list.length || list[0].chance === undefined) return null;
  return list[0].chance;
}

// ---------------------------------------------------------------- items

/**
 * Legal held items. Legality differs per regulation (M-A has fewer than M-B), which is
 * why the snapshot carries the complete list and ingest stores it as intervals.
 * Mega stones are detected through `megaStone`, the same test `variantOf` uses.
 */
function itemRecord(item) {
  return {
    name: slug(item.name),
    display_name: item.name,
    category: item.megaStone ? 'mega-stone' : item.isBerry ? 'berry' : 'other',
    fling_power: item.fling ? item.fling.basePower : null,
    description: item.shortDesc || item.desc || null,
  };
}

function abilityRecord(ability) {
  return { name: slug(ability.name), description: ability.shortDesc || null };
}

// ---------------------------------------------------------------- type chart check

/**
 * The 18x18 chart as (attacking, defending, multiplier_pct), for comparison against
 * migration 0002. Stellar is excluded: it is a Tera-only type, Champions has no
 * Terastallization, and there is no `type` row for it to key against.
 */
function typeChartRows(dex) {
  const types = dex.types.all().map((t) => t.name).filter((n) => n !== 'Stellar');
  const rows = [];

  for (const attacking of types) {
    for (const defending of types) {
      const multiplier = dex.getImmunity(attacking, defending)
        ? Math.pow(2, dex.getEffectiveness(attacking, defending))
        : 0;
      rows.push({
        attacking: slug(attacking),
        defending: slug(defending),
        multiplier_pct: Math.round(multiplier * 100),
      });
    }
  }
  return rows;
}

// ---------------------------------------------------------------- main

function buildSnapshot(dex, regulation, warnings) {
  const species = legalSpecies(dex);
  const legalIds = new Set(species.map((s) => s.id));

  const pokemon = [];

  // Base formes first: `variant.base_pokemon_id` is a self-referencing foreign key,
  // so a variant cannot be inserted before the row it points at.
  for (const s of species) {
    if (!s.forme) pokemon.push(speciesRecord(dex, s, warnings));
  }
  for (const s of species) {
    if (!s.forme) continue;
    const baseId = dex.toID(s.baseSpecies);
    if (!legalIds.has(baseId)) {
      legalIds.add(baseId);
      pokemon.push(baseFormePlaceholder(dex.species.get(baseId)));
      warnings.push(`${s.name}: base forme ${s.baseSpecies} is illegal; emitted identity-only row`);
    }
  }
  for (const s of species) {
    if (s.forme) pokemon.push(speciesRecord(dex, s, warnings));
  }

  return {
    regulation,
    abilities: dex.abilities.all().filter((a) => !a.isNonstandard).map(abilityRecord),
    moves: legalMoves(dex).map(moveRecord),
    pokemon,
    items: dex.items.all().filter((i) => !i.isNonstandard).map(itemRecord),
  };
}

function main() {
  const mod = process.argv[2] || 'champions';
  const outDir = process.argv[3] || path.join(__dirname, 'out');

  const regulation = REGULATION_OF_MOD[mod];
  if (!regulation) {
    console.error(`unknown mod '${mod}'. known: ${Object.keys(REGULATION_OF_MOD).join(', ')}`);
    console.error('add it to REGULATION_OF_MOD once the regulation it maps to is decided.');
    process.exit(1);
  }

  const dex = Dex.mod(mod);
  const warnings = [];
  const snapshot = buildSnapshot(dex, regulation, warnings);

  fs.mkdirSync(outDir, { recursive: true });
  write(outDir, `snapshot.${slug(regulation)}.json`, snapshot);
  write(outDir, 'typechart-check.json', typeChartRows(dex));

  console.log(`\n${mod} -> ${regulation}`);
  console.log(`  pokemon   ${snapshot.pokemon.length}`);
  console.log(`  moves     ${snapshot.moves.length}`);
  console.log(`  abilities ${snapshot.abilities.length}`);
  console.log(`  items     ${snapshot.items.length}`);
  console.log(`  learnset  ${snapshot.pokemon.reduce((n, p) => n + (p.learnset ? p.learnset.length : 0), 0)}`);
  for (const w of warnings) console.log(`  note: ${w}`);
}

function write(dir, file, data) {
  const target = path.join(dir, file);
  fs.writeFileSync(target, JSON.stringify(data, null, 1));
  const kb = (fs.statSync(target).size / 1024).toFixed(0);
  console.log(`${file.padEnd(34)} ${kb.padStart(6)} KB`);
}

main();
