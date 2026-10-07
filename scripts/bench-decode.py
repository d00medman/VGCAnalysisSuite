#!/usr/bin/env python3
"""Compare the analyzer's decode speed experiments (analyzer/src/decode.rs) against the
default, on the same slice of real recordings: wall time, CPU time, and whether the
transcripts match.

    scripts/bench-decode.py [--ss 60] [--t 180] [--only base,vdpau,...] [video...]

Default videos: 7 spread across Batch 1, each cut to ss..ss+t seconds. Run it on an
otherwise idle machine; anything else using the CPU skews the timings. Results go to
sandbox/bench/<timestamp>/: one folder of transcripts per variant, and report.md. The
analyzer is built into sandbox/bench/target, never over the binary transcribe-all.py uses.
"""

import argparse
import difflib
import hashlib
import os
import resource
import subprocess
import sys
import time
from concurrent.futures import ThreadPoolExecutor
from datetime import datetime
from pathlib import Path

import transcripts
from transcripts import ANALYZER_DIR, RECORDINGS, REPO

BENCH = REPO / "sandbox" / "bench"
ANALYZER = BENCH / "target" / "release" / "analyzer"
FFMPEG = ANALYZER_DIR / "vendor" / "ffmpeg"

ALL = {"ANALYZER_CROP_FIRST": "1", "ANALYZER_SKIP_NONREF": "1", "ANALYZER_HWACCEL": "vdpau"}
# name → (videos at once, environment)
VARIANTS = {
    "base": (1, {}),
    "crop-first": (1, {"ANALYZER_CROP_FIRST": "1"}),
    "skip-nonref": (1, {"ANALYZER_SKIP_NONREF": "1"}),
    "vdpau": (1, {"ANALYZER_HWACCEL": "vdpau"}),
    "all": (1, ALL),
    "base-x2": (2, {}),
    "all-x2": (2, ALL),
}


def say(msg: str) -> None:
    print(msg, file=sys.stderr, flush=True)


def crop_first_pixels_match(video: Path, ss: float) -> bool:
    """Does crop-before-rotate take exactly the default's pixels? Checked on 60 frames."""
    def md5(*args):
        cmd = [str(FFMPEG), "-v", "error", *args, "-frames:v", "60", "-f", "rawvideo", "-pix_fmt", "rgb24", "-"]
        return hashlib.md5(subprocess.run(cmd, capture_output=True, check=True).stdout).hexdigest()
    default = md5("-ss", str(ss), "-i", str(video), "-vf", "crop=1936:120:500:740")
    first = md5("-noautorotate", "-ss", str(ss), "-i", str(video), "-vf", "crop=120:1936:266:500,transpose=cclock")
    return default == first


def run_variant(name: str, par: int, env: dict, videos: list[Path], ss: float, t: float, out: Path):
    """Transcribe every slice under one variant. Returns (wall s, CPU s, failures)."""
    d = out / name
    d.mkdir()
    full_env = {**os.environ, **env}
    failures = []

    def one(i_video):
        i, video = i_video
        start = time.monotonic()
        with (d / f"{video.stem}.txt").open("w") as f, (d / f"{video.stem}.err").open("w") as err:
            code = subprocess.run([str(ANALYZER), "-q", "transcript", str(video), "--ss", str(ss), "--t", str(t)],
                                  stdout=f, stderr=err, env=full_env, stdin=subprocess.DEVNULL).returncode
        status = "done" if code == 0 else f"FAILED (exit {code}, see {name}/{video.stem}.err)"
        if code:
            failures.append(video.stem)
        say(f"    {name}: {i}/{len(videos)} {video.stem} {status} in {time.monotonic() - start:.0f}s")

    # CPU of every finished child (the analyzer, and the ffmpeg it waits for), before and after.
    before = resource.getrusage(resource.RUSAGE_CHILDREN)
    start = time.monotonic()
    with ThreadPoolExecutor(max_workers=par) as pool:
        list(pool.map(one, enumerate(videos, 1)))
    wall = time.monotonic() - start
    after = resource.getrusage(resource.RUSAGE_CHILDREN)
    cpu = (after.ru_utime - before.ru_utime) + (after.ru_stime - before.ru_stime)
    return wall, cpu, failures


def compare(base: dict, got: dict):
    """(lines matching base by text, total lines, max time drift of matching lines, unclear)."""
    same = total = unclear = 0
    drift = 0.0
    for stem, bl in base.items():
        vl = got.get(stem, [])
        unclear += sum(l.unclear for l in vl)
        total += max(len(bl), len(vl))
        sm = difflib.SequenceMatcher(a=[l.text for l in bl], b=[l.text for l in vl], autojunk=False)
        for blk in sm.get_matching_blocks():
            same += blk.size
            for k in range(blk.size):
                drift = max(drift, abs(bl[blk.a + k].t - vl[blk.b + k].t))
    return same, total, drift, unclear


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    ap.add_argument("videos", nargs="*", type=Path, help="recordings (default: 7 across Batch 1)")
    ap.add_argument("--ss", type=float, default=60, help="slice start, seconds (default 60)")
    ap.add_argument("--t", type=float, default=180, help="slice length, seconds (default 180)")
    ap.add_argument("--only", help=f"comma-separated variants (default all: {', '.join(VARIANTS)})")
    args = ap.parse_args()

    videos = args.videos or transcripts.videos(RECORDINGS / "Batch1_2026-09-29")[::19]
    names = args.only.split(",") if args.only else list(VARIANTS)
    unknown = [n for n in names if n not in VARIANTS]
    if unknown:
        ap.error(f"unknown variant: {', '.join(unknown)}")
    if "base" not in names:
        names.insert(0, "base")  # everything is compared against it
    out = BENCH / datetime.now().strftime("%Y%m%dT%H%M%S")
    out.mkdir(parents=True)

    say("building the analyzer into sandbox/bench/target")
    subprocess.run(["cargo", "build", "--release", "--quiet", "--manifest-path", str(ANALYZER_DIR / "Cargo.toml")],
                   env={**os.environ, "CARGO_TARGET_DIR": str(BENCH / "target")}, check=True)

    say("checking crop-first pixels against the default (60 frames)")
    pixels = "identical" if crop_first_pixels_match(videos[0], args.ss) else "DIFFERENT"
    say(f"  {pixels}")

    results = {}
    for name in names:
        par, env = VARIANTS[name]
        say(f"variant {name}: {par} at once, {' '.join(f'{k}={v}' for k, v in env.items()) or 'default settings'}")
        results[name] = run_variant(name, par, env, videos, args.ss, args.t, out)
        say(f"  {name} finished in {results[name][0]:.0f}s")

    base = transcripts.read_dir(out / "base")
    b_wall = results["base"][0]
    rows = []
    for name, (wall, cpu, failures) in results.items():
        same, total, drift, unclear = compare(base, transcripts.read_dir(out / name))
        runs = "all ok" if not failures else f"{len(failures)} FAILED"
        rows.append(f"| {name} | {wall:.0f}s | {b_wall / wall:.2f}x | {cpu:.0f}s | {runs} | "
                    f"{same}/{total} | {drift:.2f}s | {unclear} |")
    report = (f"# Decode benchmark\n\n{len(videos)} videos, {args.t:.0f}s each from {args.ss:.0f}s. "
              f"Crop-first pixels: {pixels}.\n\n"
              "| variant | wall | speedup | CPU | runs | lines same as base | max time drift | unclear |\n"
              "|---|---|---|---|---|---|---|---|\n" + "\n".join(rows) + "\n")
    (out / "report.md").write_text(report)
    print(report)
    say(f"transcripts and report: {out}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
