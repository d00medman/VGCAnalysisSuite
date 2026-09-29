#!/usr/bin/env bash
# Transcribe every video in raw_recordings/ and its batch folders to sandbox/transcripts/,
# mirroring the layout: raw_recordings/<batch>/<name>.MP4 → sandbox/transcripts/<batch>/<name>.txt
# (gitignored).
#
#   scripts/transcribe-all.sh
#
# Safe to rerun from any directory: videos that already have a transcript are skipped, so an
# interrupted run picks up where it left off. Delete a .txt to redo that video. Each
# transcript is written to a .tmp file and renamed only once the analyzer succeeds.
set -euo pipefail

repo=$(cd "$(dirname "$0")/.." && pwd)
videos=$repo/raw_recordings
out=$repo/sandbox/transcripts
mkdir -p "$out"

cargo build --release --quiet --manifest-path "$repo/analyzer/Cargo.toml"
analyzer=$repo/analyzer/target/release/analyzer

shopt -s nullglob nocaseglob
files=("$videos"/*.mp4 "$videos"/*/*.mp4)
total=${#files[@]}
n=0 done=0 skipped=0 failed=()

for video in "${files[@]}"; do
  n=$((n + 1))
  name=$(basename "$video")
  rel=${video#"$videos"/}
  txt=$out/${rel%.*}.txt
  mkdir -p "$(dirname "$txt")"
  if [[ -e $txt ]]; then
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
    mv "$txt.tmp" "$txt"
    done=$((done + 1))
  else
    rm -f "$txt.tmp"
    failed+=("$rel")
  fi
done

echo "transcribed $done, skipped $skipped (already done), failed ${#failed[@]} → $out" >&2
for f in "${failed[@]}"; do echo "  failed: $f" >&2; done
[[ ${#failed[@]} -eq 0 ]]
