//! Persistent record of what has already been imported.
//!
//! The bash version decided "have I copied this?" by testing whether the
//! destination filename existed. That answer is wrong in both directions: move
//! the videos to editing storage and everything gets copied again, and a
//! half-written file from an interrupted transfer looks exactly like a finished
//! one.
//!
//! So imports are recorded instead of inferred. The ledger is append-only JSONL
//! living next to the videos, one line per successful import. Append-only means
//! a crash mid-run can at worst lose the last line -- it can never corrupt the
//! history of everything imported before it, which a rewritten JSON blob could.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs::{File, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};

pub const LEDGER_NAME: &str = ".import_ledger.jsonl";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Entry {
    /// Stable identity of the file *on the phone*, e.g. "202607_a/MYVX1527.MP4".
    ///
    /// Deliberately the folder's basename rather than its full path: iOS exposes
    /// folders under a store id (`/store_00010001/...`) that is not guaranteed
    /// stable across reconnects, while the date-named folder is.
    pub key: String,
    /// Full device path at import time, kept for debugging.
    pub folder: String,
    pub name: String,
    pub size: Option<u64>,
    pub mtime: Option<i64>,
    /// Filename within the destination directory -- relative, so the whole
    /// folder can be moved without invalidating the ledger.
    pub dest: String,
    pub imported_at: u64,
}

pub struct Ledger {
    path: PathBuf,
    entries: HashMap<String, Entry>,
}

impl Ledger {
    pub fn load(dir: &Path) -> Result<Self> {
        let path = dir.join(LEDGER_NAME);
        let mut entries = HashMap::new();

        if path.exists() {
            let file = File::open(&path)
                .with_context(|| format!("opening ledger {}", path.display()))?;

            for (n, line) in BufReader::new(file).lines().enumerate() {
                let line = line.with_context(|| format!("reading ledger line {}", n + 1))?;
                if line.trim().is_empty() {
                    continue;
                }
                match serde_json::from_str::<Entry>(&line) {
                    // A later line for the same key supersedes an earlier one,
                    // so re-imports simply overwrite rather than needing the
                    // file to be rewritten.
                    Ok(entry) => {
                        entries.insert(entry.key.clone(), entry);
                    }
                    Err(e) => eprintln!(
                        "warning: ignoring unparseable ledger line {}: {e}",
                        n + 1
                    ),
                }
            }
        }

        Ok(Self { path, entries })
    }

    pub fn get(&self, key: &str) -> Option<&Entry> {
        self.entries.get(key)
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Append one entry and fsync it.
    ///
    /// Synced per entry on purpose: the cost is negligible beside a multi-
    /// hundred-megabyte transfer, and it means a hard power loss cannot leave
    /// a file on disk that the ledger has no record of -- which would silently
    /// re-download it next run.
    pub fn append(&mut self, entry: Entry) -> Result<()> {
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)
            .with_context(|| format!("opening ledger {} for append", self.path.display()))?;

        let line = serde_json::to_string(&entry)?;
        writeln!(file, "{line}")?;
        file.flush()?;
        file.sync_all()?;

        self.entries.insert(entry.key.clone(), entry);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn tmpdir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "ledger-test-{tag}-{}-{}",
            std::process::id(),
            SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn entry(key: &str, size: u64) -> Entry {
        Entry {
            key: key.into(),
            folder: "/store_00010001/202607_a".into(),
            name: key.rsplit('/').next().unwrap().into(),
            size: Some(size),
            mtime: Some(1_751_000_000),
            dest: key.replace('/', "_"),
            imported_at: 1_756_000_000,
        }
    }

    #[test]
    fn missing_ledger_loads_empty() {
        let dir = tmpdir("empty");
        let l = Ledger::load(&dir).unwrap();
        assert_eq!(l.len(), 0);
        assert!(l.get("202607_a/X.MP4").is_none());
    }

    #[test]
    fn appended_entries_survive_reload() {
        let dir = tmpdir("reload");
        {
            let mut l = Ledger::load(&dir).unwrap();
            l.append(entry("202607_a/A.MP4", 111)).unwrap();
            l.append(entry("202607_a/B.MP4", 222)).unwrap();
            assert_eq!(l.len(), 2);
        }
        let l = Ledger::load(&dir).unwrap();
        assert_eq!(l.len(), 2);
        assert_eq!(l.get("202607_a/A.MP4").unwrap().size, Some(111));
        assert_eq!(l.get("202607_a/B.MP4").unwrap().dest, "202607_a_B.MP4");
    }

    #[test]
    fn later_line_supersedes_earlier_for_same_key() {
        let dir = tmpdir("supersede");
        {
            let mut l = Ledger::load(&dir).unwrap();
            l.append(entry("202607_a/A.MP4", 111)).unwrap();
            l.append(entry("202607_a/A.MP4", 999)).unwrap();
        }
        let l = Ledger::load(&dir).unwrap();
        assert_eq!(l.len(), 1, "same key must not double-count");
        assert_eq!(l.get("202607_a/A.MP4").unwrap().size, Some(999));
    }

    /// A crash mid-write can leave a truncated final line. That must cost us
    /// exactly that one record, not the entire import history.
    #[test]
    fn truncated_trailing_line_does_not_lose_earlier_entries() {
        let dir = tmpdir("truncated");
        {
            let mut l = Ledger::load(&dir).unwrap();
            l.append(entry("202607_a/A.MP4", 111)).unwrap();
            l.append(entry("202607_a/B.MP4", 222)).unwrap();
        }
        let path = dir.join(LEDGER_NAME);
        let mut raw = std::fs::read_to_string(&path).unwrap();
        raw.push_str("{\"key\":\"202607_a/C.MP4\",\"size\":33");
        std::fs::write(&path, raw).unwrap();

        let l = Ledger::load(&dir).unwrap();
        assert_eq!(l.len(), 2);
        assert!(l.get("202607_a/A.MP4").is_some());
        assert!(l.get("202607_a/B.MP4").is_some());
        assert!(l.get("202607_a/C.MP4").is_none());
    }

    #[test]
    fn blank_lines_are_ignored() {
        let dir = tmpdir("blank");
        let path = dir.join(LEDGER_NAME);
        let line = serde_json::to_string(&entry("202607_a/A.MP4", 111)).unwrap();
        std::fs::write(&path, format!("\n{line}\n\n")).unwrap();
        assert_eq!(Ledger::load(&dir).unwrap().len(), 1);
    }
}
