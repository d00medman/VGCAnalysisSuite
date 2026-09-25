// Client for the transcription API (../server). All paths are relative, so the same build
// works behind the Vite dev proxy and behind nginx in the container.

export type Status = "uploaded" | "queued" | "transcribing" | "done" | "failed";

export interface Video {
  id: string;
  name: string;
  size: number;
  uploaded_at: number;
  status: Status;
  started_at: number | null;
  finished_at: number | null;
  message_count: number | null;
  error: string | null;
  /** A browser-playable H.264 copy exists (made during transcription). */
  has_preview: boolean;
}

export interface Message {
  t0: number;
  t1: number;
  text: string;
  conf: number;
  /** False when the reader could not identify every character; `text` then contains `?`. */
  clean: boolean;
}

export interface Live {
  progress: { t: number; t_start: number; span: number | null; frames: number; messages: number } | null;
  fraction: number | null;
  elapsed: number;
}

export interface VideoDetail {
  video: Video;
  live: Live | null;
  messages: Message[];
}

export const isActive = (s: Status) => s === "queued" || s === "transcribing";

export const originalUrl = (id: string) => `/api/videos/${id}/file`;
export const previewUrl = (id: string) => `/api/videos/${id}/preview`;
/** Latest frame the transcriber has reached; `bust` forces a fresh fetch. */
export const snapshotUrl = (id: string, bust: number) => `/api/videos/${id}/live.jpg?f=${bust}`;

/** The message line the transcriber reads, as fractions of the 2436x1126 frame (analyzer MESSAGE_ROI). */
export const MESSAGE_ROI = { left: 500 / 2436, top: 740 / 1126, width: 1936 / 2436, height: 120 / 1126 };

/** iPhone recordings are HEVC; only some browsers (Safari, some GPU setups) can play them directly. */
export const canPlayHevc = () =>
  document.createElement("video").canPlayType('video/mp4; codecs="hvc1.1.6.L150.B0"') !== "";

async function json<T>(res: Response): Promise<T> {
  if (!res.ok) {
    const body = await res.json().catch(() => null);
    throw new Error(body?.error ?? `${res.status} ${res.statusText}`);
  }
  return res.json();
}

export const listVideos = () => fetch("/api/videos").then((r) => json<Video[]>(r));

export const getVideo = (id: string) => fetch(`/api/videos/${id}`).then((r) => json<VideoDetail>(r));

export const transcribe = (id: string) =>
  fetch(`/api/videos/${id}/transcribe`, { method: "POST" }).then((r) => json<Video>(r));

/** Upload the raw file. XHR rather than fetch, because fetch cannot report upload progress. */
export function uploadVideo(file: File, onProgress: (fraction: number) => void): Promise<Video> {
  return new Promise((resolve, reject) => {
    const xhr = new XMLHttpRequest();
    xhr.open("POST", `/api/videos?name=${encodeURIComponent(file.name)}`);
    xhr.upload.onprogress = (e) => e.lengthComputable && onProgress(e.loaded / e.total);
    xhr.onload = () => {
      let body: any = null;
      try {
        body = JSON.parse(xhr.responseText);
      } catch {
        /* non-JSON error page */
      }
      if (xhr.status >= 200 && xhr.status < 300) resolve(body);
      else reject(new Error(body?.error ?? `upload failed: ${xhr.status}`));
    };
    xhr.onerror = () => reject(new Error("upload failed: network error"));
    xhr.send(file);
  });
}

// ---------------------------------------------------------------- pokedex (read-only)
// Every call takes a regulation id; the server resolves stats, types, learnsets and
// legality as of that regulation. Names are stored slugs ("close-combat").

export interface Regulation {
  id: number;
  name: string;
  effective_from: string;
  /** Null for the current regulation. */
  effective_to: string | null;
}

export interface StatBlock {
  hp: number;
  atk: number;
  def: number;
  spa: number;
  spd: number;
  spe: number;
  bst: number;
}

export interface DexPokemon extends StatBlock {
  id: number;
  dex: number;
  form: string;
  name: string;
  variant_kind: string | null;
  /** Regulation whose stored stat row this resolves to. */
  stats_from: string;
  types: string[] | null;
  abilities: string[] | null;
  moves: number;
}

export interface DexMove {
  id: number;
  name: string;
  /** As the game prints it: "King's Shield". */
  display_name: string;
  type: string;
  class: "physical" | "special" | "status";
  power: number | null;
  accuracy: number | null;
  pp: number | null;
  priority: number;
  effect_chance: number | null;
  /** JSON of Showdown's secondaries, as stored. */
  secondary_effect: string | null;
  data_from: string;
  learners: number;
}

export interface DexItem {
  id: number;
  name: string;
  display_name: string;
  category: "mega-stone" | "berry" | "other";
  fling_power: number | null;
  description: string | null;
  legal: boolean;
}

export interface DexAbility {
  id: number;
  name: string;
  display_name: string;
  description: string | null;
}

export interface DexPokemonDetail {
  id: number;
  dex: number;
  form: string;
  name: string;
  height_dm: number | null;
  weight_hg: number | null;
  variant: { kind: string; required_item: string | null; base: { id: number; name: string } } | null;
  variants: { id: number; name: string; kind: string; required_item: string | null }[] | null;
  /** Null when the pokemon is not in this regulation. */
  stats: (StatBlock & { from: string }) | null;
  /** Every stored stat row, one per regulation where the stats changed. */
  stat_rows: (StatBlock & { regulation: string })[] | null;
  types: string[] | null;
  abilities: { slot: string; name: string; description: string | null }[] | null;
  defense: { type: string; pct: number }[] | null;
  learnset:
    | {
        name: string;
        method: string;
        level: number;
        type: string | null;
        class: string | null;
        power: number | null;
        accuracy: number | null;
        pp: number | null;
        priority: number | null;
      }[]
    | null;
}

const dex = <T,>(path: string, regulation: number | null) =>
  fetch(`/api/pokedex/${path}${regulation === null ? "" : `?regulation=${regulation}`}`).then((r) => json<T>(r));

export const listRegulations = () => fetch("/api/pokedex/regulations").then((r) => json<Regulation[]>(r));
export const listPokemon = (reg: number | null) => dex<DexPokemon[]>("pokemon", reg);
export const getPokemon = (id: number, reg: number | null) => dex<DexPokemonDetail>(`pokemon/${id}`, reg);
export const listMoves = (reg: number | null) => dex<DexMove[]>("moves", reg);
export const listItems = (reg: number | null) => dex<DexItem[]>("items", reg);
export const listAbilities = () => fetch("/api/pokedex/abilities").then((r) => json<DexAbility[]>(r));
