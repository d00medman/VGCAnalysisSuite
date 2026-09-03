//! Import videos off an iPhone over USB, without ever copying the same file twice.
//!
//! This replaces a bash script that drove the `gphoto2` CLI. Two things about
//! that CLI shaped the old design and are gone here:
//!
//!   * `--get-file` addresses files by *index*, and the index printed by a
//!     recursive listing is not the index it expects when given `--folder`.
//!     Getting that wrong downloaded the wrong file while still reporting
//!     success, so the script needed a whole second pass to re-list every
//!     folder and recover folder-local numbering. `download_to` takes a
//!     filename, so that pass and that entire failure mode are gone.
//!
//!   * Every file meant a fresh process re-claiming the PTP session from
//!     scratch -- a thousand USB re-enumerations per run, and the reason the
//!     script needed so much reconnect machinery. Here the session is claimed
//!     once and held.
//!
//! Note that this is still strictly serial: PTP permits one session per device,
//! so there is no parallelism available to exploit, in Rust or anywhere else.

mod device;
mod ledger;

use anyhow::{Context as _, Result};
use clap::Parser;
use gphoto2::Camera;
use ledger::{Entry, Ledger, LEDGER_NAME};
use std::path::{Path, PathBuf};
use std::time::{Instant, SystemTime, UNIX_EPOCH};

#[derive(Parser, Debug)]
#[command(about = "Import videos from an iPhone over PTP, skipping anything already imported")]
struct Args {
    /// Destination directory (also holds the import ledger).
    #[arg(long)]
    dest: Option<PathBuf>,

    /// File extensions to import; repeat for more. Case-insensitive.
    #[arg(long = "ext", default_values_t = [String::from("mp4")])]
    exts: Vec<String>,

    /// Show what would happen, transfer nothing.
    #[arg(long)]
    dry_run: bool,

    /// Re-download everything, ignoring the ledger entirely.
    #[arg(long)]
    force: bool,

    /// Re-download files the ledger has imported but that are no longer on disk.
    /// Off by default -- deleting or moving an imported file is normally
    /// deliberate, and re-copying it is exactly what this tool exists to avoid.
    #[arg(long)]
    recopy_missing: bool,

    /// Re-download imported files whose size on disk disagrees with the phone.
    #[arg(long)]
    repair: bool,

    /// Don't kill gvfs/photo-manager processes before claiming the device.
    #[arg(long)]
    no_evict: bool,

    /// Connection attempts before giving up.
    #[arg(long, default_value_t = 5)]
    attempts: u32,
}

/// A file as it exists on the phone.
struct DeviceFile {
    /// Full PTP path, e.g. "/store_00010001/202607_a".
    folder: String,
    name: String,
}

impl DeviceFile {
    /// Basename of the containing folder: "202607_a".
    fn folder_name(&self) -> &str {
        self.folder.rsplit('/').next().unwrap_or(&self.folder)
    }

    /// Ledger identity. Folder basename rather than full path, because the
    /// `store_00010001` component is an artifact of this particular attachment
    /// and the date-named folder is what actually identifies the file.
    fn key(&self) -> String {
        format!("{}/{}", self.folder_name(), self.name)
    }

    /// The scheme the bash script used. Still needed: files it already copied
    /// carry these names, and they are recognised and adopted rather than
    /// re-fetched. Also the fallback when the phone reports no capture time.
    fn legacy_dest_name(&self) -> String {
        format!("{}_{}", self.folder_name(), self.name)
    }

    /// Destination filename, capture time first so the directory sorts
    /// chronologically, original device name retained so a file can still be
    /// traced back to the phone and to its ledger key.
    ///
    /// Rendered in *local* time. The container stores UTC, and for an evening
    /// recording that lands on the following calendar day -- a 20:31 June 30
    /// capture is `2026-07-01T00:31Z` -- which would put files under a date
    /// nobody recorded on. The tradeoff is that names depend on the importing
    /// machine's timezone.
    fn dest_name(&self, mtime: Option<i64>) -> String {
        match mtime.filter(|t| *t > 0).and_then(local_stamp) {
            Some(stamp) => format!("{}_{}", stamp, self.name),
            None => self.legacy_dest_name(),
        }
    }
}

/// What to do about one file, decided before any transfer begins.
enum Decision {
    /// Not seen before -- fetch it.
    Download { size: Option<u64>, mtime: Option<i64> },
    /// The ledger has it. Nothing to do.
    AlreadyImported,
    /// On disk at the right size but predating the ledger (e.g. copied by the
    /// old bash script). Record it under whichever name it already has and
    /// move on -- no reason to re-fetch bytes that are already correct, and no
    /// reason to rename a file the user did not ask to have renamed.
    Adopt { size: u64, mtime: Option<i64>, dest: String },
    /// Imported once, but the file is no longer where we put it. Assumed
    /// deliberate; skipped unless --recopy-missing.
    MissingLocally,
    /// Imported, still present, but the size disagrees with the phone --
    /// the signature of a transfer interrupted partway.
    SizeMismatch { on_disk: u64, on_device: u64, dest: String },
}

fn main() -> Result<()> {
    let args = Args::parse();

    let dest = match &args.dest {
        Some(d) => d.clone(),
        None => {
            let home = std::env::var("HOME").context("HOME is not set")?;
            PathBuf::from(home).join("Videos").join("pokemon_recordings")
        }
    };
    std::fs::create_dir_all(&dest)
        .with_context(|| format!("creating destination {}", dest.display()))?;

    let exts: Vec<String> = args.exts.iter().map(|e| e.trim_start_matches('.').to_lowercase()).collect();

    let mut ledger = Ledger::load(&dest)?;
    println!("Ledger: {} file(s) previously imported.", ledger.len());
    println!("Destination: {}", dest.display());
    println!();

    println!("Checking iPhone connection...");
    let camera = device::connect(args.attempts, !args.no_evict)?;

    println!();
    println!("Scanning iPhone for matching files...");
    let found = scan(&camera, &exts)?;
    println!("Found {} file(s) matching {:?} on the device.", found.len(), exts);

    // Decide everything up front. This is what makes a re-run cheap: files the
    // ledger already knows about need no round-trip to the phone for metadata,
    // so a no-op run costs one directory walk rather than one query per file.
    println!("Comparing against ledger...");
    let mut planned: Vec<(DeviceFile, Decision)> = Vec::with_capacity(found.len());
    for file in found {
        let decision = decide(&camera, &ledger, &dest, &file, &args)?;
        planned.push((file, decision));
    }

    let mut adopted = 0usize;
    let mut skipped = 0usize;
    let mut missing = 0usize;
    let mut mismatched: Vec<String> = Vec::new();
    let mut queue: Vec<(DeviceFile, Option<u64>, Option<i64>)> = Vec::new();

    for (file, decision) in planned {
        match decision {
            Decision::Download { size, mtime } => queue.push((file, size, mtime)),
            Decision::AlreadyImported => skipped += 1,
            Decision::Adopt { size, mtime, dest: name } => {
                if args.dry_run {
                    println!("  would adopt existing file: {name}");
                } else {
                    ledger.append(entry_for(&file, Some(size), mtime, name))?;
                }
                adopted += 1;
            }
            Decision::MissingLocally => {
                println!(
                    "  imported previously but not on disk (skipping; --recopy-missing to re-fetch): {}",
                    file.key()
                );
                missing += 1;
            }
            Decision::SizeMismatch { on_disk, on_device, dest: name } => {
                println!(
                    "  WARNING size mismatch, likely a truncated earlier transfer: {name} ({} on disk, {} on phone)",
                    human_size(on_disk),
                    human_size(on_device)
                );
                mismatched.push(name);
            }
        }
    }

    let total = queue.len();
    let total_bytes: u64 = queue.iter().filter_map(|(_, s, _)| *s).sum();

    println!();
    println!("Already imported: {skipped}");
    if adopted > 0 {
        println!("Adopted (already on disk, now recorded): {adopted}");
    }
    if missing > 0 {
        println!("Recorded but missing locally: {missing}");
    }
    if !mismatched.is_empty() {
        println!("Size mismatches: {} (re-run with --repair to re-fetch)", mismatched.len());
    }
    println!("To download: {total} ({})", human_size(total_bytes));
    println!();

    if total == 0 {
        println!("Nothing to download.");
        return Ok(());
    }

    if args.dry_run {
        println!("Dry run; would download:");
        for (file, size, mtime) in &queue {
            println!(
                "  {} ({})",
                file.dest_name(*mtime),
                size.map(human_size).unwrap_or_else(|| "?".into())
            );
        }
        return Ok(());
    }

    let start = Instant::now();
    let mut done_bytes = 0u64;
    let mut ok = 0usize;
    let mut failed = 0usize;

    for (i, (file, size, mtime)) in queue.iter().enumerate() {
        let elapsed = start.elapsed().as_secs();
        println!();
        println!(
            "[{}/{}] {}% | elapsed {} | {} of {}",
            i + 1,
            total,
            (i * 100) / total,
            hms(elapsed),
            human_size(done_bytes),
            human_size(total_bytes)
        );
        println!("Folder: {}", file.folder);
        println!(
            "File:   {} ({})",
            file.name,
            size.map(human_size).unwrap_or_else(|| "?".into())
        );

        match download_one(&camera, &dest, file, *size, *mtime) {
            Ok(written) => {
                done_bytes += written;
                ok += 1;
                // The ledger becomes a third copy of the capture time, after
                // the container's own CreateDate atom and the filesystem
                // mtime. Both of those can be stripped by a careless copy or
                // an upload; this one cannot.
                ledger.append(entry_for(file, Some(written), *mtime, file.dest_name(*mtime)))?;
            }
            Err(e) => {
                eprintln!("  FAILED: {e:#}");
                failed += 1;
            }
        }
    }

    let elapsed = start.elapsed().as_secs();
    println!();
    println!("========================================");
    println!("Transfer complete");
    println!();
    println!("Downloaded: {ok}");
    println!("Failed:     {failed}");
    println!("Data:       {}", human_size(done_bytes));
    println!("Elapsed:    {}", hms(elapsed));
    println!();
    println!("Destination: {}", dest.display());
    println!("Ledger:      {}", dest.join(LEDGER_NAME).display());
    println!("========================================");

    if failed > 0 {
        std::process::exit(1);
    }
    Ok(())
}

/// Walk the device tree collecting files with a matching extension.
///
/// Deliberately does *not* fetch per-file metadata here -- that costs a round
/// trip each, and most files on a re-run are resolved from the ledger without
/// ever needing it.
fn scan(camera: &Camera, exts: &[String]) -> Result<Vec<DeviceFile>> {
    let fs = camera.fs();
    let mut stack = vec![String::from("/")];
    let mut out = Vec::new();

    while let Some(folder) = stack.pop() {
        let files: Vec<String> = fs
            .list_files(&folder)
            .wait()
            .with_context(|| format!("listing files in {folder}"))?
            .collect();

        for name in files {
            if matches_ext(&name, exts) {
                out.push(DeviceFile { folder: folder.clone(), name });
            }
        }

        let subfolders: Vec<String> = fs
            .list_folders(&folder)
            .wait()
            .with_context(|| format!("listing folders in {folder}"))?
            .collect();

        for sub in subfolders {
            stack.push(if folder == "/" {
                format!("/{sub}")
            } else {
                format!("{folder}/{sub}")
            });
        }
    }

    out.sort_by(|a, b| (a.folder.as_str(), a.name.as_str()).cmp(&(&b.folder, &b.name)));
    Ok(out)
}

fn matches_ext(name: &str, exts: &[String]) -> bool {
    match name.rsplit_once('.') {
        Some((_, ext)) => exts.iter().any(|e| e == &ext.to_lowercase()),
        None => false,
    }
}

fn decide(
    camera: &Camera,
    ledger: &Ledger,
    dest: &Path,
    file: &DeviceFile,
    args: &Args,
) -> Result<Decision> {
    if args.force {
        let (size, mtime) = device_info(camera, file)?;
        return Ok(Decision::Download { size, mtime });
    }

    // For a known file the ledger records the name we actually wrote, which may
    // predate timestamped naming. Trusting it rather than recomputing means a
    // change of naming scheme never orphans an existing import.
    let known = ledger.get(&file.key());
    let on_disk = known
        .map(|e| dest.join(&e.dest))
        .and_then(|p| std::fs::metadata(p).ok())
        .map(|m| m.len());

    match (known, on_disk) {
        // Known and present: the overwhelmingly common case on a re-run.
        // Verify the size only if the ledger recorded one.
        (Some(entry), Some(actual)) => match entry.size {
            Some(expected) if expected != actual => {
                if args.repair {
                    let (size, mtime) = device_info(camera, file)?;
                    Ok(Decision::Download { size, mtime })
                } else {
                    Ok(Decision::SizeMismatch {
                        on_disk: actual,
                        on_device: expected,
                        dest: entry.dest.clone(),
                    })
                }
            }
            _ => Ok(Decision::AlreadyImported),
        },

        // Known but gone. The whole point of the ledger: do not silently
        // re-copy something the user moved or deleted on purpose.
        (Some(_), None) => {
            if args.recopy_missing {
                let (size, mtime) = device_info(camera, file)?;
                Ok(Decision::Download { size, mtime })
            } else {
                Ok(Decision::MissingLocally)
            }
        }

        // Unknown to the ledger but already sitting in the destination. Either
        // the bash script put it there or a previous run died before recording
        // it. Trust it only if the size matches the phone exactly; otherwise
        // treat it as a partial file and fetch it again.
        // Unknown to the ledger. It may still be on disk under either naming
        // scheme -- the bash script's, or a timestamped name from an earlier
        // run that died before recording it. Check both before fetching.
        (None, _) => {
            let (size, mtime) = device_info(camera, file)?;
            let candidates = [file.dest_name(mtime), file.legacy_dest_name()];

            match find_existing(dest, &candidates, size) {
                Some((name, actual)) => {
                    Ok(Decision::Adopt { size: actual, mtime, dest: name })
                }
                None => Ok(Decision::Download { size, mtime }),
            }
        }
    }
}

/// One metadata round-trip to the phone, returning both fields we care about.
/// Kept as a single call because each query is a real USB exchange and the
/// adopt path needs size and mtime together.
/// First candidate filename that already exists in `dest` at exactly the size
/// the phone reports.
///
/// The size check is what makes adoption safe: a file left half-written by an
/// interrupted run has the right name but the wrong length, and must be
/// re-fetched rather than trusted. If the phone reports no size at all we
/// cannot verify anything, so nothing is adopted and the file is downloaded.
fn find_existing(dest: &Path, candidates: &[String], expected: Option<u64>) -> Option<(String, u64)> {
    let expected = expected?;
    for name in candidates {
        if let Ok(meta) = std::fs::metadata(dest.join(name)) {
            if meta.len() == expected {
                return Some((name.clone(), expected));
            }
        }
    }
    None
}

fn device_info(camera: &Camera, file: &DeviceFile) -> Result<(Option<u64>, Option<i64>)> {
    let info = camera
        .fs()
        .file_info(&file.folder, &file.name)
        .wait()
        .with_context(|| format!("reading info for {}/{}", file.folder, file.name))?;
    let f = info.file();
    Ok((f.size(), f.mtime().map(|t| t as i64)))
}

/// Fetch one file via a `.part` staging path.
///
/// The rename is the point: a file only appears at its real name once it is
/// complete, so an interrupted run can never leave something that looks
/// finished. The old script wrote straight to the destination, where a
/// truncated file would be skipped as "already there" forever after.
fn download_one(
    camera: &Camera,
    dest: &Path,
    file: &DeviceFile,
    expected: Option<u64>,
    mtime: Option<i64>,
) -> Result<u64> {
    let name = file.dest_name(mtime);
    let final_path = dest.join(&name);
    let part_path = dest.join(format!("{name}.part"));

    if part_path.exists() {
        std::fs::remove_file(&part_path).ok();
    }

    camera
        .fs()
        .download_to(&file.folder, &file.name, &part_path)
        .wait()
        .with_context(|| format!("downloading {}/{}", file.folder, file.name))?;

    let written = std::fs::metadata(&part_path)
        .with_context(|| format!("stat {}", part_path.display()))?
        .len();

    if let Some(expected) = expected {
        if written != expected {
            std::fs::remove_file(&part_path).ok();
            anyhow::bail!(
                "size mismatch: got {} bytes, phone reported {}",
                written,
                expected
            );
        }
    }

    std::fs::rename(&part_path, &final_path)
        .with_context(|| format!("renaming into place: {}", final_path.display()))?;

    // libgphoto2 already stamps the capture time onto the file it writes, and
    // rename preserves it. Re-applying it explicitly costs nothing and means
    // the timestamp does not depend on that behaviour staying true -- the
    // mtime is the most convenient copy of the capture time, even though the
    // container's own CreateDate atom is the durable one.
    if let Some(secs) = mtime {
        if secs > 0 {
            if let Err(e) = set_mtime(&final_path, secs) {
                eprintln!("  warning: could not set mtime on {}: {e:#}", final_path.display());
            }
        }
    }

    Ok(written)
}

fn set_mtime(path: &Path, secs: i64) -> Result<()> {
    let when = UNIX_EPOCH
        .checked_add(std::time::Duration::from_secs(secs as u64))
        .context("timestamp out of range")?;
    std::fs::File::options()
        .write(true)
        .open(path)
        .with_context(|| format!("opening {} to set mtime", path.display()))?
        .set_modified(when)
        .with_context(|| format!("setting mtime on {}", path.display()))?;
    Ok(())
}

fn entry_for(file: &DeviceFile, size: Option<u64>, mtime: Option<i64>, dest: String) -> Entry {
    Entry {
        key: file.key(),
        folder: file.folder.clone(),
        name: file.name.clone(),
        size,
        mtime,
        dest,
        imported_at: SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0),
    }
}

/// Capture time as `YYYY-MM-DD_HH-MM-SS` in the local zone. Colons are illegal
/// or awkward in filenames on other platforms, hence the dashes.
fn local_stamp(secs: i64) -> Option<String> {
    use chrono::{Local, TimeZone};
    match Local.timestamp_opt(secs, 0) {
        chrono::LocalResult::Single(dt) => Some(dt.format("%Y-%m-%d_%H-%M-%S").to_string()),
        // Ambiguous (a DST fall-back hour) or nonexistent: prefer the earlier
        // reading over refusing to name the file.
        chrono::LocalResult::Ambiguous(dt, _) => Some(dt.format("%Y-%m-%d_%H-%M-%S").to_string()),
        chrono::LocalResult::None => None,
    }
}

fn human_size(bytes: u64) -> String {
    const K: f64 = 1024.0;
    let b = bytes as f64;
    if b < K {
        format!("{bytes} B")
    } else if b < K * K {
        format!("{:.1} KB", b / K)
    } else if b < K * K * K {
        format!("{:.1} MB", b / (K * K))
    } else {
        format!("{:.2} GB", b / (K * K * K))
    }
}

fn hms(secs: u64) -> String {
    format!("{:02}:{:02}:{:02}", secs / 3600, (secs % 3600) / 60, secs % 60)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn df(folder: &str, name: &str) -> DeviceFile {
        DeviceFile { folder: folder.into(), name: name.into() }
    }

    #[test]
    fn key_and_dest_use_folder_basename() {
        let f = df("/store_00010001/202607_a", "MYVX1527.MP4");
        assert_eq!(f.folder_name(), "202607_a");
        assert_eq!(f.key(), "202607_a/MYVX1527.MP4");
        assert_eq!(f.legacy_dest_name(), "202607_a_MYVX1527.MP4");
    }

    /// The destination scheme must match what the bash script produced, or the
    /// 20-odd files it already copied would be re-downloaded instead of adopted.
    #[test]
    fn dest_name_matches_existing_bash_output() {
        assert_eq!(
            df("/store_00010001/202606_a", "CNEM0645.MP4").legacy_dest_name(),
            "202606_a_CNEM0645.MP4"
        );
    }

    /// The store id is not part of the identity: iOS can present a different
    /// store path across reconnects, and re-importing everything because of
    /// that would defeat the whole point.
    #[test]
    fn key_is_stable_across_store_id_changes() {
        let a = df("/store_00010001/202607_a", "A.MP4");
        let b = df("/store_00020001/202607_a", "A.MP4");
        assert_eq!(a.key(), b.key());
    }

    /// Timezone-independent: rather than hardcoding a rendering that only
    /// holds in one zone, parse the generated stamp back and check it names
    /// the same instant we started from.
    #[test]
    fn timestamped_name_round_trips_to_the_same_instant() {
        use chrono::{Local, NaiveDateTime, TimeZone};

        let f = df("/store_00010001/202606_a", "CNEM0645.MP4");
        let epoch = 1_782_865_895i64; // 2026-06-30 20:31:35 -04:00
        let name = f.dest_name(Some(epoch));

        assert!(name.ends_with("_CNEM0645.MP4"), "original name must survive: {name}");
        let stamp = name.strip_suffix("_CNEM0645.MP4").unwrap();

        let naive = NaiveDateTime::parse_from_str(stamp, "%Y-%m-%d_%H-%M-%S")
            .unwrap_or_else(|e| panic!("stamp {stamp:?} not in expected format: {e}"));
        let back = Local.from_local_datetime(&naive).unwrap();
        assert_eq!(back.timestamp(), epoch);
    }

    /// Without a capture time there is nothing to name the file after, so it
    /// must fall back rather than inventing a date.
    #[test]
    fn naming_falls_back_when_capture_time_is_absent_or_bogus() {
        let f = df("/store_00010001/202606_a", "CNEM0645.MP4");
        assert_eq!(f.dest_name(None), "202606_a_CNEM0645.MP4");
        assert_eq!(f.dest_name(Some(0)), "202606_a_CNEM0645.MP4");
        assert_eq!(f.dest_name(Some(-5)), "202606_a_CNEM0645.MP4");
    }

    fn scratch(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("adopt-{tag}-{}", std::process::id()));
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    /// The 35 files the bash script already copied carry legacy names. Failing
    /// to recognise them would re-download ~18GB, so this is the single most
    /// expensive thing to get wrong.
    #[test]
    fn legacy_named_file_is_adopted_not_redownloaded() {
        let d = scratch("legacy");
        let legacy = "202606_a_CNEM0645.MP4";
        std::fs::write(d.join(legacy), vec![0u8; 500]).unwrap();

        let candidates = ["2026-06-30_20-31-35_CNEM0645.MP4".to_string(), legacy.to_string()];
        assert_eq!(
            find_existing(&d, &candidates, Some(500)),
            Some((legacy.to_string(), 500))
        );
        std::fs::remove_dir_all(&d).ok();
    }

    #[test]
    fn timestamped_name_wins_when_both_are_present() {
        let d = scratch("both");
        let stamped = "2026-06-30_20-31-35_CNEM0645.MP4";
        let legacy = "202606_a_CNEM0645.MP4";
        std::fs::write(d.join(stamped), vec![0u8; 500]).unwrap();
        std::fs::write(d.join(legacy), vec![0u8; 500]).unwrap();

        let candidates = [stamped.to_string(), legacy.to_string()];
        assert_eq!(
            find_existing(&d, &candidates, Some(500)).map(|(n, _)| n),
            Some(stamped.to_string())
        );
        std::fs::remove_dir_all(&d).ok();
    }

    /// A truncated file has the right name and the wrong length. Adopting it
    /// would make the corruption permanent.
    #[test]
    fn wrong_size_is_never_adopted() {
        let d = scratch("truncated");
        let legacy = "202606_a_CNEM0645.MP4";
        std::fs::write(d.join(legacy), vec![0u8; 120]).unwrap();

        let candidates = [legacy.to_string()];
        assert_eq!(find_existing(&d, &candidates, Some(500)), None);
        std::fs::remove_dir_all(&d).ok();
    }

    /// No reported size means no way to verify, so nothing may be adopted.
    #[test]
    fn nothing_is_adopted_without_a_reported_size() {
        let d = scratch("nosize");
        let legacy = "202606_a_CNEM0645.MP4";
        std::fs::write(d.join(legacy), vec![0u8; 500]).unwrap();

        let candidates = [legacy.to_string()];
        assert_eq!(find_existing(&d, &candidates, None), None);
        std::fs::remove_dir_all(&d).ok();
    }

    #[test]
    fn absent_file_is_not_adopted() {
        let d = scratch("absent");
        let candidates = ["nope.MP4".to_string()];
        assert_eq!(find_existing(&d, &candidates, Some(500)), None);
        std::fs::remove_dir_all(&d).ok();
    }

    #[test]
    fn extension_matching_is_case_insensitive() {
        let exts = vec![String::from("mp4")];
        assert!(matches_ext("VIDEO.MP4", &exts));
        assert!(matches_ext("video.mp4", &exts));
        assert!(matches_ext("video.Mp4", &exts));
        assert!(!matches_ext("IMG_0001.HEIC", &exts));
        assert!(!matches_ext("noextension", &exts));
    }

    #[test]
    fn extension_matching_honours_multiple_extensions() {
        let exts = vec![String::from("mp4"), String::from("mov")];
        assert!(matches_ext("a.MOV", &exts));
        assert!(matches_ext("a.mp4", &exts));
        assert!(!matches_ext("a.jpg", &exts));
    }

    /// A name that is nothing but an extension still matches. Harmless in
    /// practice (no camera produces one) and documented here so the behaviour
    /// is deliberate rather than accidental.
    #[test]
    fn extension_matching_accepts_bare_dotfile() {
        let exts = vec![String::from("mp4")];
        assert!(matches_ext(".mp4", &exts));
    }

    #[test]
    fn human_size_boundaries() {
        assert_eq!(human_size(0), "0 B");
        assert_eq!(human_size(1023), "1023 B");
        assert_eq!(human_size(1024), "1.0 KB");
        assert_eq!(human_size(1024 * 1024), "1.0 MB");
        assert_eq!(human_size(1024 * 1024 * 1024), "1.00 GB");
        assert_eq!(human_size(1_221_506_443), "1.14 GB");
    }

    /// The mtime is the copy of the capture time users actually see in file
    /// properties, so stamping it must genuinely work rather than silently
    /// no-op.
    #[test]
    fn set_mtime_round_trips() {
        let path = std::env::temp_dir().join(format!("mtime-test-{}", std::process::id()));
        std::fs::write(&path, b"x").unwrap();

        let when = 1782865895_i64; // 2026-06-30 20:31:35 -04:00

        set_mtime(&path, when).unwrap();

        let got = std::fs::metadata(&path)
            .unwrap()
            .modified()
            .unwrap()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();
        std::fs::remove_file(&path).ok();
        assert_eq!(got, when as u64);
    }

    #[test]
    fn hms_formats_hours() {
        assert_eq!(hms(0), "00:00:00");
        assert_eq!(hms(59), "00:00:59");
        assert_eq!(hms(3661), "01:01:01");
        assert_eq!(hms(86_400), "24:00:00");
    }
}
