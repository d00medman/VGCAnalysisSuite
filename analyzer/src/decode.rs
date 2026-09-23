//! ffmpeg subprocess decode with presentation-timestamp-correct frame iteration.
//!
//! The recordings are VFR (iOS drops frames while the screen is static), so frame index is
//! not proportional to time. Every frame is paired with its PTS as reported by ffmpeg's
//! `showinfo` filter on stderr — never derived from a counter and a nominal rate.

use anyhow::{bail, Context, Result};
use std::io::{BufRead, BufReader, Read};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdout, Command, Stdio};
use std::sync::mpsc::{self, Receiver};
use std::sync::{Arc, OnceLock};
use std::thread::JoinHandle;

/// A region of the decoded, auto-rotated (landscape 2436x1126) frame.
#[derive(Clone, Copy, Debug)]
pub struct Rect {
    pub x: u32,
    pub y: u32,
    pub w: u32,
    pub h: u32,
}

pub struct Frame {
    /// Presentation time in seconds.
    pub t: f64,
    /// Packed RGB24, `rect.w * rect.h * 3` bytes.
    pub rgb: Vec<u8>,
}

pub struct Decoder {
    child: Child,
    stdout: ChildStdout,
    pts: Receiver<f64>,
    stderr_thread: Option<JoinHandle<String>>,
    /// Input duration from ffmpeg's `Duration:` header line, once it has been printed.
    duration: Arc<OnceLock<f64>>,
    frame_len: usize,
}

/// Locate the pinned ffmpeg: explicit path, else `vendor/ffmpeg` beside this crate, else PATH.
pub fn ffmpeg_path(explicit: Option<&Path>) -> PathBuf {
    if let Some(p) = explicit {
        return p.to_path_buf();
    }
    let vendored = Path::new(env!("CARGO_MANIFEST_DIR")).join("vendor/ffmpeg");
    if vendored.exists() {
        vendored
    } else {
        PathBuf::from("ffmpeg")
    }
}

impl Decoder {
    /// Decode `video`, cropped to `rect`, optionally limited to `[start, start+duration)`.
    pub fn open(
        ffmpeg: &Path,
        video: &Path,
        rect: Rect,
        start: Option<f64>,
        duration: Option<f64>,
    ) -> Result<Self> {
        let mut cmd = Command::new(ffmpeg);
        cmd.args(["-hide_banner", "-nostats", "-loglevel", "info"]);
        if let Some(s) = start {
            // Input-side seek, with -copyts so reported PTS stay in file time.
            cmd.args(["-ss", &format!("{s}"), "-copyts"]);
        }
        if let Some(d) = duration {
            // Input-side too, so it counts from the seek point regardless of -copyts.
            cmd.args(["-t", &format!("{d}")]);
        }
        cmd.arg("-i").arg(video);
        cmd.args([
            "-map",
            "0:v:0",
            "-an",
            // Passthrough: emit exactly the decoded frames, never duplicate or drop to hit a rate.
            "-fps_mode",
            "passthrough",
            "-vf",
            &format!("crop={}:{}:{}:{},showinfo", rect.w, rect.h, rect.x, rect.y),
            "-f",
            "rawvideo",
            "-pix_fmt",
            "rgb24",
            "-",
        ]);
        let mut child = cmd
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .with_context(|| format!("spawning {}", ffmpeg.display()))?;

        let stdout = child.stdout.take().expect("piped stdout");
        let stderr = child.stderr.take().expect("piped stderr");
        let (tx, rx) = mpsc::channel();
        let duration = Arc::new(OnceLock::new());
        let duration_w = Arc::clone(&duration);
        let stderr_thread = std::thread::spawn(move || {
            // Forward showinfo PTS; keep the non-showinfo tail for error reporting.
            let mut tail = String::new();
            for line in BufReader::new(stderr).lines().map_while(|l| l.ok()) {
                if line.contains("Parsed_showinfo") {
                    if let Some(t) = parse_pts_time(&line) {
                        if tx.send(t).is_err() {
                            break;
                        }
                    }
                } else {
                    if let Some(d) = parse_duration(&line) {
                        let _ = duration_w.set(d);
                    }
                    tail.push_str(&line);
                    tail.push('\n');
                    if tail.len() > 16_384 {
                        tail.drain(..tail.len() - 8_192);
                    }
                }
            }
            tail
        });

        Ok(Self {
            child,
            stdout,
            pts: rx,
            stderr_thread: Some(stderr_thread),
            duration,
            frame_len: (rect.w * rect.h * 3) as usize,
        })
    }

    /// Length of the whole input file in seconds, if ffmpeg has reported it yet. It is printed
    /// when the input is opened, so it is available by the first frame.
    pub fn duration(&self) -> Option<f64> {
        self.duration.get().copied()
    }

    /// Next frame, or `None` at end of stream.
    pub fn next_frame(&mut self) -> Result<Option<Frame>> {
        let mut rgb = vec![0u8; self.frame_len];
        let mut filled = 0;
        while filled < rgb.len() {
            let n = self.stdout.read(&mut rgb[filled..])?;
            if n == 0 {
                break;
            }
            filled += n;
        }
        if filled == 0 {
            self.finish()?;
            return Ok(None);
        }
        if filled < rgb.len() {
            bail!("truncated frame from ffmpeg ({filled} of {} bytes)", rgb.len());
        }
        // showinfo logs a frame before it reaches the encoder, so its PTS is already sent
        // (or will be momentarily) by the time the pixels arrive.
        let t = self
            .pts
            .recv()
            .context("ffmpeg produced a frame without a showinfo timestamp")?;
        Ok(Some(Frame { t, rgb }))
    }

    fn finish(&mut self) -> Result<()> {
        let status = self.child.wait()?;
        let tail = self
            .stderr_thread
            .take()
            .map(|h| h.join().unwrap_or_default())
            .unwrap_or_default();
        if !status.success() {
            bail!("ffmpeg exited with {status}:\n{tail}");
        }
        Ok(())
    }
}

impl Drop for Decoder {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// `  Duration: 00:09:39.54, start: 0.000000, bitrate: ...` → seconds.
fn parse_duration(line: &str) -> Option<f64> {
    let rest = line.trim_start().strip_prefix("Duration: ")?;
    let hms = rest.split(',').next()?;
    let mut parts = hms.split(':');
    let h: f64 = parts.next()?.parse().ok()?;
    let m: f64 = parts.next()?.parse().ok()?;
    let s: f64 = parts.next()?.parse().ok()?;
    Some(h * 3600.0 + m * 60.0 + s)
}

fn parse_pts_time(line: &str) -> Option<f64> {
    let rest = &line[line.find("pts_time:")? + "pts_time:".len()..];
    let end = rest.find(char::is_whitespace).unwrap_or(rest.len());
    rest[..end].parse().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_showinfo_line() {
        let l = "[Parsed_showinfo_1 @ 0x5] n:  12 pts:  48000 pts_time:0.8     duration:600";
        assert_eq!(parse_pts_time(l), Some(0.8));
    }

    #[test]
    fn parses_duration_header() {
        let l = "  Duration: 00:09:39.54, start: 0.000000, bitrate: 15112 kb/s";
        assert_eq!(parse_duration(l), Some(579.54));
        assert_eq!(parse_duration("  Stream #0:0: Video: hevc"), None);
    }
}
