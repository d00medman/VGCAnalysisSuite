//! The transcription loop as a library call, shared by the CLI and the web server.
//!
//! Frames are independent until the episode tracker, so the work is pipelined: one thread
//! reads frames from ffmpeg, a pool of workers segments and reads them, and the calling thread
//! puts the readings back in frame order and feeds the tracker. The output is identical to
//! reading the frames one at a time.

use crate::atlas::{Atlas, GlyphCache, Reading};
use crate::decode::{Decoder, Extras, Frame};
use crate::episode::{Message, Tracker};
use crate::text::{self, Image, MESSAGE_ROI};
use anyhow::Result;
use std::collections::BTreeMap;
use std::path::Path;
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::time::Instant;

/// Where a running transcription is, reported once per decoded frame.
#[derive(Clone, Copy, Debug, Default, serde::Serialize)]
pub struct Status {
    /// Video time of the frame just processed, in seconds.
    pub t: f64,
    /// Video time the run started at.
    pub t_start: f64,
    /// How much video the run covers, once known (from `duration` or the file header).
    pub span: Option<f64>,
    pub frames: u64,
    pub messages: usize,
    pub timing: Timing,
}

/// Seconds spent per stage so far. `decode` is time the reader thread waited on ffmpeg for
/// frames; `segment` and `read` are summed over all workers, so together they can exceed
/// wall-clock time.
#[derive(Clone, Copy, Debug, Default, serde::Serialize)]
pub struct Timing {
    pub decode: f64,
    pub segment: f64,
    pub read: f64,
    pub workers: usize,
}

impl Status {
    /// Fraction done in `[0, 1]`, once the span is known.
    pub fn fraction(&self) -> Option<f64> {
        self.span.filter(|s| *s > 0.0).map(|s| ((self.t - self.t_start) / s).clamp(0.0, 1.0))
    }
}

/// Analysis threads: `ANALYZER_JOBS` if set, else half the cores (ffmpeg's HEVC decoder
/// wants the rest), at most 6.
fn worker_count() -> usize {
    if let Some(n) = std::env::var("ANALYZER_JOBS").ok().and_then(|v| v.parse().ok()) {
        return std::cmp::max(n, 1);
    }
    let cores = std::thread::available_parallelism().map_or(2, |n| n.get());
    (cores / 2).clamp(1, 6)
}

/// One frame's reading, tagged with its position in the stream.
struct Analyzed {
    seq: u64,
    t: f64,
    reading: Option<Reading>,
    decode: f64,
    segment: f64,
    read: f64,
}

/// Transcribe the message line of `video`, optionally limited to
/// `[start, start + duration)`.
///
/// `on_frame` is called after every decoded frame and `on_message` as soon as each message
/// is final, so callers can stream results. Returns every message, in order.
pub fn transcribe(
    ffmpeg: &Path,
    atlas: &Atlas,
    video: &Path,
    start: Option<f64>,
    duration: Option<f64>,
    on_frame: impl FnMut(&Status),
    on_message: impl FnMut(&Message),
) -> Result<Vec<Message>> {
    transcribe_with(ffmpeg, atlas, video, start, duration, &Extras::default(), on_frame, on_message)
}

/// As `transcribe`, also writing the decoder side outputs in `extras` (a playable preview,
/// a live snapshot) from the same decode.
#[allow(clippy::too_many_arguments)]
pub fn transcribe_with(
    ffmpeg: &Path,
    atlas: &Atlas,
    video: &Path,
    start: Option<f64>,
    duration: Option<f64>,
    extras: &Extras,
    mut on_frame: impl FnMut(&Status),
    mut on_message: impl FnMut(&Message),
) -> Result<Vec<Message>> {
    let dec = Decoder::open_with(ffmpeg, video, MESSAGE_ROI, start, duration, extras)?;
    // The file duration arrives on ffmpeg's stderr as the input opens.
    let file_duration = dec.duration_cell();
    let workers = worker_count();
    let t_start = start.unwrap_or(0.0);
    let mut status = Status { t: t_start, t_start, span: duration, ..Default::default() };
    status.timing.workers = workers;
    let mut tracker = Tracker::default();
    let mut messages = Vec::new();
    let mut keep = |m: Message, status: &mut Status, messages: &mut Vec<Message>| {
        on_message(&m);
        messages.push(m);
        status.messages = messages.len();
    };

    std::thread::scope(|scope| -> Result<()> {
        // Bounded, so the reader stays only a few frames ahead of the workers.
        let (frame_tx, frame_rx) = mpsc::sync_channel::<(u64, Frame, f64)>(workers * 4);
        // Shared by the workers only: once they all exit, the receiver drops and a blocked
        // reader sees the disconnect instead of waiting forever.
        let frame_rx = Arc::new(Mutex::new(frame_rx));
        let (result_tx, result_rx) = mpsc::channel::<Analyzed>();

        let reader = scope.spawn(move || -> Result<()> {
            let mut dec = dec;
            let mut seq = 0;
            loop {
                let started = Instant::now();
                let Some(f) = dec.next_frame()? else { return Ok(()) };
                if frame_tx.send((seq, f, started.elapsed().as_secs_f64())).is_err() {
                    return Ok(());
                }
                seq += 1;
            }
        });

        for _ in 0..workers {
            let frame_rx = Arc::clone(&frame_rx);
            let result_tx = result_tx.clone();
            scope.spawn(move || {
                let mut cache = GlyphCache::default();
                loop {
                    let next = frame_rx.lock().unwrap().recv();
                    let Ok((seq, f, decode)) = next else { break };
                    let started = Instant::now();
                    let img = Image { w: MESSAGE_ROI.w, h: MESSAGE_ROI.h, rgb: &f.rgb };
                    let rows = text::text_rows(&img);
                    let segmented = Instant::now();
                    let reading = (!rows.is_empty()).then(|| atlas.read_rows_cached(&rows, &mut cache));
                    let done = Analyzed {
                        seq,
                        t: f.t,
                        reading,
                        decode,
                        segment: (segmented - started).as_secs_f64(),
                        read: segmented.elapsed().as_secs_f64(),
                    };
                    if result_tx.send(done).is_err() {
                        break;
                    }
                }
            });
        }
        drop((frame_rx, result_tx));

        // Workers finish out of order; hold results until the next frame in sequence arrives.
        let mut pending = BTreeMap::new();
        let mut next = 0;
        for a in result_rx {
            pending.insert(a.seq, a);
            while let Some(a) = pending.remove(&next) {
                next += 1;
                if status.span.is_none() {
                    status.span = file_duration.get().map(|d| d - t_start);
                }
                if let Some(m) = tracker.push(a.t, a.reading.as_ref()) {
                    keep(m, &mut status, &mut messages);
                }
                status.t = a.t;
                status.frames += 1;
                status.timing.decode += a.decode;
                status.timing.segment += a.segment;
                status.timing.read += a.read;
                on_frame(&status);
            }
        }
        match reader.join() {
            Ok(r) => r,
            Err(panic) => std::panic::resume_unwind(panic),
        }
    })?;

    for m in tracker.flush() {
        keep(m, &mut status, &mut messages);
    }
    on_frame(&status);
    Ok(messages)
}
