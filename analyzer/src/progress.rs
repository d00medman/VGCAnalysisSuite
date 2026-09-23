//! Progress reporting on stderr, so a multi-minute decode is visibly alive.
//!
//! On a terminal the status line is redrawn in place; when stderr is redirected to a file,
//! one line is written per update instead, so logs stay readable. stdout is never touched —
//! it carries the transcript.

use std::io::{IsTerminal, Write};
use std::time::{Duration, Instant};

/// Wall-clock time between status updates.
const INTERVAL: Duration = Duration::from_secs(2);

pub struct Progress {
    enabled: bool,
    tty: bool,
    started: Instant,
    last: Instant,
    /// Video time the run starts at (`--ss`), and how much video it will cover.
    t_start: f64,
    span: Option<f64>,
    frames: u64,
    /// Draw on the next `frame` regardless of `INTERVAL` (the line was just cleared).
    redraw: bool,
}

pub fn clock(t: f64) -> String {
    let t = t.max(0.0);
    let m = (t / 60.0).floor() as u64;
    format!("{m:02}:{:05.2}", t - m as f64 * 60.0)
}

fn short(t: f64) -> String {
    let t = t.max(0.0).round() as u64;
    format!("{}:{:02}", t / 60, t % 60)
}

impl Progress {
    pub fn new(enabled: bool, t_start: f64, span: Option<f64>) -> Self {
        let now = Instant::now();
        Self {
            enabled,
            tty: std::io::stderr().is_terminal(),
            started: now,
            last: now,
            t_start,
            span,
            frames: 0,
            redraw: false,
        }
    }

    /// Log a one-off line (start-up details, completion summary).
    pub fn log(&self, msg: &str) {
        if self.enabled {
            let mut e = std::io::stderr().lock();
            if self.tty {
                let _ = write!(e, "\r\x1b[2K");
            }
            let _ = writeln!(e, "[analyzer] {msg}");
        }
    }

    /// Erase the in-place status line so other output (the transcript on stdout) starts on a
    /// clean line; the status is redrawn on the next frame.
    pub fn clear_line(&mut self) {
        if self.enabled && self.tty {
            let mut e = std::io::stderr().lock();
            let _ = write!(e, "\r\x1b[2K");
            let _ = e.flush();
            self.redraw = true;
        }
    }

    /// Fill in the span once the decoder knows the file's duration.
    pub fn set_span_if_unknown(&mut self, span: Option<f64>) {
        if self.span.is_none() {
            self.span = span;
        }
    }

    /// Call once per decoded frame with its timestamp; prints at most every `INTERVAL`.
    pub fn frame(&mut self, t: f64, messages: usize) {
        self.frames += 1;
        if !self.enabled || (!self.redraw && self.last.elapsed() < INTERVAL) {
            return;
        }
        self.redraw = false;
        self.last = Instant::now();
        let done = (t - self.t_start).max(0.0);
        let wall = self.started.elapsed().as_secs_f64();
        let speed = if wall > 0.0 { done / wall } else { 0.0 };
        let mut line = format!("{} ", clock(t));
        if let Some(span) = self.span {
            let pct = (done / span * 100.0).min(100.0);
            let eta = if speed > 0.0 { (span - done).max(0.0) / speed } else { 0.0 };
            line += &format!("/ {} ({pct:4.1}%) · eta {}", clock(self.t_start + span), short(eta));
        }
        line += &format!(" · {speed:.1}x realtime · {} frames · {messages} messages", self.frames);
        let mut e = std::io::stderr().lock();
        if self.tty {
            let _ = write!(e, "\r\x1b[2K[analyzer] {line}");
        } else {
            let _ = writeln!(e, "[analyzer] {line}");
        }
        let _ = e.flush();
    }

    pub fn finish(&self, messages: usize) {
        let wall = self.started.elapsed().as_secs_f64();
        self.log(&format!(
            "done: {} frames, {messages} messages in {}",
            self.frames,
            short(wall)
        ));
    }
}
