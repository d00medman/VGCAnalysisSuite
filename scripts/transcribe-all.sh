#!/usr/bin/env bash
# Transcribe every video in raw_recordings/ and its batch folders to sandbox/transcripts/,
# mirroring the layout: raw_recordings/<batch>/<name>.MP4 → sandbox/transcripts/<batch>/<name>.txt
# (gitignored).
#
#   scripts/transcribe-all.sh [--regenerate] [batch...]
#
#   batch...       only these batch folders, e.g. Batch0_2026-08-28 (default: everything)
#   --regenerate   also redo transcripts older than the current analyzer build, e.g. after
#                  an atlas or reader change. Each one is replaced only once its new
#                  transcript succeeds, and the previous version is kept in
#                  sandbox/transcripts-previous/ for diffing.
#
# Safe to rerun from any directory: videos with an up-to-date transcript are skipped, so an
# interrupted run picks up where it left off, with or without --regenerate. Delete a .txt to
# redo that video. Each transcript is written to a .tmp file and renamed only once the
# analyzer succeeds.
set -euo pipefail

regenerate=0
batches=()
for arg in "$@"; do
  case $arg in
    --regenerate) regenerate=1 ;;
    -*) echo "unknown option: $arg" >&2; exit 2 ;;
    *) batches+=("$arg") ;;
  esac
done

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
n=0 done=0 skipped=0 failed=()

for video in "${files[@]}"; do
  n=$((n + 1))
  name=$(basename "$video")
  rel=${video#"$videos"/}
  txt=$out/${rel%.*}.txt
  mkdir -p "$(dirname "$txt")"
  if [[ -e $txt ]] && { ((!regenerate)) || [[ $txt -nt $newest ]]; }; then
    skipped=$((skipped + 1))
    continue
  fi
  echo "[$n/$total] $rel" >&2
  {
    echo "video: $name"
    echo "path:  $video"
    echo
  } >"$txt.tmp"
  if "$analyzer" transcript "$video" >>"$txt.tmp"; then
    if [[ -e $txt ]]; then
      mkdir -p "$(dirname "$previous/$rel")"
      mv "$txt" "$previous/${rel%.*}.txt"
    fi
    mv "$txt.tmp" "$txt"
    done=$((done + 1))
  else
    rm -f "$txt.tmp"
    failed+=("$rel")
  fi
done

echo "transcribed $done, skipped $skipped (up to date), failed ${#failed[@]} → $out" >&2
for f in "${failed[@]}"; do echo "  failed: $f" >&2; done
[[ ${#failed[@]} -eq 0 ]]
