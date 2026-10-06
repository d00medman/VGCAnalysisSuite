#!/usr/bin/env bash
# Transcribe every video in raw_recordings/ and its batch folders to sandbox/transcripts/,
# mirroring the layout: raw_recordings/<batch>/<name>.MP4 → sandbox/transcripts/<batch>/<name>.txt
# (gitignored).
#
#   scripts/transcribe-all.sh [--regenerate] [-j N] [batch...]
#
#   batch...       only these batch folders, e.g. Batch0_2026-08-28 (default: everything)
#   --regenerate   also redo transcripts older than the current analyzer build, e.g. after
#                  an atlas or reader change. Each one is replaced only once its new
#                  transcript succeeds, and the previous version is kept in
#                  sandbox/transcripts-previous/ for diffing.
#   -j N           transcribe N videos at once (default 1). Decoding one video leaves some
#                  cores idle, so 2 can finish a batch sooner. Per-video progress lines are
#                  then suppressed, since they would interleave.
#
# The analyzer's speed experiments (ANALYZER_CROP_FIRST, ANALYZER_SKIP_NONREF,
# ANALYZER_HWACCEL; see analyzer/src/decode.rs) pass through from the environment.
#
# Safe to rerun from any directory: videos with an up-to-date transcript are skipped, so an
# interrupted run picks up where it left off, with or without --regenerate. Delete a .txt to
# redo that video. Each transcript is written to a .tmp file and renamed only once the
# analyzer succeeds.
set -euo pipefail

regenerate=0
jobs=1
batches=()
while (($#)); do
  case $1 in
    --regenerate) regenerate=1 ;;
    -j) jobs=${2:?-j needs a number}; shift ;;
    -j*) jobs=${1#-j} ;;
    -*) echo "unknown option: $1" >&2; exit 2 ;;
    *) batches+=("$1") ;;
  esac
  shift
done
[[ $jobs =~ ^[1-9][0-9]*$ ]] || { echo "-j needs a positive number, got $jobs" >&2; exit 2; }

repo=$(cd "$(dirname "$0")/.." && pwd)
videos=$repo/raw_recordings
out=$repo/sandbox/transcripts
previous=$repo/sandbox/transcripts-previous
mkdir -p "$out"

cargo build --release --quiet --manifest-path "$repo/analyzer/Cargo.toml"
analyzer=$repo/analyzer/target/release/analyzer
# The reader's output depends on the binary and the atlas and lexicon it loads; a transcript
# older than any of them is stale.
newest=$analyzer
for f in "$repo"/analyzer/atlas/{glyphs,lexicon}.txt; do
  [[ $f -nt $newest ]] && newest=$f
done

shopt -s nullglob nocaseglob
files=()
if ((${#batches[@]})); then
  for b in "${batches[@]}"; do
    [[ -d $videos/$b ]] || { echo "no batch folder: $videos/$b" >&2; exit 2; }
    files+=("$videos/$b"/*.mp4)
  done
else
  files=("$videos"/*.mp4 "$videos"/*/*.mp4)
fi
total=${#files[@]}
quiet=()
((jobs > 1)) && quiet=(-q)
# Each job leaves its outcome here, since background jobs can't update this shell's counters.
results=$(mktemp -d)
trap 'rm -rf "$results"' EXIT

# Transcribe video number $2 ($1); $3 is its transcript path.
transcribe_one() {
  local video=$1 n=$2 txt=$3 rel=${1#"$videos"/}
  {
    echo "video: $(basename "$video")"
    echo "path:  $video"
    echo
  } >"$txt.tmp"
  if "$analyzer" "${quiet[@]}" transcript "$video" >>"$txt.tmp"; then
    if [[ -e $txt ]]; then
      mkdir -p "$(dirname "$previous/$rel")"
      mv "$txt" "$previous/${rel%.*}.txt"
    fi
    mv "$txt.tmp" "$txt"
    touch "$results/$n.done"
    ((jobs > 1)) && echo "[$n/$total] done $rel" >&2
  else
    rm -f "$txt.tmp"
    echo "$rel" >"$results/$n.failed"
  fi
  return 0
}

n=0 skipped=0
for video in "${files[@]}"; do
  n=$((n + 1))
  rel=${video#"$videos"/}
  txt=$out/${rel%.*}.txt
  mkdir -p "$(dirname "$txt")"
  if [[ -e $txt ]] && { ((!regenerate)) || [[ $txt -nt $newest ]]; }; then
    skipped=$((skipped + 1))
    continue
  fi
  echo "[$n/$total] $rel" >&2
  if ((jobs == 1)); then
    transcribe_one "$video" "$n" "$txt"
  else
    transcribe_one "$video" "$n" "$txt" &
    while (($(jobs -rp | wc -l) >= jobs)); do wait -n || true; done
  fi
done
wait

done=$(find "$results" -name '*.done' | wc -l)
failed=()
for f in "$results"/*.failed; do [[ -e $f ]] && failed+=("$(<"$f")"); done
echo "transcribed $done, skipped $skipped (up to date), failed ${#failed[@]} → $out" >&2
for f in "${failed[@]}"; do echo "  failed: $f" >&2; done
[[ ${#failed[@]} -eq 0 ]]
