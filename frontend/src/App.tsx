import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import {
  canPlayHevc,
  getVideo,
  isActive,
  listVideos,
  MESSAGE_ROI,
  originalUrl,
  previewUrl,
  snapshotUrl,
  transcribe,
  uploadVideo,
  type Message,
  type Video,
  type VideoDetail,
} from "./api";
import Pokedex from "./Pokedex";

const clock = (t: number) => {
  const m = Math.floor(t / 60);
  return `${String(m).padStart(2, "0")}:${(t - m * 60).toFixed(2).padStart(5, "0")}`;
};

const duration = (s: number) => {
  s = Math.max(0, Math.round(s));
  return `${Math.floor(s / 60)}:${String(s % 60).padStart(2, "0")}`;
};

const size = (bytes: number) =>
  bytes >= 1e9 ? `${(bytes / 1e9).toFixed(2)} GB` : `${(bytes / 1e6).toFixed(1)} MB`;

const date = (ms: number) =>
  new Date(ms).toLocaleString(undefined, { dateStyle: "medium", timeStyle: "short" });

/** The URL hash, kept current. `#pokedex...` is the pokedex; anything else is a video id. */
function useHash() {
  const [hash, setHash] = useState(() => location.hash.slice(1));
  useEffect(() => {
    const on = () => setHash(location.hash.slice(1));
    addEventListener("hashchange", on);
    return () => removeEventListener("hashchange", on);
  }, []);
  return hash;
}

export default function App() {
  const hash = useHash();
  const [error, setError] = useState<string | null>(null);
  const dex = hash === "pokedex" || hash.startsWith("pokedex/");

  return (
    <div className="app">
      <header>
        <h1>{dex ? "Pokédex" : "Battle Transcripts"}</h1>
        <nav className="tabs">
          <a className={dex ? "" : "on"} href="#">
            Transcripts
          </a>
          <a className={dex ? "on" : ""} href="#pokedex">
            Pokédex
          </a>
        </nav>
      </header>
      {error && (
        <div className="error" onClick={() => setError(null)}>
          {error}
        </div>
      )}
      {dex ? <Pokedex route={hash} onError={setError} /> : <Transcripts onError={setError} />}
    </div>
  );
}

function Transcripts({ onError: setError }: { onError: (e: string) => void }) {
  const [videos, setVideos] = useState<Video[]>([]);
  // The selected video lives in the URL hash, so a page reload or a shared link reopens it.
  const [selected, setSelectedState] = useState<string | null>(() => location.hash.slice(1) || null);
  const setSelected = useCallback((id: string | null) => {
    setSelectedState(id);
    history.replaceState(null, "", id ? `#${id}` : location.pathname);
  }, []);

  const refresh = useCallback(async () => {
    try {
      setVideos(await listVideos());
    } catch (e) {
      setError(String((e as Error).message));
    }
  }, [setError]);

  useEffect(() => {
    refresh();
  }, [refresh]);

  // Keep the history list's status badges current while anything is running.
  const anyActive = videos.some((v) => isActive(v.status));
  useEffect(() => {
    if (!anyActive) return;
    const t = setInterval(refresh, 3000);
    return () => clearInterval(t);
  }, [anyActive, refresh]);

  return (
    <div className="layout">
      <aside>
        <Upload
          onUploaded={(v) => {
            refresh();
            setSelected(v.id);
          }}
          onError={setError}
        />
        <h2>History</h2>
        {videos.length === 0 && <p className="muted">No videos yet.</p>}
        <ul className="history">
          {videos.map((v) => (
            <li key={v.id}>
              <button className={v.id === selected ? "selected" : ""} onClick={() => setSelected(v.id)}>
                <span className="name">{v.name}</span>
                <span className="meta">
                  {date(v.uploaded_at)} · <Badge status={v.status} />
                </span>
              </button>
            </li>
          ))}
        </ul>
      </aside>
      <main>
        {selected ? (
          <Detail key={selected} id={selected} onChanged={refresh} onError={setError} />
        ) : (
          <p className="muted placeholder">Upload a recording, or pick one from the history.</p>
        )}
      </main>
    </div>
  );
}

function Badge({ status }: { status: Video["status"] }) {
  return <span className={`badge ${status}`}>{status}</span>;
}

function Upload({ onUploaded, onError }: { onUploaded: (v: Video) => void; onError: (e: string) => void }) {
  const [file, setFile] = useState<File | null>(null);
  const [progress, setProgress] = useState<number | null>(null);
  const input = useRef<HTMLInputElement>(null);

  const upload = async () => {
    if (!file) return;
    setProgress(0);
    try {
      const v = await uploadVideo(file, setProgress);
      setFile(null);
      if (input.current) input.current.value = "";
      onUploaded(v);
    } catch (e) {
      onError(String((e as Error).message));
    } finally {
      setProgress(null);
    }
  };

  return (
    <section className="card upload">
      <h2>Upload a recording</h2>
      <input
        ref={input}
        type="file"
        accept="video/*"
        disabled={progress !== null}
        onChange={(e) => setFile(e.target.files?.[0] ?? null)}
      />
      {file && <p className="muted">{size(file.size)}</p>}
      {progress !== null ? (
        <Bar fraction={progress} label={`Uploading… ${Math.round(progress * 100)}%`} />
      ) : (
        <button className="primary" disabled={!file} onClick={upload}>
          Upload
        </button>
      )}
    </section>
  );
}

function Bar({ fraction, label }: { fraction: number | null; label: string }) {
  return (
    <div className="bar" role="progressbar" aria-valuenow={Math.round((fraction ?? 0) * 100)}>
      <div className="fill" style={{ width: `${(fraction ?? 0) * 100}%` }} />
      <span>{label}</span>
    </div>
  );
}

function Detail({ id, onChanged, onError }: { id: string; onChanged: () => void; onError: (e: string) => void }) {
  const [detail, setDetail] = useState<VideoDetail | null>(null);
  const player = useRef<HTMLVideoElement>(null);
  /** Playback position, for highlighting the message on screen. */
  const [now, setNow] = useState<number | null>(null);
  /** While transcribing: show the transcriber's live view (true) or the playable video. */
  const [follow, setFollow] = useState(true);
  const hevc = useMemo(canPlayHevc, []);

  const load = useCallback(async () => {
    try {
      setDetail(await getVideo(id));
    } catch (e) {
      onError(String((e as Error).message));
    }
  }, [id, onError]);

  useEffect(() => {
    load();
  }, [load]);

  // Poll once a second while this video is queued or transcribing; refresh the history
  // when it settles so its badge updates too.
  const active = detail ? isActive(detail.video.status) : false;
  useEffect(() => {
    if (!active) return;
    setFollow(true);
    const t = setInterval(load, 1000);
    return () => {
      clearInterval(t);
      onChanged();
    };
  }, [active, load, onChanged]);

  if (!detail) return <p className="muted">Loading…</p>;
  const { video, live, messages } = detail;

  const start = async () => {
    try {
      await transcribe(id);
      await load();
      onChanged();
    } catch (e) {
      onError(String((e as Error).message));
    }
  };

  const f = live?.fraction ?? null;
  const eta = f && f > 0.02 && live ? (live.elapsed * (1 - f)) / f : null;

  // The preview is made during transcription; HEVC-capable browsers can play the original
  // before that. A preview from an earlier run stays playable during a re-transcription.
  const src = video.has_preview ? previewUrl(id) : hevc ? originalUrl(id) : null;
  const showLive = video.status === "transcribing" && (follow || !src);

  const seek = (t: number) => {
    const v = player.current;
    if (!v) return;
    v.currentTime = t;
    setNow(t);
  };

  return (
    <section>
      <div className="detail-head">
        <div>
          <h2 className="title">{video.name}</h2>
          <p className="muted">
            Uploaded {date(video.uploaded_at)} · {size(video.size)} · <Badge status={video.status} />
          </p>
        </div>
        <button className="primary" disabled={active} onClick={start}>
          {video.status === "done" || video.status === "failed" ? "Transcribe again" : "Transcribe"}
        </button>
      </div>

      {video.status === "queued" && <p className="muted">Waiting for another transcription to finish…</p>}
      {video.status === "transcribing" && (
        <Bar
          fraction={f}
          label={
            f === null
              ? "Starting…"
              : `${Math.round(f * 100)}% · ${live?.progress?.messages ?? 0} messages` +
                (eta !== null ? ` · about ${duration(eta)} left` : "")
          }
        />
      )}
      {video.status === "failed" && <div className="error">Transcription failed: {video.error}</div>}
      {video.status === "done" && video.finished_at && video.started_at && (
        <p className="muted">
          Transcribed {date(video.finished_at)} in {duration((video.finished_at - video.started_at) / 1000)} ·{" "}
          {messages.length} messages
        </p>
      )}

      {video.status === "transcribing" && src && (
        <div className="toggle">
          <button className={follow ? "on" : ""} onClick={() => setFollow(true)}>
            Follow transcription
          </button>
          <button className={follow ? "" : "on"} onClick={() => setFollow(false)}>
            Watch video
          </button>
        </div>
      )}

      {showLive ? (
        <LiveView id={id} t={live?.progress?.t ?? null} frames={live?.progress?.frames ?? 0} />
      ) : src ? (
        <video
          ref={player}
          className="player"
          src={src}
          controls
          preload="metadata"
          onTimeUpdate={(e) => setNow(e.currentTarget.currentTime)}
          onSeeked={(e) => setNow(e.currentTarget.currentTime)}
        />
      ) : (
        <div className="player placeholder-box">
          <p>This browser can’t play the original recording (HEVC).</p>
          <p className="muted">A playable copy is made during transcription — press Transcribe to create it.</p>
        </div>
      )}

      {messages.length > 0 && (
        <Transcript messages={messages} now={showLive ? null : now} onSeek={!showLive && src ? seek : null} />
      )}
    </section>
  );
}

/** The frame the transcriber has reached, with the message line it reads outlined. */
function LiveView({ id, t, frames }: { id: string; t: number | null; frames: number }) {
  const [ready, setReady] = useState(false);
  return (
    <div className="player live">
      <img
        src={snapshotUrl(id, frames)}
        alt=""
        onLoad={() => setReady(true)}
        onError={() => setReady(false)}
        style={{ visibility: ready ? "visible" : "hidden" }}
      />
      {ready && (
        <div
          className="roi"
          style={{
            left: `${MESSAGE_ROI.left * 100}%`,
            top: `${MESSAGE_ROI.top * 100}%`,
            width: `${MESSAGE_ROI.width * 100}%`,
            height: `${MESSAGE_ROI.height * 100}%`,
          }}
        />
      )}
      <span className="caption">{t === null ? "Starting…" : `Reading ${clock(t)}`}</span>
    </div>
  );
}

/** Index of the message on screen at time `t`: the last one started, while it or a short tail is showing. */
function activeIndex(messages: Message[], t: number | null): number {
  if (t === null) return -1;
  let i = -1;
  for (let k = 0; k < messages.length && messages[k].t0 <= t; k++) i = k;
  return i >= 0 && t <= messages[i].t1 + 1 ? i : -1;
}

function Transcript({
  messages,
  now,
  onSeek,
}: {
  messages: Message[];
  now: number | null;
  onSeek: ((t: number) => void) | null;
}) {
  const current = activeIndex(messages, now);
  const list = useRef<HTMLOListElement>(null);
  useEffect(() => {
    if (current < 0) return;
    list.current?.children[current]?.scrollIntoView({ block: "nearest" });
  }, [current]);

  return (
    <ol className="transcript" ref={list}>
      {messages.map((m, i) => (
        <li
          key={i}
          className={[m.clean ? "" : "unclear", i === current ? "current" : ""].join(" ")}
          title={`confidence ${m.conf.toFixed(2)}`}
        >
          {onSeek ? (
            <button className="time" onClick={() => onSeek(m.t0)} title="Jump to this moment">
              {clock(m.t0)}
            </button>
          ) : (
            <time>{clock(m.t0)}</time>
          )}
          <span>{m.text}</span>
          {!m.clean && <em>unclear</em>}
        </li>
      ))}
    </ol>
  );
}
