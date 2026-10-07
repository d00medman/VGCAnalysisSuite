"""Shared helpers for the scripts that work on transcripts: where things live in the repo,
and reading the transcript files the analyzer writes.

A transcript file starts with `video:` and `path:` header lines, then one message per line:

    [mm:ss.cc] <text>
    [mm:ss.cc] <text>  [unclear]

`[unclear]` marks a line with at least one glyph the reader couldn't identify, shown as `?`.
"""

import re
from dataclasses import dataclass
from pathlib import Path

REPO = Path(__file__).resolve().parent.parent
RECORDINGS = REPO / "raw_recordings"
TRANSCRIPTS = REPO / "sandbox" / "transcripts"
ANALYZER_DIR = REPO / "analyzer"

_LINE = re.compile(r"^\[(\d+):(\d+\.\d+)\] (.*?)(  \[unclear\])?$")


@dataclass(frozen=True)
class Line:
    t: float  # video seconds
    text: str
    unclear: bool


def parse(text: str) -> list[Line]:
    """The message lines of a transcript, in order. Header and blank lines are skipped."""
    out = []
    for raw in text.splitlines():
        if m := _LINE.match(raw):
            out.append(Line(int(m[1]) * 60 + float(m[2]), m[3], bool(m[4])))
    return out


def read(path: Path) -> list[Line]:
    return parse(path.read_text())


def read_dir(directory: Path) -> dict[str, list[Line]]:
    """Every transcript in `directory`, keyed by file stem (the video's name)."""
    return {p.stem: read(p) for p in sorted(directory.glob("*.txt"))}


def videos(batch: Path) -> list[Path]:
    """The recordings in a batch folder, in name order (which is capture-time order)."""
    return sorted(p for p in batch.iterdir() if p.suffix.lower() == ".mp4")
