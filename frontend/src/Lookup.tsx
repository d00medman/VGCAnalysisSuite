// Hover cards for names in battle text: Pokémon, moves, items and abilities.
//
// Matching is deliberately naive: every known display name, longest first, case-sensitive,
// on word boundaries. Battle text capitalises names, so case-sensitivity is what keeps
// "protected itself" from matching Protect. Known misses: nicknames, and a name that is
// also an ordinary capitalised word at the start of a sentence.

import { useEffect, useState, type CSSProperties, type ReactNode } from "react";
import {
  listAbilities,
  listItems,
  listMoves,
  listPokemon,
  listRegulations,
  type DexAbility,
  type DexItem,
  type DexMove,
  type DexPokemon,
  type Regulation,
} from "./api";
import { STATS, Type } from "./Pokedex";

type Entry =
  | { kind: "pokemon"; p: DexPokemon }
  | { kind: "move"; m: DexMove }
  | { kind: "item"; i: DexItem }
  | { kind: "ability"; a: DexAbility };

export interface Lookup {
  regulation: Regulation;
  pattern: RegExp;
  /** Keyed by name with curly apostrophes straightened. */
  byText: Map<string, Entry[]>;
  abilityName: Map<string, string>;
}

const straighten = (s: string) => s.replace(/’/g, "'");
const escape = (s: string) => s.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
const slug = (s: string) =>
  s
    .toLowerCase()
    .replace(/['".]/g, "")
    .replace(/[^a-z0-9]+/g, "-")
    .replace(/^-+|-+$/g, "");

/** "Charizard-Mega-X" with form "mega-x" -> "Charizard": strip the suffix the form slug names. */
function baseName(p: DexPokemon): string {
  if (!p.form) return p.name;
  for (let i = p.name.indexOf("-"); i > 0; i = p.name.indexOf("-", i + 1)) {
    if (slug(p.name.slice(i + 1)) === p.form) return p.name.slice(0, i);
  }
  return p.name;
}

/** How battle text names a mega: "Charizard-Mega-X" -> "Mega Charizard X". */
function megaName(p: DexPokemon): string | null {
  const m = /^(.*)-Mega(?:-(\w+))?$/.exec(p.name);
  return m ? `Mega ${m[1]}${m[2] ? ` ${m[2]}` : ""}` : null;
}

export function build(
  regulation: Regulation,
  pokemon: DexPokemon[],
  moves: DexMove[],
  items: DexItem[],
  abilities: DexAbility[],
): Lookup {
  const byText = new Map<string, Entry[]>();
  const add = (text: string, e: Entry) => {
    const k = straighten(text);
    byText.set(k, [...(byText.get(k) ?? []), e]);
  };

  // Battle text calls every non-mega form by its species name ("Rotom", "Floette"), so the
  // species name points at the base form, or at the forms if the base form isn't legal.
  const bySpecies = new Map<string, DexPokemon[]>();
  for (const p of pokemon) {
    const mega = p.variant_kind === "mega" ? megaName(p) : null;
    if (mega) add(mega, { kind: "pokemon", p });
    else {
      const b = baseName(p);
      bySpecies.set(b, [...(bySpecies.get(b) ?? []), p]);
    }
  }
  for (const [name, forms] of bySpecies) {
    const base = forms.filter((p) => !p.form);
    for (const p of base.length ? base : forms) add(name, { kind: "pokemon", p });
  }
  for (const m of moves) add(m.display_name, { kind: "move", m });
  for (const i of items) add(i.display_name, { kind: "item", i });
  for (const a of abilities) add(a.display_name, { kind: "ability", a });

  // Longest first, so "Toxic Spikes" wins over "Toxic". Apostrophes match either form.
  const names = [...byText.keys()].sort((a, b) => b.length - a.length);
  const alt = names.map((n) => escape(n).replace(/'/g, "['’]")).join("|");
  return {
    regulation,
    pattern: new RegExp(`(?<![\\p{L}\\p{N}])(?:${alt})(?![\\p{L}\\p{N}])`, "gu"),
    byText,
    abilityName: new Map(abilities.map((a) => [a.name, a.display_name])),
  };
}

// One load per regulation for the whole page.
const cache = new Map<number, Promise<Lookup>>();
let regulations: Promise<Regulation[]> | null = null;

/** The regulation in force on a date (unix ms), else the latest. */
function regulationAt(rs: Regulation[], ms: number): Regulation {
  const day = new Date(ms).toISOString().slice(0, 10);
  return [...rs].reverse().find((r) => r.effective_from <= day) ?? rs[rs.length - 1];
}

/** Names known as of the regulation in force at `at` (unix ms); null while loading. */
export function useLookup(at: number): Lookup | null {
  const [lookup, setLookup] = useState<Lookup | null>(null);
  useEffect(() => {
    let live = true;
    regulations ??= listRegulations();
    regulations
      .then((rs) => {
        if (rs.length === 0) return null;
        const reg = regulationAt(rs, at);
        if (!cache.has(reg.id)) {
          cache.set(
            reg.id,
            Promise.all([listPokemon(reg.id), listMoves(reg.id), listItems(reg.id), listAbilities()]).then(
              ([p, m, i, a]) => build(reg, p, m, i, a),
            ),
          );
        }
        return cache.get(reg.id)!;
      })
      .then((l) => live && l && setLookup(l))
      // Hover cards are an extra; the transcript works without them.
      .catch((e) => console.warn("pokedex lookup unavailable:", e));
    return () => {
      live = false;
    };
  }, [at]);
  return lookup;
}

export interface Hover {
  entries: Entry[];
  rect: DOMRect;
}

/** `text` with every known name wrapped in a hoverable link to its Pokédex page. */
export function annotate(text: string, lookup: Lookup | null, onHover: (h: Hover | null) => void): ReactNode {
  if (!lookup) return text;
  const out: ReactNode[] = [];
  let last = 0;
  for (const m of text.matchAll(lookup.pattern)) {
    const entries = lookup.byText.get(straighten(m[0]));
    if (!entries) continue;
    const start = m.index!;
    if (start > last) out.push(text.slice(last, start));
    const first = entries[0];
    const href =
      first.kind === "pokemon"
        ? `#pokedex/pokemon/${first.p.id}`
        : first.kind === "move"
          ? "#pokedex/moves"
          : first.kind === "item"
            ? "#pokedex/items"
            : undefined;
    const show = (el: HTMLElement) => onHover({ entries, rect: el.getBoundingClientRect() });
    out.push(
      <a
        key={start}
        className={`term k-${first.kind}`}
        href={href}
        tabIndex={0}
        onMouseEnter={(e) => show(e.currentTarget)}
        onFocus={(e) => show(e.currentTarget)}
        onMouseLeave={() => onHover(null)}
        onBlur={() => onHover(null)}
      >
        {m[0]}
      </a>,
    );
    last = start + m[0].length;
  }
  if (last < text.length) out.push(text.slice(last));
  return out;
}

/** The floating card for the name under the pointer. */
export function HoverCard({ hover, lookup }: { hover: Hover | null; lookup: Lookup | null }) {
  if (!hover || !lookup) return null;
  const { rect } = hover;
  const below = rect.bottom < innerHeight * 0.6;
  const style: CSSProperties = {
    left: Math.max(8, Math.min(rect.left, innerWidth - 360)),
    ...(below ? { top: rect.bottom + 6 } : { bottom: innerHeight - rect.top + 6 }),
  };
  const shown = hover.entries.slice(0, 3);
  return (
    <div className="hovercard" style={style} role="tooltip">
      {shown.map((e, k) => (
        <div key={k} className="hc-entry">
          <Card e={e} abilityName={lookup.abilityName} />
        </div>
      ))}
      {hover.entries.length > shown.length && (
        <p className="muted small">+{hover.entries.length - shown.length} more forms</p>
      )}
      <p className="muted small hc-foot">as of {lookup.regulation.name}</p>
    </div>
  );
}

function Card({ e, abilityName }: { e: Entry; abilityName: Map<string, string> }) {
  switch (e.kind) {
    case "pokemon": {
      const p = e.p;
      return (
        <>
          <div className="hc-head">
            <b>{p.name}</b> <span className="muted">#{p.dex}</span>
            <span className="types">
              {p.types?.map((t) => <Type key={t} name={t} />)}
            </span>
          </div>
          <table className="hc-stats">
            <thead>
              <tr>
                {STATS.map(([k, label]) => (
                  <th key={k}>{label}</th>
                ))}
              </tr>
            </thead>
            <tbody>
              <tr>
                {STATS.map(([k]) => (
                  <td key={k} className={k === "bst" ? "total" : ""}>
                    {p[k]}
                  </td>
                ))}
              </tr>
            </tbody>
          </table>
          {p.abilities && (
            <p className="small">{p.abilities.map((a) => abilityName.get(a) ?? a).join(" · ")}</p>
          )}
        </>
      );
    }
    case "move": {
      const m = e.m;
      const v = (x: number | null) => (x === null ? "—" : x);
      return (
        <>
          <div className="hc-head">
            <b>{m.display_name}</b> <span className="tag">move</span> <Type name={m.type} />
            <span className="muted small">{m.class}</span>
          </div>
          <p className="small">
            Power {v(m.power)} · Acc {v(m.accuracy)} · PP {v(m.pp)}
            {m.priority !== 0 && ` · Priority ${m.priority > 0 ? "+" : ""}${m.priority}`}
            {m.effect_chance !== null && ` · Effect ${m.effect_chance}%`}
          </p>
        </>
      );
    }
    case "item":
      return (
        <>
          <div className="hc-head">
            <b>{e.i.display_name}</b> <span className="tag">item</span>
            <span className="muted small">{e.i.category}</span>
            {!e.i.legal && <b className="illegal small">not legal</b>}
          </div>
          {e.i.description && <p className="small">{e.i.description}</p>}
        </>
      );
    case "ability":
      return (
        <>
          <div className="hc-head">
            <b>{e.a.display_name}</b> <span className="tag">ability</span>
          </div>
          {e.a.description && <p className="small">{e.a.description}</p>}
        </>
      );
  }
}
