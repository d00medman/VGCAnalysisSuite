#!/usr/bin/env python3
"""Group the [unclear] lines in sandbox/transcripts/ by what probably went wrong.

    scripts/unclear-report.py                 # every batch
    scripts/unclear-report.py Batch1_2026-09-29
    scripts/unclear-report.py --lines out.tsv # also write every unclear line, classified

An unclear line has at least one glyph the reader couldn't identify, printed as `?`. Lines
fall into two kinds:

- **Partial:** readable text with a few `?`s. For each word with a `?`, the report guesses
  the missing character by matching the word against known words: pokedex names (from
  pokedex/data/*.csv) and every word read cleanly elsewhere in the transcripts. Only
  characters the glyph atlas lacks can fill a `?`, since the reader would have read any
  other. A character that keeps turning up should be added to the atlas.
  Partial lines are also split by where the `?`s are: in the player's name (a line like
  `<name> sent out ...`), or elsewhere.
- **Unreadable:** almost no letters read. Probably not message text at all (a popup, a
  transition, another script). Listed as bursts per video, so they can be found in the
  footage.

Reads text files only; never runs the analyzer.
"""

import argparse
import csv
import re
import sys
from collections import Counter, defaultdict
from pathlib import Path

REPO = Path(__file__).resolve().parent.parent
TRANSCRIPTS = REPO / "sandbox" / "transcripts"
ATLAS = REPO / "analyzer" / "atlas" / "glyphs.txt"
# Words the game prints that no clean line can supply, because the atlas lacks a character.
EXTRA_WORDS = ["Pokémon", "Poké"]
# A trainer's name opens these lines; nothing else in the game text comes first.
PLAYER_LINE = re.compile(r"^(.+?) (sent out|withdrew) ")
LINE = re.compile(r"^\[(\d+):(\d+\.\d+)\] (.*?)(  \[unclear\])?$")
# A line with fewer letters than this, once the `?`s are gone, counts as unreadable.
MIN_LETTERS = 4
# Unreadable lines closer together than this (seconds) are one burst.
BURST_GAP = 10.0


def load_lines(batches):
    """Yield (batch, video, seconds, text, unclear) for every transcript line."""
    for batch in batches:
        for txt in sorted(batch.glob("*.txt")):
            for raw in txt.read_text().splitlines():
                m = LINE.match(raw)
                if m:
                    secs = int(m[1]) * 60 + float(m[2])
                    yield batch.name, txt.stem, secs, m[3], bool(m[4])


def atlas_chars():
    """Every character the reader has a glyph for."""
    return set(re.findall(r'^glyph "(.)"', ATLAS.read_text(), re.M))


def vocabulary(lines):
    """Known words: pokedex display names plus every word of every clean line."""
    words = set(EXTRA_WORDS)
    data = REPO / "pokedex" / "data"
    for name, col in [("pokemon", "name"), ("move", "display_name"),
                      ("ability", "display_name"), ("item", "display_name")]:
        path = data / f"{name}.csv"
        if path.exists():
            with path.open() as f:
                for row in csv.DictReader(f):
                    words.update(tokens(row[col]))
    for _, _, _, text, unclear in lines:
        if not unclear:
            words.update(tokens(text))
    return words


def tokens(text):
    """Words, with surrounding punctuation stripped but inner hyphens and apostrophes kept."""
    out = []
    for w in text.split():
        w = w.strip("!.,:;\"()")
        if w:
            out.append(w)
    return out


def guess(word, by_shape, known):
    """Characters that could fill the `?`s in `word`, if exactly one known word fits. A
    filler must be a character the atlas lacks (`known` is what it has)."""
    pattern = re.compile("^" + "".join("(.)" if c == "?" else re.escape(c) for c in word) + "$")
    fits = {
        m.groups()
        for w in by_shape.get(len(word), ())
        if (m := pattern.match(w)) and not known.intersection(m.groups())
    }
    return "".join(next(iter(fits))) if len(fits) == 1 else None


def main():
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    ap.add_argument("batches", nargs="*", help="batch folder names (default: all)")
    ap.add_argument("--lines", type=Path, help="write every unclear line, classified, as TSV")
    ap.add_argument("--top", type=int, default=25, help="rows per table (default 25)")
    args = ap.parse_args()

    batches = [TRANSCRIPTS / b for b in args.batches] if args.batches else sorted(
        p for p in TRANSCRIPTS.iterdir() if p.is_dir())
    lines = list(load_lines(batches))
    if not lines:
        sys.exit(f"no transcript lines under {', '.join(map(str, batches))}")

    known = atlas_chars()
    by_shape = defaultdict(set)
    for w in vocabulary(lines):
        by_shape[len(w)].add(w)

    unclear = [l for l in lines if l[4]]
    videos = {(b, v) for b, v, *_ in lines}
    per_video = Counter((b, v) for b, v, *_ in unclear)

    filled = Counter()            # character → how many `?` words it fills
    examples = defaultdict(Counter)
    unguessed = Counter()         # `?` words no single known word fits
    unreadable = defaultdict(list)
    where = Counter()             # partial lines: "player name", "elsewhere", or both
    rows = []
    for b, v, t, text, _ in unclear:
        letters = sum(c.isalpha() for c in text)
        if letters < MIN_LETTERS:
            unreadable[(b, v)].append((t, text))
            rows.append((b, v, t, "unreadable", "", text))
            continue
        m = PLAYER_LINE.match(text)
        in_name = bool(m and "?" in m[1])
        rest = text[m.end():] if m else text
        if in_name and "?" in rest:
            where["player name and elsewhere"] += 1
        elif in_name:
            where["player name"] += 1
        else:
            where["elsewhere"] += 1
        guesses = []
        for w in tokens(text):
            if "?" not in w:
                continue
            # A word of only `?`s says nothing about which character is missing.
            g = guess(w, by_shape, known) if w.strip("?") else None
            if g:
                for c in set(g):
                    filled[c] += 1
                    examples[c][w] += 1
                guesses.append(f"{w}→{g}")
            else:
                unguessed[w] += 1
                guesses.append(f"{w}→?")
        rows.append((b, v, t, "partial", " ".join(guesses), text))

    n_unread = sum(len(x) for x in unreadable.values())
    print(f"# Unclear lines: {', '.join(b.name for b in batches)}\n")
    print(f"- **Transcripts:** {len(videos)}, of which {len(per_video)} have at least one unclear line")
    print(f"- **Lines:** {len(lines)}, of which {len(unclear)} unclear ({100 * len(unclear) / len(lines):.1f}%)")
    print(f"  - partial (readable text with `?`s): {len(unclear) - n_unread}")
    print(f"  - unreadable (fewer than {MIN_LETTERS} letters): {n_unread}")
    print("\n## Where the `?`s are, in partial lines\n")
    print("| where | lines |")
    print("|---|---|")
    for k in ["player name", "player name and elsewhere", "elsewhere"]:
        print(f"| {k} | {where[k]} |")
    print("\nThe atlas has: " + "".join(sorted(known)))

    print("\n## Likely missing glyphs\n")
    print("Characters that fill a `?` in a partial line, where exactly one known word fits.\n")
    print("| char | words fixed | examples |")
    print("|---|---|---|")
    for c, n in filled.most_common(args.top):
        ex = ", ".join(f"`{w}`×{k}" for w, k in examples[c].most_common(4))
        print(f"| `{c}` | {n} | {ex} |")

    print("\n## `?` words with no single match\n")
    print("| word | count |")
    print("|---|---|")
    for w, n in unguessed.most_common(args.top):
        print(f"| `{w}` | {n} |")

    print("\n## Unreadable bursts\n")
    print(f"Unreadable lines less than {BURST_GAP:.0f}s apart, grouped. Times are video time.\n")
    print("| video | at | lines | sample |")
    print("|---|---|---|---|")
    bursts = []
    for (b, v), items in unreadable.items():
        items.sort()
        start = prev = items[0][0]
        group = [items[0]]
        for t, text in items[1:]:
            if t - prev > BURST_GAP:
                bursts.append((len(group), v, start, group))
                start, group = t, []
            group.append((t, text))
            prev = t
        bursts.append((len(group), v, start, group))
    for n, v, start, group in sorted(bursts, key=lambda x: -x[0])[: args.top]:
        mins, secs = divmod(start, 60)
        print(f"| {v} | {int(mins):02d}:{secs:05.2f} | {n} | `{group[0][1]}` |")
    print(f"\n{len(bursts)} bursts in {len(unreadable)} videos.")

    print("\n## Videos with the most unclear lines\n")
    print("| video | unclear |")
    print("|---|---|")
    for (b, v), n in per_video.most_common(args.top):
        print(f"| {v} | {n} |")

    if args.lines:
        with args.lines.open("w") as f:
            f.write("batch\tvideo\tseconds\tkind\tguesses\ttext\n")
            for r in rows:
                f.write("\t".join(map(str, r)) + "\n")
        print(f"\nWrote {len(rows)} lines to {args.lines}")


if __name__ == "__main__":
    main()
