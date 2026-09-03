// Champions Pokédex export — dumps the Pokémon Showdown `champions` mod to flat JSON
// for the Rust ingester. See POKEDEX_DEVLOG.md (2026-09-03) for why this exists.
//
//   npm install pokemon-showdown        # verified against 0.11.11 / upstream 2f5b2739
//   node pokedex_data_export.js [outdir]
//
// Why the simulator instead of parsing data/mods/champions/*.ts directly: the mod is an
// overlay (base stats and the type chart are inherited from the root Gen 9 data, moves
// and learnsets are partial overrides). Letting the Dex resolve the inherit chain is the
// only way to get the merged, actually-correct values.
//
// Level-50 stats are deliberately NOT exported. They are a pure function of what is:
//   HP     = base_hp + SP_hp + 75
//   others = trunc((base + SP + 20) * nature_mod)     nature_mod in {0.9, 1.0, 1.1}
const fs = require('fs');
const {Dex} = require('pokemon-showdown');
const dex = Dex.mod('champions');
const OUT = process.argv[2] || (__dirname + '/out');
fs.mkdirSync(OUT, {recursive: true});
const legal = dex.species.all().filter(s => s.tier !== 'Illegal' && !s.isNonstandard);

const species = legal.map(s => ({
  id: s.id, num: s.num, name: s.name, base_species: s.baseSpecies, forme: s.forme || null,
  types: s.types, base_stats: s.baseStats,
  abilities: Object.values(s.abilities),
  required_item: s.requiredItem || null, is_mega: !!(s.requiredItem && s.forme?.startsWith('Mega')),
  tier: s.tier, weightkg: s.weightkg, heightm: s.heightm,
}));

const moves = dex.moves.all().filter(m => !m.isNonstandard).map(m => ({
  id: m.id, num: m.num, name: m.name, type: m.type, category: m.category,
  base_power: m.basePower, accuracy: m.accuracy === true ? null : m.accuracy,
  pp: m.pp, priority: m.priority, target: m.target,
  secondary: m.secondary || (m.secondaries?.length ? m.secondaries : null),
  flags: m.flags, short_desc: m.shortDesc,
}));

const learnsets = [];
for (const s of legal) {
  const ls = dex.species.getLearnsetData(s.id).learnset || {};
  for (const mid of Object.keys(ls)) if (dex.moves.get(mid).exists && !dex.moves.get(mid).isNonstandard) learnsets.push({species_id: s.id, move_id: mid, sources: ls[mid]});
}

const TYPES = dex.types.all().map(t => t.name);
const typechart = [];
for (const atk of TYPES) for (const def of TYPES) {
  let mult;
  if (!dex.getImmunity(atk, def)) mult = 0;
  else mult = Math.pow(2, dex.getEffectiveness(atk, def));
  typechart.push({attacking: atk, defending: def, multiplier: mult});
}

const natures = dex.natures.all().map(n => ({id:n.id, name:n.name, plus:n.plus||null, minus:n.minus||null}));
const abilities = dex.abilities.all().filter(a=>!a.isNonstandard).map(a => ({id:a.id, name:a.name, short_desc:a.shortDesc, rating:a.rating}));
const items = dex.items.all().filter(i=>!i.isNonstandard).map(i => ({id:i.id, name:i.name, short_desc:i.shortDesc, mega_stone:i.megaStone||null}));

const w=(f,o)=>{fs.writeFileSync(`${OUT}/${f}`, JSON.stringify(o,null,1)); console.log(f.padEnd(22), String(Array.isArray(o)?o.length:'-').padStart(6), 'rows', (fs.statSync(`${OUT}/${f}`).size/1024).toFixed(0)+'KB');};
w('species.json', species); w('moves.json', moves); w('learnsets.json', learnsets);
w('typechart.json', typechart); w('natures.json', natures); w('abilities.json', abilities); w('items.json', items);
console.log('\nnonzero typechart rows:', typechart.filter(r=>r.multiplier!==1).length);
console.log('moves with secondary:', moves.filter(m=>m.secondary).length);
console.log('megas:', species.filter(s=>s.is_mega).length, '| dual-type:', species.filter(s=>s.types.length===2).length, '| >2 types:', species.filter(s=>s.types.length>2).length);
