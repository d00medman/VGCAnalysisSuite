#!/usr/bin/env bash
#
# Bulk-import every MP4 off an iPhone over USB using gphoto2 (PTP).
#
# Why gphoto2 and not just mounting the phone: iOS exposes photos/videos
# through PTP (Picture Transfer Protocol), not as a normal USB mass-storage
# disk. There is no filesystem to cp from -- you talk to the device with a
# protocol, and only one process may hold the PTP session at a time. That
# single-claimant rule is the source of most of the complexity below.
#

dest="$HOME/Videos/pokemon_recordings"
mkdir -p "$dest"

#
# gphoto2 reports every size in KB, so that is the unit carried through the
# manifest and the counters. This renders it for humans using integer math
# only (no bc dependency): the fractional digits are computed by scaling the
# remainder before dividing.
#
human_size() {
    local kb=$1

    case "$kb" in
        ''|*[!0-9]*) printf '?'; return ;;
    esac

    if [ "$kb" -lt 1024 ]; then
        printf '%d KB' "$kb"
    elif [ "$kb" -lt 1048576 ]; then
        printf '%d.%01d MB' \
            $((kb / 1024)) \
            $(((kb % 1024) * 10 / 1024))
    else
        printf '%d.%02d GB' \
            $((kb / 1048576)) \
            $(((kb % 1048576) * 100 / 1048576))
    fi
}

#
# Desktop Linux auto-claims the phone the moment it is plugged in: gvfs
# spawns a gphoto2 backend to mount it, and photo managers pounce on the
# new-device signal. Any of them holding the PTP session makes our gphoto2
# calls fail with "Could not claim the USB device".
#
# So we evict the competition before each attempt. gvfsd will be respawned
# by D-Bus later; we only need it out of the way for the duration of a claim.
#
cleanup_phone_lock() {
    pkill -f gthumb 2>/dev/null || true
    pkill -f shotwell 2>/dev/null || true
    pkill -f gvfsd-gphoto2 2>/dev/null || true
    pkill -f gvfs-gphoto2-volume-monitor 2>/dev/null || true
}

#
# Try to take ownership of the PTP session, evicting squatters each round.
# The sleeps are load-bearing: after a pkill the USB device needs a moment
# to be released, and iOS itself is slow to re-offer the session.
#
# --summary is used purely as a cheap "can I talk to the device?" probe.
#
connect_phone() {
    local attempts=5

    for ((i=1; i<=attempts; i++)); do
        echo "Connection attempt $i/$attempts..."

        cleanup_phone_lock
        sleep 2

        if gphoto2 --summary >/dev/null 2>&1; then
            echo "iPhone connection OK."
            return 0
        fi

        echo "Could not claim iPhone. Retrying..."
        sleep 3
    done

    echo
    echo "ERROR: Could not establish a connection to the iPhone."
    echo "Make sure the phone is connected and unlocked."
    #
    # "unlocked" is not boilerplate: a locked iPhone refuses PTP entirely,
    # and the Trust prompt must have been accepted for this host.
    #
    return 1
}


echo "Checking iPhone connection..."
connect_phone || exit 1


#
# Two scratch files:
#   folders_tmp  -- folders on the phone that contain at least one MP4
#   manifest_tmp -- the work queue: folder + file number + filename + size
#
folders_tmp=$(mktemp)
manifest_tmp=$(mktemp)

trap 'rm -f "$folders_tmp" "$manifest_tmp"' EXIT


echo
echo "Scanning iPhone for folders containing MP4 files..."

#
# PASS 1 -- discovery. One recursive listing to find which folders matter.
#
# gphoto2 --list-files --recurse emits blocks like:
#
#   There are 42 files in folder '/store_00010001/202607_a'.
#   #1     IMG_0001.HEIC              rd   2140 KB image/heic
#   #2     IMG_0002.MP4               rd 842137 KB video/mp4
#
# The awk below is a tiny state machine: remember the folder from each
# header line, and emit that folder whenever a following entry is an MP4.
#
# sprintf("%c", 39) builds a single-quote character. It is written this way
# because a literal ' inside a single-quoted awk program would terminate the
# shell quoting -- this sidesteps the quoting fight entirely. Splitting the
# header on quotes puts the folder path in parts[2].
#
gphoto2 --list-files --recurse |
awk '
/files in folder/ {
    quote = sprintf("%c", 39)
    split($0, parts, quote)
    folder = parts[2]
}

# Entry lines start with #<number>. Match .mp4 case-insensitively, and
# require trailing whitespace so we hit the filename column, not a stray
# substring elsewhere on the line.
/^#[0-9]+/ && tolower($0) ~ /\.mp4[[:space:]]/ {
    print folder
}
' |
sort -u > "$folders_tmp"


folder_count=$(wc -l < "$folders_tmp")

echo "Found MP4s in $folder_count folders."
echo
echo "Building MP4 manifest using folder-local file numbers..."


#
# PASS 2 -- the whole reason this script has two passes.
#
# gphoto2 addresses files by INDEX, not by name: --get-file 3. There is no
# "download the file called X" option. And the index printed by a --recurse
# listing is not the index --get-file expects when you pass --folder: with
# --folder, numbering restarts at 1 within that folder.
#
# Downloading with numbers harvested from the recursive listing therefore
# fetches the wrong files -- silently, since the transfer still "succeeds".
#
# Hence: re-list each interesting folder on its own, and record the
# folder-local number. That number is what --get-file will agree with.
#
while IFS= read -r folder; do

    echo "Scanning $folder"

    listing=$(gphoto2 --folder "$folder" --list-files 2>/dev/null)

    # $? here is the exit status of the command substitution above --
    # i.e. gphoto2's own status. A failure almost always means the PTP
    # session dropped (phone locked, sleep, USB hiccup), so reconnect
    # and try the folder once more before giving up.
    if [ $? -ne 0 ]; then
        echo "Lost connection while scanning $folder."
        echo "Attempting to reconnect..."

        connect_phone || exit 1

        listing=$(gphoto2 --folder "$folder" --list-files 2>/dev/null)

        if [ $? -ne 0 ]; then
            echo "ERROR: Could not list $folder after reconnecting."
            exit 1
        fi
    fi

    # Emit a tab-separated work queue: folder, local number, filename, KB.
    #
    # $1 is "#12" -> strip the # to get the bare index.
    #
    # The size is found by walking backwards for the literal "KB" token and
    # taking the number in front of it, rather than trusting a fixed column.
    # gphoto2 omits the permissions or size field when the camera does not
    # report it, which shifts the columns; scanning for the unit survives
    # that, and leaves size empty when the device reported none.
    echo "$listing" |
    awk -v folder="$folder" '
    /^#[0-9]+/ && tolower($0) ~ /\.mp4[[:space:]]/ {
        num=$1
        sub(/^#/, "", num)

        filename=$2

        kb=""
        for (i = NF; i > 2; i--) {
            if ($i == "KB" && $(i-1) ~ /^[0-9]+$/) {
                kb=$(i-1)
                break
            }
        }

        print folder "\t" num "\t" filename "\t" kb
    }
    ' >> "$manifest_tmp"

# Note: the loop is fed by redirection, not a pipe. That keeps the body in
# the current shell, so counters and `exit` behave as written. Piping into
# `while` would run it in a subshell and quietly discard both.
done < "$folders_tmp"


total=$(wc -l < "$manifest_tmp")

# Sum column 4 for the batch total. Files with no reported size contribute
# nothing, so this is a lower bound rather than a guess.
total_kb=$(awk -F'\t' '{ s += $4 } END { print s + 0 }' "$manifest_tmp")

echo
echo "Found $total MP4 files ($(human_size "$total_kb"))."
echo


if [ "$total" -eq 0 ]; then
    echo "Nothing to download."
    exit 0
fi


#
# Sanity print: confirms the numbering really did restart per folder
# (you should see #1, #2, ... reappear as the folder column changes).
#
echo "First few files:"
head -10 "$manifest_tmp"

echo
echo "Starting download..."
echo


current=0
success=0
failed=0
done_kb=0
start_time=$(date +%s)


#
# PASS 3 -- transfer. Long-running (hundreds of GB over USB2-speed PTP),
# so it reports progress and survives mid-run disconnects rather than
# forcing a restart from zero.
#
while IFS=$'\t' read -r folder num filename sizekb; do

    current=$((current + 1))

    # /store_00010001/202607_a -> 202607_a
    # iOS names its PTP folders by capture year-month, so this prefix keeps
    # files from different months distinct and preserves rough chronology.
    foldername="${folder##*/}"

    elapsed=$(( $(date +%s) - start_time ))
    percent=$(( current * 100 / total ))

    # Byte progress is tracked alongside file-count progress because the
    # files vary hugely in size -- "12 of 103" says much less about how far
    # along you really are than the transferred volume does.
    printf '\n[%d/%d] %d%% | elapsed %02d:%02d:%02d | %s of %s\n' \
        "$current" \
        "$total" \
        "$percent" \
        $((elapsed / 3600)) \
        $(((elapsed % 3600) / 60)) \
        $((elapsed % 60)) \
        "$(human_size "$done_kb")" \
        "$(human_size "$total_kb")"

    echo "Folder: $folder"
    echo "File:   #$num $filename ($(human_size "$sizekb"))"

    output="$dest/${foldername}_${filename}"

    # Resumability: a completed file is never re-fetched, so re-running
    # after an abort picks up roughly where it left off. Skipped files still
    # count toward done_kb -- the bytes are on disk either way.
    if [ -f "$output" ]; then
        echo "Already exists — skipping:"
        echo "$output"

        success=$((success + 1))
        done_kb=$((done_kb + ${sizekb:-0}))
        continue
    fi


    if gphoto2 \
        --folder "$folder" \
        --get-file "$num" \
        --filename "$output"
    then

        success=$((success + 1))
        done_kb=$((done_kb + ${sizekb:-0}))

    else

        # Same failure mode as the scan loop: the session dropped. Re-claim
        # the device and retry this one file. A second failure is treated as
        # this file's problem, not the run's -- skip it and keep going.
        echo
        echo "Download failed. Attempting to reconnect..."

        if connect_phone; then

            echo "Retrying $filename..."

            if gphoto2 \
                --folder "$folder" \
                --get-file "$num" \
                --filename "$output"
            then

                success=$((success + 1))
                done_kb=$((done_kb + ${sizekb:-0}))

            else

                echo "Retry failed; skipping $filename."
                failed=$((failed + 1))

            fi

        else

            # Can't re-claim the device at all -- the phone is gone,
            # locked, or unplugged. Nothing further will work; stop.
            echo "Unable to reconnect; aborting."
            exit 1

        fi
    fi

done < "$manifest_tmp"


elapsed=$(( $(date +%s) - start_time ))


echo
echo "========================================"
echo "Transfer complete"
echo
echo "Successful: $success"
echo "Failed:     $failed"
echo "Total:      $total"
echo "Data:       $(human_size "$done_kb") of $(human_size "$total_kb")"
printf 'Elapsed:    %02d:%02d:%02d\n' \
    $((elapsed / 3600)) \
    $(((elapsed % 3600) / 60)) \
    $((elapsed % 60))
echo
echo "Destination:"
echo "$dest"
echo "========================================"
