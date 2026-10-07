#!/usr/bin/env python3
"""Transcribe every recording in raw_recordings/ and its batch folders to sandbox/transcripts/,
mirroring the layout: raw_recordings/<batch>/<name>.MP4 → sandbox/transcripts/<batch>/<name>.txt
(gitignored).

    scripts/transcribe-all.py [--regenerate] [-j N] [batch...]

Safe to rerun from any directory: videos with an up-to-date transcript are skipped, so an
interrupted run picks up where it left off, with or without --regenerate. Delete a .txt to
redo that video. Each transcript is written to a .tmp file and renamed only once the
analyzer succeeds. Ctrl+C stops the run and keeps every transcript already finished.

The analyzer's speed experiments (ANALYZER_CROP_FIRST, ANALYZER_SKIP_NONREF,
ANALYZER_HWACCEL; see analyzer/src/decode.rs) pass through from the environment.
"""

import argparse
import os
import subprocess
import sys
import threading
import time
from concurrent.futures import ThreadPoolExecutor
from pathlib import Path

from transcripts import ANALYZER_DIR, RECORDINGS, REPO, TRANSCRIPTS, videos

PREVIOUS = REPO / "sandbox" / "transcripts-previous"
ANALYZER = ANALYZER_DIR / "target" / "release" / "analyzer"
# The reader's output depends on the binary and the atlas and lexicon it loads; a transcript
# older than any of them is stale.
READER_INPUTS = [ANALYZER, ANALYZER_DIR / "atlas" / "glyphs.txt", ANALYZER_DIR / "atlas" / "lexicon.txt"]

stop = threading.Event()
print_lock = threading.Lock()


def say(msg: str) -> None:
    with print_lock:
        print(msg, file=sys.stderr, flush=True)


def transcribe(video: Path, txt: Path, quiet: bool, children: set) -> bool | None:
    """Write `video`'s transcript to `txt`, keeping any previous version. True on success,
    False on failure, None if the run was interrupted."""
    rel = video.relative_to(RECORDINGS)
    tmp = txt.with_suffix(".txt.tmp")
    with tmp.open("w") as out:
        out.write(f"video: {video.name}\npath:  {video}\n\n")
        out.flush()
        cmd = [str(ANALYZER), *(["-q"] if quiet else []), "transcript", str(video)]
        proc = subprocess.Popen(cmd, stdout=out, stdin=subprocess.DEVNULL)
        children.add(proc)
        try:
            code = proc.wait()
        finally:
            children.discard(proc)
    if code != 0 or stop.is_set():
        tmp.unlink(missing_ok=True)
        # Ctrl+C reaches the analyzer too, often before this script sees it.
        return None if stop.is_set() or code < 0 else False
    if txt.exists():
        prev = PREVIOUS / rel.with_suffix(".txt")
        prev.parent.mkdir(parents=True, exist_ok=True)
        txt.replace(prev)
    tmp.replace(txt)
    return True


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    ap.add_argument("batches", nargs="*", help="batch folders, e.g. Batch0_2026-08-28 (default: all)")
    ap.add_argument("--regenerate", action="store_true",
                    help="also redo transcripts older than the analyzer, atlas or lexicon; "
                         "the previous version goes to sandbox/transcripts-previous/")
    ap.add_argument("-j", type=int, default=1, metavar="N",
                    help="transcribe N videos at once (default 1); per-video progress is then "
                         "suppressed, since it would interleave")
    args = ap.parse_args()
    if args.j < 1:
        ap.error("-j must be at least 1")

    if args.batches:
        folders = [RECORDINGS / b for b in args.batches]
        missing = [str(f) for f in folders if not f.is_dir()]
        if missing:
            ap.error(f"no batch folder: {', '.join(missing)}")
        files = [v for f in folders for v in videos(f)]
    else:
        files = videos(RECORDINGS) + [v for f in sorted(RECORDINGS.iterdir()) if f.is_dir() for v in videos(f)]

    subprocess.run(["cargo", "build", "--release", "--quiet", "--manifest-path",
                    str(ANALYZER_DIR / "Cargo.toml")], check=True)
    newest = max(p.stat().st_mtime for p in READER_INPUTS if p.exists())

    todo = []
    for n, video in enumerate(files, 1):
        txt = TRANSCRIPTS / video.relative_to(RECORDINGS).with_suffix(".txt")
        if txt.exists() and (not args.regenerate or txt.stat().st_mtime > newest):
            continue
        txt.parent.mkdir(parents=True, exist_ok=True)
        todo.append((n, video, txt))
    total = len(files)
    say(f"{len(todo)} to transcribe, {total - len(todo)} up to date")

    children: set = set()
    done, failed = [], []

    def one(item):
        n, video, txt = item
        if stop.is_set():
            return
        rel = video.relative_to(RECORDINGS)
        say(f"[{n}/{total}] {rel}")
        start = time.monotonic()
        result = transcribe(video, txt, quiet=args.j > 1, children=children)
        if result:
            done.append(rel)
            if args.j > 1:
                say(f"[{n}/{total}] done {rel} in {time.monotonic() - start:.0f}s")
        elif result is False:
            failed.append(rel)
            say(f"[{n}/{total}] FAILED {rel}")

    pool = ThreadPoolExecutor(max_workers=args.j)
    try:
        for f in [pool.submit(one, item) for item in todo]:
            f.result()
    except KeyboardInterrupt:
        stop.set()
        for p in list(children):
            p.terminate()
        pool.shutdown(wait=True, cancel_futures=True)
        say(f"\ninterrupted after {len(done)} transcripts; they're kept, and a rerun continues from there")
        return 130
    pool.shutdown()

    say(f"transcribed {len(done)}, skipped {total - len(todo)} (up to date), failed {len(failed)} → {TRANSCRIPTS}")
    for f in failed:
        say(f"  failed: {f}")
    return 1 if failed else 0


if __name__ == "__main__":
    sys.exit(main())
