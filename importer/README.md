# pokemon-import

Imports videos off an iPhone over USB (PTP) using libgphoto2 directly, instead
of shelling out to the `gphoto2` CLI. Replaces `../import_all.sh`.

## Why not the CLI

* `gphoto2 --get-file` addresses files by **index**, and the index from a
  recursive listing is not the index it expects alongside `--folder`. Getting
  that wrong downloaded the wrong file *while reporting success*, which is why
  the shell script needed a second pass to re-list every folder and recover
  folder-local numbering. `download_to()` takes a filename, so that pass and
  that failure mode are gone.
* The script spawned one `gphoto2` per file, each re-claiming the PTP session
  from scratch — a thousand USB re-enumerations per run, and the reason it
  needed so much reconnect machinery. Here the session is claimed once and held.
* Sizes come back as `u64` rather than being scraped out of a text column.

Still strictly serial: PTP allows one session per device, so there is no
parallelism to exploit.

## Not copying the same file twice

The script decided "have I already copied this?" by testing whether the
destination filename existed. That is wrong in both directions — move the
videos to editing storage and everything is copied again; a half-written file
from an interrupted transfer is indistinguishable from a finished one.

Imports are now **recorded**, in append-only JSONL at
`<dest>/.import_ledger.jsonl`, one line per successful import. Each file on the
phone is identified by `<folder-basename>/<filename>` (e.g.
`202607_a/MYVX1527.MP4`) — the `store_00010001` prefix is left out because it
is an artifact of a particular attachment, while the date-named folder is not.

Five cases, decided before any transfer starts:

| Ledger | On disk | Action |
| --- | --- | --- |
| yes | yes, size matches | skip |
| yes | yes, size differs | warn — a truncated earlier transfer; `--repair` to re-fetch |
| yes | no | **skip** — you moved or deleted it on purpose; `--recopy-missing` to override |
| no | yes, size matches phone | adopt: record it, don't re-fetch (this is how files the bash script already copied are picked up) |
| no | no | download |

Downloads stage through `<name>.part` and are renamed only after the byte count
matches what the phone reported, so an interrupted run can never leave
something that later looks complete.

Re-runs are cheap: files the ledger already knows need no metadata round-trip
to the phone, so a no-op run costs one directory walk rather than a query per
file.

## File naming and timestamps

New imports are named `2026-06-30_20-31-35_CNEM0645.MP4` — capture time first so
the directory sorts chronologically, original device name kept so a file stays
traceable to the phone and to its ledger key.

The time is rendered in the **local** zone. The MP4 container stores its
`CreateDate` in UTC with no timezone tag, and for an evening recording that
lands on the following calendar day (a 20:31 June 30 capture is
`2026-07-01T00:31Z`), which would file videos under a date nobody recorded on.
The tradeoff is that names depend on the importing machine's timezone.

Files copied by the old bash script keep their `202606_a_CNEM0645.MP4` names;
they are recognised and adopted, never renamed or re-fetched. When the phone
reports no capture time, naming falls back to that same legacy scheme rather
than inventing a date.

### Where the capture time lives

Three independent copies, in decreasing order of durability:

1. **`QuickTime:CreateDate` inside the MP4.** Survives renaming, copying,
   cloud sync — anything short of re-encoding. Read it with
   `exiftool -api QuickTimeUTC -CreateDate file.MP4`; without that flag you get
   the raw UTC value, shifted hours off and sometimes onto the wrong day.
2. **The filename**, once imported by this tool. Survives even transcoding and
   metadata-stripping uploads.
3. **Filesystem mtime.** The one you see in file properties, and the most
   fragile: `cp` without `-p`, `rsync` without `-t`/`-a`, cloud-sync uploads,
   zip round-trips and exFAT (2-second granularity) all destroy it. Renaming
   does *not* — `rename(2)` touches ctime only.

The ledger records the capture time too, as a fourth fallback.

If mtimes ever get flattened, restore them from the container:

```sh
exiftool -api QuickTimeUTC "-FileModifyDate<CreateDate" *.MP4
```

## Building

Needs the libgphoto2 development package (build time only):

```sh
sudo apt install libgphoto2-dev
cargo build --release
```

The resulting binary links only against the already-installed runtime libs
(`libgphoto2.so.6`, `libgphoto2_port.so.12`, `libexif.so.12`) — the dev package
is not needed to *run* it.

## Usage

```sh
./target/release/pokemon-import --dry-run     # show the plan, transfer nothing
./target/release/pokemon-import               # import new files
./target/release/pokemon-import --ext mp4 --ext mov
```

`--force` ignores the ledger entirely. `--no-evict` skips killing
gvfs/gthumb/shotwell before claiming the device. Exits non-zero if any file
failed.
