import { useEffect, useMemo, useState, type ReactNode } from "react";
import {
  getPokemon,
  listItems,
  listMoves,
  listPokemon,
  listRegulations,
  type DexItem,
  type DexMove,
  type DexPokemon,
  type DexPokemonDetail,
  type Regulation,
} from "./api";

// Routes live in the URL hash: #pokedex, #pokedex/moves, #pokedex/items, #pokedex/pokemon/<id>.

/** "close-combat" -> "Close Combat". The stored slug stays visible in the tooltip. */
const pretty = (slug: string) => slug.replace(/(^|-)([a-z0-9])/g, (_, sep, c) => (sep ? " " : "") + c.toUpperCase());

const multiplier = (pct: number) => ({ 0: "0×", 25: "¼×", 50: "½×", 100: "1×", 200: "2×", 400: "4×" })[pct] ?? `${pct / 100}×`;

export default function Pokedex({ route, onError }: { route: string; onError: (e: string) => void }) {
  const [regulations, setRegulations] = useState<Regulation[]>([]);
  // null until regulations load; then the current (latest) one unless the user picks another.
  const [reg, setReg] = useState<number | null>(null);
  const [filter, setFilter] = useState("");

  useEffect(() => {
    listRegulations()
      .then((rs) => {
        setRegulations(rs);
        setReg((r) => r ?? rs[rs.length - 1]?.id ?? null);
      })
      .catch((e) => onError(String(e.message)));
  }, [onError]);

  const parts = route.split("/").slice(1); // drop "pokedex"
  const tab = parts[0] === "moves" || parts[0] === "items" ? parts[0] : "pokemon";
  const pokemonId = parts[0] === "pokemon" && parts[1] ? Number(parts[1]) : null;

  if (regulations.length === 0) return <p className="muted placeholder">Loading…</p>;
  const current = regulations.find((r) => r.id === reg);

  return (
    <div className="dex">
      <div className="dex-bar">
        <nav className="toggle">
          <a className={tab === "pokemon" ? "on" : ""} href="#pokedex">Pokémon</a>
          <a className={tab === "moves" ? "on" : ""} href="#pokedex/moves">Moves</a>
          <a className={tab === "items" ? "on" : ""} href="#pokedex/items">Items</a>
        </nav>
        <label>
          Regulation{" "}
          <select value={reg ?? ""} onChange={(e) => setReg(Number(e.target.value))}>
            {regulations.map((r) => (
              <option key={r.id} value={r.id}>
                {r.name}
              </option>
            ))}
          </select>
        </label>
        {current && (
          <span className="muted">
            {current.effective_from} → {current.effective_to ?? "current"}
          </span>
        )}
        {pokemonId === null && (
          <input
            className="search"
            type="search"
            placeholder="Filter…"
            value={filter}
            onChange={(e) => setFilter(e.target.value)}
          />
        )}
      </div>

      {reg === null ? null : pokemonId !== null ? (
        <PokemonDetail key={`${pokemonId}-${reg}`} id={pokemonId} reg={reg} onError={onError} />
      ) : tab === "moves" ? (
        <MovesTable reg={reg} filter={filter} onError={onError} />
      ) : tab === "items" ? (
        <ItemsTable reg={reg} filter={filter} onError={onError} />
      ) : (
        <PokemonTable reg={reg} filter={filter} onError={onError} />
      )}
    </div>
  );
}

/** Fetch whenever `reg` changes; null while loading. */
function useLoad<T>(load: (reg: number) => Promise<T>, reg: number, onError: (e: string) => void): T | null {
  const [data, setData] = useState<T | null>(null);
  useEffect(() => {
    let live = true;
    setData(null);
    load(reg)
      .then((d) => live && setData(d))
      .catch((e) => onError(String(e.message)));
    return () => {
      live = false;
    };
  }, [load, reg, onError]);
  return data;
}

// ---------------------------------------------------------------- sortable table

interface Column<T> {
  key: string;
  label: string;
  /** Value to sort by; omit for an unsortable column. */
  sort?: (row: T) => string | number | null;
  render: (row: T) => ReactNode;
  num?: boolean;
}

function Table<T extends { id: number }>({ rows, columns, initial }: { rows: T[]; columns: Column<T>[]; initial: string }) {
  const [by, setBy] = useState(initial);
  const [desc, setDesc] = useState(false);
  const sorted = useMemo(() => {
    const col = columns.find((c) => c.key === by);
    if (!col?.sort) return rows;
    const get = col.sort;
    // Nulls last in either direction.
    return [...rows].sort((a, b) => {
      const x = get(a);
      const y = get(b);
      if (x === y) return 0;
      if (x === null) return 1;
      if (y === null) return -1;
      return (x < y ? -1 : 1) * (desc ? -1 : 1);
    });
  }, [rows, columns, by, desc]);

  return (
    <div className="table-wrap">
      <table className="dex-table">
        <thead>
          <tr>
            {columns.map((c) => (
              <th
                key={c.key}
                className={[c.num ? "num" : "", c.sort ? "sortable" : "", c.key === by ? "sorted" : ""].join(" ")}
                onClick={() => {
                  if (!c.sort) return;
                  if (c.key === by) setDesc(!desc);
                  else {
                    setBy(c.key);
                    setDesc(!!c.num); // numbers default to highest first
                  }
                }}
              >
                {c.label}
                {c.key === by && (desc ? " ▼" : " ▲")}
              </th>
            ))}
          </tr>
        </thead>
        <tbody>
          {sorted.map((r) => (
            <tr key={r.id}>
              {columns.map((c) => (
                <td key={c.key} className={c.num ? "num" : ""}>
                  {c.render(r)}
                </td>
              ))}
            </tr>
          ))}
        </tbody>
      </table>
    </div>
  );
}

const Type = ({ name }: { name: string | null }) =>
  name ? <span className={`type t-${name}`}>{pretty(name)}</span> : <span className="muted">—</span>;

const Types = ({ types }: { types: string[] | null }) => (
  <span className="types">{types?.map((t) => <Type key={t} name={t} />) ?? <span className="muted">none</span>}</span>
);

const dash = (v: number | null | undefined) => (v === null || v === undefined ? <span className="muted">—</span> : v);

const matches = (filter: string, ...fields: (string | null | undefined)[]) => {
  const f = filter.trim().toLowerCase();
  return !f || fields.some((x) => x?.toLowerCase().includes(f));
};

// ---------------------------------------------------------------- pokemon

const STATS = [
  ["hp", "HP"],
  ["atk", "Atk"],
  ["def", "Def"],
  ["spa", "SpA"],
  ["spd", "SpD"],
  ["spe", "Spe"],
  ["bst", "BST"],
] as const;

function PokemonTable({ reg, filter, onError }: { reg: number; filter: string; onError: (e: string) => void }) {
  const rows = useLoad(listPokemon, reg, onError);
  const columns: Column<DexPokemon>[] = useMemo(
    () => [
      { key: "dex", label: "#", num: true, sort: (p) => p.dex * 1000 + (p.form ? 1 : 0), render: (p) => p.dex },
      {
        key: "name",
        label: "Name",
        sort: (p) => p.name,
        render: (p) => (
          <a href={`#pokedex/pokemon/${p.id}`}>
            {p.name}
            {p.variant_kind && <span className="tag">{p.variant_kind}</span>}
          </a>
        ),
      },
      { key: "types", label: "Types", sort: (p) => p.types?.join("/") ?? null, render: (p) => <Types types={p.types} /> },
      ...STATS.map(([k, label]) => ({
        key: k,
        label,
        num: true,
        sort: (p: DexPokemon) => p[k],
        render: (p: DexPokemon) => (k === "bst" ? <b>{p[k]}</b> : p[k]),
      })),
      {
        key: "abilities",
        label: "Abilities",
        render: (p) => (
          <span className="small">
            {p.abilities?.map(pretty).join(" · ") ?? <span className="muted">none</span>}
          </span>
        ),
      },
      { key: "moves", label: "Moves", num: true, sort: (p) => p.moves, render: (p) => p.moves },
      { key: "from", label: "Stats from", sort: (p) => p.stats_from, render: (p) => <span className="small muted">{p.stats_from}</span> },
    ],
    [],
  );
  if (!rows) return <p className="muted">Loading…</p>;
  const shown = rows.filter((p) => matches(filter, p.name, p.form, ...(p.types ?? []), ...(p.abilities ?? [])));
  return (
    <>
      <p className="muted">
        {shown.length} of {rows.length} Pokémon in this regulation
      </p>
      <Table rows={shown} columns={columns} initial="dex" />
    </>
  );
}

function PokemonDetail({ id, reg, onError }: { id: number; reg: number; onError: (e: string) => void }) {
  const [p, setP] = useState<DexPokemonDetail | null>(null);
  useEffect(() => {
    getPokemon(id, reg)
      .then(setP)
      .catch((e) => onError(String(e.message)));
  }, [id, reg, onError]);
  if (!p) return <p className="muted">Loading…</p>;

  const byMult = new Map<number, string[]>();
  for (const d of p.defense ?? []) byMult.set(d.pct, [...(byMult.get(d.pct) ?? []), d.type]);

  return (
    <section className="dex-detail">
      <p>
        <a href="#pokedex">← All Pokémon</a>
      </p>
      <div className="detail-head">
        <div>
          <h2 className="title">
            #{p.dex} {p.name}
          </h2>
          <p className="muted">
            form_slug “{p.form}” · id {p.id}
            {p.height_dm !== null && ` · ${(p.height_dm / 10).toFixed(1)} m`}
            {p.weight_hg !== null && ` · ${(p.weight_hg / 10).toFixed(1)} kg`}
          </p>
          <Types types={p.types} />
        </div>
      </div>

      {!p.stats && <div className="error">Not in this regulation: no stat row resolves here.</div>}

      {p.variant && (
        <p>
          {pretty(p.variant.kind)} of <a href={`#pokedex/pokemon/${p.variant.base.id}`}>{p.variant.base.name}</a>
          {p.variant.required_item && <> · needs {p.variant.required_item}</>}
        </p>
      )}
      {p.variants && (
        <p>
          Variants:{" "}
          {p.variants.map((v, i) => (
            <span key={v.id}>
              {i > 0 && " · "}
              <a href={`#pokedex/pokemon/${v.id}`}>{v.name}</a> <span className="muted">({v.kind})</span>
            </span>
          ))}
        </p>
      )}

      <div className="dex-grid">
        <div className="card">
          <h3>Base stats</h3>
          {p.stats ? (
            <>
              <table className="stats">
                <tbody>
                  {STATS.map(([k, label]) => (
                    <tr key={k} className={k === "bst" ? "total" : ""}>
                      <th>{label}</th>
                      <td className="num">{p.stats![k]}</td>
                      <td className="bar-cell">
                        {k !== "bst" && <div className="stat-bar" style={{ width: `${(p.stats![k] / 255) * 100}%` }} />}
                      </td>
                    </tr>
                  ))}
                </tbody>
              </table>
              <p className="muted small">Resolved from the {p.stats.from} row.</p>
            </>
          ) : (
            <p className="muted">—</p>
          )}
          {p.stat_rows && (
            <>
              <h3>Stored stat rows</h3>
              <table className="dex-table compact">
                <thead>
                  <tr>
                    <th>Regulation</th>
                    {STATS.map(([k, label]) => (
                      <th key={k} className="num">
                        {label}
                      </th>
                    ))}
                  </tr>
                </thead>
                <tbody>
                  {p.stat_rows.map((r) => (
                    <tr key={r.regulation}>
                      <td>{r.regulation}</td>
                      {STATS.map(([k]) => (
                        <td key={k} className="num">
                          {r[k]}
                        </td>
                      ))}
                    </tr>
                  ))}
                </tbody>
              </table>
            </>
          )}
        </div>

        <div className="card">
          <h3>Abilities</h3>
          {p.abilities ? (
            <dl className="abilities">
              {p.abilities.map((a) => (
                <div key={a.slot}>
                  <dt title={a.name}>
                    {pretty(a.name)} <span className="tag">{a.slot}</span>
                  </dt>
                  <dd className="muted small">{a.description ?? "no description"}</dd>
                </div>
              ))}
            </dl>
          ) : (
            <p className="muted">none</p>
          )}

          <h3>Damage taken</h3>
          {byMult.size === 0 ? (
            <p className="muted">—</p>
          ) : (
            <table className="matchups">
              <tbody>
                {[400, 200, 100, 50, 25, 0]
                  .filter((m) => byMult.has(m))
                  .map((m) => (
                    <tr key={m}>
                      <th>{multiplier(m)}</th>
                      <td>
                        <span className="types">
                          {byMult.get(m)!.map((t) => (
                            <Type key={t} name={t} />
                          ))}
                        </span>
                      </td>
                    </tr>
                  ))}
              </tbody>
            </table>
          )}
        </div>
      </div>

      <h3>Learnset ({p.learnset?.length ?? 0})</h3>
      {p.learnset ? (
        <Table
          rows={p.learnset.map((m, i) => ({ ...m, id: i }))}
          initial="name"
          columns={[
            { key: "name", label: "Move", sort: (m) => m.name, render: (m) => <span title={m.name}>{pretty(m.name)}</span> },
            { key: "type", label: "Type", sort: (m) => m.type, render: (m) => <Type name={m.type} /> },
            { key: "class", label: "Class", sort: (m) => m.class, render: (m) => m.class ?? "—" },
            { key: "power", label: "Power", num: true, sort: (m) => m.power, render: (m) => dash(m.power) },
            { key: "acc", label: "Acc", num: true, sort: (m) => m.accuracy, render: (m) => dash(m.accuracy) },
            { key: "pp", label: "PP", num: true, sort: (m) => m.pp, render: (m) => dash(m.pp) },
            { key: "prio", label: "Prio", num: true, sort: (m) => m.priority, render: (m) => dash(m.priority) },
            {
              key: "method",
              label: "Method",
              sort: (m) => m.method,
              render: (m) => (
                <span className="small muted">
                  {m.method}
                  {m.level > 0 && ` ${m.level}`}
                </span>
              ),
            },
          ]}
        />
      ) : (
        <p className="muted">none</p>
      )}
    </section>
  );
}

// ---------------------------------------------------------------- moves

function MovesTable({ reg, filter, onError }: { reg: number; filter: string; onError: (e: string) => void }) {
  const rows = useLoad(listMoves, reg, onError);
  const columns: Column<DexMove>[] = useMemo(
    () => [
      { key: "name", label: "Move", sort: (m) => m.name, render: (m) => <span title={m.name}>{pretty(m.name)}</span> },
      { key: "type", label: "Type", sort: (m) => m.type, render: (m) => <Type name={m.type} /> },
      { key: "class", label: "Class", sort: (m) => m.class, render: (m) => m.class },
      { key: "power", label: "Power", num: true, sort: (m) => m.power, render: (m) => dash(m.power) },
      { key: "acc", label: "Acc", num: true, sort: (m) => m.accuracy, render: (m) => dash(m.accuracy) },
      { key: "pp", label: "PP", num: true, sort: (m) => m.pp, render: (m) => dash(m.pp) },
      { key: "prio", label: "Prio", num: true, sort: (m) => m.priority, render: (m) => m.priority },
      { key: "chance", label: "Chance", num: true, sort: (m) => m.effect_chance, render: (m) => dash(m.effect_chance) },
      {
        key: "effect",
        label: "Secondary effect (stored)",
        render: (m) => (m.secondary_effect ? <code className="small">{m.secondary_effect}</code> : <span className="muted">—</span>),
      },
      { key: "learners", label: "Learners", num: true, sort: (m) => m.learners, render: (m) => m.learners },
      { key: "from", label: "Data from", sort: (m) => m.data_from, render: (m) => <span className="small muted">{m.data_from}</span> },
    ],
    [],
  );
  if (!rows) return <p className="muted">Loading…</p>;
  const shown = rows.filter((m) => matches(filter, m.name, pretty(m.name), m.type, m.class));
  return (
    <>
      <p className="muted">
        {shown.length} of {rows.length} moves in this regulation. Power and accuracy “—” mean
        variable/none and never misses.
      </p>
      <Table rows={shown} columns={columns} initial="name" />
    </>
  );
}

// ---------------------------------------------------------------- items

function ItemsTable({ reg, filter, onError }: { reg: number; filter: string; onError: (e: string) => void }) {
  const rows = useLoad(listItems, reg, onError);
  const [showIllegal, setShowIllegal] = useState(false);
  const columns: Column<DexItem>[] = useMemo(
    () => [
      { key: "name", label: "Item", sort: (i) => i.display_name, render: (i) => <span title={i.name}>{i.display_name}</span> },
      { key: "category", label: "Category", sort: (i) => i.category, render: (i) => i.category },
      { key: "legal", label: "Legal", sort: (i) => (i.legal ? 0 : 1), render: (i) => (i.legal ? "yes" : <b className="illegal">no</b>) },
      { key: "fling", label: "Fling", num: true, sort: (i) => i.fling_power, render: (i) => dash(i.fling_power) },
      { key: "desc", label: "Description", render: (i) => <span className="small">{i.description ?? "—"}</span> },
    ],
    [],
  );
  if (!rows) return <p className="muted">Loading…</p>;
  const legal = rows.filter((i) => i.legal).length;
  const shown = rows.filter((i) => (showIllegal || i.legal) && matches(filter, i.name, i.display_name, i.category));
  return (
    <>
      <p className="muted">
        {legal} legal in this regulation, {rows.length - legal} stored but not legal.{" "}
        <label>
          <input type="checkbox" checked={showIllegal} onChange={(e) => setShowIllegal(e.target.checked)} /> Show
          illegal items
        </label>
      </p>
      <Table rows={shown} columns={columns} initial="name" />
    </>
  );
}
