#!/usr/bin/env node
'use strict';

/**
 * Showdown -> pokedex snapshot exporter.
 *
 *   ./export.sh <showdown-sha>        # all three regulations -> ../snapshots/
 *
 *   node showdown-snapshot.js m-a [outDir]                    # npm pokemon-showdown 0.11.11
 *   SHOWDOWN_PATH=<built checkout> SHOWDOWN_SHA=<sha> \
 *     node showdown-snapshot.js m-b|m-c [outDir]              # Showdown master, Node >= 22
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
// npm 0.11.11 by default; a built Showdown checkout when SHOWDOWN_PATH is set.
const SHOWDOWN = process.env.SHOWDOWN_PATH || 'pokemon-showdown';
const { Dex } = require(SHOWDOWN);

/**
 * Where each regulation comes from. Each Showdown mod is a frozen snapshot of one
 * regulation, but the names move: `champions` tracks the current regulation, so it
 * was M-B in npm 0.11.11 and is M-C on master, and master deleted `championsregma`.
 * A mod name alone therefore does not identify a regulation; the source does too.
 * See devlog/DataSourcingResearchResults.md, rev 3.
 */
const EXPORTS = {
  'm-a': { regulation: 'Regulation M-A', source: 'npm', mod: 'championsregma' },
  'm-b': {
    regulation: 'Regulation M-B',
    source: 'git',
    mod: 'championsregmb',
    // championsregmb inherits move legality from `champions`, so M-C's signature moves
    // (Pyro Ball, Snipe Shot, ...) show up as legal with no M-B species to learn them.
    learnedMovesOnly: true,
  },
  'm-c': { regulation: 'Regulation M-C', source: 'git', mod: 'champions' },
};

const SHOWDOWN_REPO = 'https://github.com/smogon/pokemon-showdown';

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
 * later without a translation table. The slug cannot be turned back into the display
 * name (King's Shield, U-turn), so records carry `display_name` too.
 */
function slug(name) {
  return name
    .toLowerCase()
    .replace(/['’".]/g, '') // ’: Farfetch’d, Sirfetch’d
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
function learnsetOf(dex, species, warnings) {
  const source = learnsetSource(dex, species);
  if (source !== species) warnings.inherited.push(`${species.name} <- ${source.name}`);
  const learnset = dex.species.getLearnsetData(source.id).learnset || {};
  const entries = [];

  for (const moveId of Object.keys(learnset)) {
    const move = dex.moves.get(moveId);
    if (!move.exists || move.isNonstandard) continue; // learnable in Gen 9, cut from Champions
    entries.push({ move: slug(move.name), method: 'machine' });
  }
  return entries;
}

/**
 * The species whose learnset this one uses. Showdown keeps no learnset for most formes:
 * Megas, in-battle formes (Aegislash-Blade, Mimikyu-Busted) and cosmetic formes
 * (Vivillon, Alcremie) all learn what the forme they change from, or else their base
 * species, learns. Reading only the forme's own entry left 124 of them with no moves.
 * This mirrors `learnsetParent` in Showdown's sim/dex-species.ts, walking up until a
 * learnset turns up (Floette-Mega -> Floette-Eternal).
 */
function learnsetSource(dex, species) {
  let s = species;
  while (!dex.species.getLearnsetData(s.id).learnset && s.forme) {
    s = dex.species.get(s.changesFrom || s.baseSpecies);
  }
  return s;
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
    learnset: learnsetOf(dex, species, warnings),
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
    display_name: move.name,
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
function itemRecord(item, text) {
  return {
    name: slug(item.name),
    display_name: item.name,
    category: item.megaStone ? 'mega-stone' : item.isBerry ? 'berry' : 'other',
    fling_power: item.fling ? item.fling.basePower : null,
    description: describe(item, text.Items) || null,
  };
}

function abilityRecord(ability, text) {
  return { name: slug(ability.name), display_name: ability.name, description: describe(ability, text.Abilities) };
}

/**
 * Short description, or null. npm 0.11.11 puts text on the data object; master moved it
 * to `data/text/` behind `dex.loadTextData()`, which resolves per mod, so Champions-only
 * text (Piercing Drill's 1/4 damage) survives. Reading only the object would null out
 * every description on master.
 */
function describe(entry, table) {
  if (entry.shortDesc) return entry.shortDesc;
  const t = table && table[entry.id];
  return (t && (t.shortDesc || t.desc)) || null;
}

/** Text tables for `describe`, or empty ones where the dex has no `loadTextData`. */
function textOf(dex) {
  return typeof dex.loadTextData === 'function' ? dex.loadTextData() : { Abilities: {}, Items: {} };
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

function buildSnapshot(dex, spec, source, warnings) {
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

  let moves = legalMoves(dex);
  if (spec.learnedMovesOnly) {
    const learned = new Set(pokemon.flatMap((p) => (p.learnset || []).map((e) => e.move)));
    const dropped = moves.filter((m) => !learned.has(slug(m.name)));
    moves = moves.filter((m) => learned.has(slug(m.name)));
    warnings.push(`dropped ${dropped.length} legal moves no species learns: ${dropped.map((m) => m.id).join(', ')}`);
  }

  const text = textOf(dex);
  return {
    regulation: spec.regulation,
    // Not read by ingest (serde ignores unknown fields); here so a committed snapshot
    // says what produced it.
    source,
    abilities: dex.abilities.all().filter((a) => !a.isNonstandard).map((a) => abilityRecord(a, text)),
    moves: moves.map(moveRecord),
    pokemon,
    items: dex.items.all().filter((i) => !i.isNonstandard).map((i) => itemRecord(i, text)),
  };
}

/**
 * Provenance, and a guard against exporting a regulation from the wrong Showdown: the
 * same mod name means different regulations in npm 0.11.11 and on master.
 */
function sourceOf(spec) {
  if (spec.source === 'npm') {
    if (process.env.SHOWDOWN_PATH) fail(`${spec.regulation} comes from npm; unset SHOWDOWN_PATH`);
    return { npm: 'pokemon-showdown', version: require('pokemon-showdown/package.json').version, mod: spec.mod };
  }
  const sha = process.env.SHOWDOWN_SHA;
  if (!process.env.SHOWDOWN_PATH || !/^[0-9a-f]{40}$/.test(sha || '')) {
    fail(`${spec.regulation} comes from Showdown master; set SHOWDOWN_PATH and a full SHOWDOWN_SHA (see export.sh)`);
  }
  return { repo: SHOWDOWN_REPO, sha, mod: spec.mod };
}

function fail(message) {
  console.error(message);
  process.exit(1);
}

function main() {
  const key = process.argv[2];
  const outDir = process.argv[3] || path.join(__dirname, 'out');

  const spec = EXPORTS[key];
  if (!spec) fail(`usage: showdown-snapshot.js <${Object.keys(EXPORTS).join('|')}> [outDir]`);
  const source = sourceOf(spec);
  if (!Dex.dexes[spec.mod]) fail(`${SHOWDOWN} has no mod '${spec.mod}'`);

  const dex = Dex.mod(spec.mod);
  const warnings = [];
  warnings.inherited = [];
  const snapshot = buildSnapshot(dex, spec, source, warnings);
  const empty = snapshot.pokemon.filter((p) => p.learnset && !p.learnset.length).map((p) => p.name);

  fs.mkdirSync(outDir, { recursive: true });
  write(outDir, `snapshot.${slug(spec.regulation)}.json`, snapshot);
  write(outDir, 'typechart-check.json', typeChartRows(dex));

  console.log(`\n${spec.mod} -> ${spec.regulation}  (${source.sha || source.version})`);
  console.log(`  pokemon   ${snapshot.pokemon.length}`);
  console.log(`  moves     ${snapshot.moves.length}`);
  console.log(`  abilities ${snapshot.abilities.length}`);
  console.log(`  items     ${snapshot.items.length}`);
  console.log(`  learnset  ${snapshot.pokemon.reduce((n, p) => n + (p.learnset ? p.learnset.length : 0), 0)}`);
  console.log(`  learnset from another forme: ${warnings.inherited.length} (${warnings.inherited.join(', ')})`);
  if (empty.length) console.log(`  WARNING no moves at all: ${empty.join(', ')}`);
  for (const w of warnings) console.log(`  note: ${w}`);
}

function write(dir, file, data) {
  const target = path.join(dir, file);
  fs.writeFileSync(target, JSON.stringify(data, null, 1));
  const kb = (fs.statSync(target).size / 1024).toFixed(0);
  console.log(`${file.padEnd(34)} ${kb.padStart(6)} KB`);
}

main();
