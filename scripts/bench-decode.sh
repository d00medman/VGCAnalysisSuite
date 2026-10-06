#!/usr/bin/env bash
# Compare the analyzer's decode speed experiments (analyzer/src/decode.rs) against the
# default, on the same slice of real recordings: wall time, CPU time, and whether the
# transcripts match.
#
#   scripts/bench-decode.sh [video...]
#
# Default videos: 7 spread across Batch 1. Each is cut to SS..SS+T seconds (default 60, 180).
# Run it on an otherwise idle machine; anything else using the CPU skews the timings.
# Results go to sandbox/bench/<timestamp>/: one folder of transcripts per variant, and
# report.md. The analyzer is built into sandbox/bench/target, never over the binary that
# transcribe-all.sh uses.
set -euo pipefail

repo=$(cd "$(dirname "$0")/.." && pwd)
analyzer=$repo/sandbox/bench/target/release/analyzer
ffmpeg=$repo/analyzer/vendor/ffmpeg
ss=${SS:-60}
t=${T:-180}

# Worker mode, one per variant, so /usr/bin/time can measure its CPU including children:
#   bench-decode.sh --variant <out dir> <name> <videos at once> [VAR=value...]
# Videos come from BENCH_VIDEOS, one per line.
if [[ ${1:-} == --variant ]]; then
  dir=$2/$3 par=$4
  shift 4
  mkdir -p "$dir"
  mapfile -t videos <<<"$BENCH_VIDEOS"
  for v in "${videos[@]}"; do
    stem=$(basename "${v%.*}")
    (env "$@" "$analyzer" -q transcript "$v" --ss "$ss" --t "$t" >"$dir/$stem.txt" 2>"$dir/$stem.err" ||
      touch "$dir/$stem.failed") &
    while (($(jobs -rp | wc -l) >= par)); do wait -n || true; done
  done
  wait
  exit 0
fi

out=$repo/sandbox/bench/$(date +%Y%m%dT%H%M%S)
mkdir -p "$out"
if (($#)); then
  videos=("$@")
else
  mapfile -t videos < <(ls "$repo"/raw_recordings/Batch1_2026-09-29/*.MP4 | awk 'NR % 19 == 1')
fi
export BENCH_VIDEOS SS=$ss T=$t
BENCH_VIDEOS=$(printf '%s\n' "${videos[@]}")

echo "building the analyzer into sandbox/bench/target" >&2
CARGO_TARGET_DIR=$repo/sandbox/bench/target cargo build --release --quiet --manifest-path "$repo/analyzer/Cargo.toml"

# 1. Crop-before-rotate must take exactly the same pixels, or its timings mean nothing.
echo "checking crop-first pixels against the default (60 frames)" >&2
md5() { "$@" -frames:v 60 -f rawvideo -pix_fmt rgb24 - 2>/dev/null | md5sum | cut -d' ' -f1; }
a=$(md5 "$ffmpeg" -v error -ss "$ss" -i "${videos[0]}" -vf crop=1936:120:500:740)
b=$(md5 "$ffmpeg" -v error -noautorotate -ss "$ss" -i "${videos[0]}" -vf crop=120:1936:266:500,transpose=cclock)
if [[ $a == "$b" ]]; then pixels=identical; else pixels="DIFFERENT"; fi
echo "  $pixels" >&2

# 2. The variants: name, videos at once, environment.
all=(ANALYZER_CROP_FIRST=1 ANALYZER_SKIP_NONREF=1 ANALYZER_HWACCEL=vdpau)
variant() {
  local name=$1 start=$SECONDS
  echo "variant $name: $2 at once, ${*:3}" >&2
  /usr/bin/time -f "%U %S" -o "$out/$name.cpu" "$0" --variant "$out" "$@"
  echo "$name $((SECONDS - start))" >>"$out/times"
}
variant base 1
variant crop-first 1 ANALYZER_CROP_FIRST=1
variant skip-nonref 1 ANALYZER_SKIP_NONREF=1
variant vdpau 1 ANALYZER_HWACCEL=vdpau
variant all 1 "${all[@]}"
variant base-x2 2
variant all-x2 2 "${all[@]}"

# 3. Report: speed, and how closely each variant's transcripts match the default's.
python3 - "$out" "$pixels" "${#videos[@]}" "$ss" "$t" <<'EOF'
import sys, re, difflib
from pathlib import Path
out, pixels, n, ss, t = Path(sys.argv[1]), *sys.argv[2:]
LINE = re.compile(r"^\[(\d+):(\d+\.\d+)\] (.*?)(  \[unclear\])?$")

def transcripts(d):
    return {f.stem: [(int(m[1]) * 60 + float(m[2]), m[3], bool(m[4]))
                     for l in f.read_text().splitlines() if (m := LINE.match(l))]
            for f in sorted(d.glob("*.txt"))}

times = dict(l.split() for l in (out / "times").read_text().splitlines())
base = transcripts(out / "base")
b_secs = int(times["base"])
rows = []
for name, secs in times.items():
    secs = int(secs)
    cpu = sum(map(float, (out / f"{name}.cpu").read_text().split()[-2:]))
    failed = len(list((out / name).glob("*.failed")))
    got = transcripts(out / name)
    same = total = unclear = 0
    drift = 0.0
    for stem, bl in base.items():
        vl = got.get(stem, [])
        unclear += sum(u for *_, u in vl)
        total += max(len(bl), len(vl))
        sm = difflib.SequenceMatcher(a=[x[1] for x in bl], b=[x[1] for x in vl], autojunk=False)
        for blk in sm.get_matching_blocks():
            same += blk.size
            for k in range(blk.size):
                drift = max(drift, abs(bl[blk.a + k][0] - vl[blk.b + k][0]))
    rows.append(f"| {name} | {secs}s | {b_secs / max(secs, 1):.2f}x | {cpu:.0f}s | "
                f"{'all ok' if not failed else f'{failed} FAILED'} | {same}/{total} | {drift:.2f}s | {unclear} |")

report = (f"# Decode benchmark\n\n{n} videos, {t}s each from {ss}s. Crop-first pixels: {pixels}.\n\n"
          "| variant | wall | speedup | CPU | runs | lines same as base | max time drift | unclear |\n"
          "|---|---|---|---|---|---|---|---|\n" + "\n".join(rows) + "\n")
(out / "report.md").write_text(report)
print(report)
EOF
echo "transcripts and report: $out" >&2
