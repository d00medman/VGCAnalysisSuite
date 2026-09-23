//! Glyph template atlas: nearest-neighbour matching of segmented glyphs against labelled
//! bitmaps. No ML — the font is fixed and rendered at one size, so a glyph is identified by
//! overlap with stored examples.
//!
//! On-disk format is plain text so the atlas diffs and reviews like source:
//!
//! ```text
//! space_gap 9
//! glyph "T" top -30
//! ####....
//! ```
//!
//! `top` is the glyph's top edge relative to its row's baseline, which is what tells `,` from `'`.
//!
//! Labels are arbitrary Unicode strings, so Japanese and Korean characters can be taught the
//! same way as Latin ones. An unrecognised glyph always reads as `?` and is never dropped;
//! `harvest --unknown-only` collects exactly those crops for labelling. See the
//! TODO(cjk) note in `text` for the multi-piece character gap.

use crate::text::{Glyph, Row};
use anyhow::{bail, Context, Result};
use std::fmt::Write as _;
use std::path::Path;

/// Below this overlap a glyph is reported as unknown (`?`).
pub const MIN_SCORE: f32 = 0.7;
/// Two samples of one label at least this similar are kept as one template.
const DEDUPE_SCORE: f32 = 0.92;

#[derive(Clone, Debug)]
pub struct Template {
    pub label: String,
    pub top: i32,
    pub w: u32,
    pub h: u32,
    pub bits: Vec<bool>,
}

#[derive(Default)]
pub struct Atlas {
    /// A horizontal gap of at least this many pixels between glyphs is a space.
    pub space_gap: i32,
    pub templates: Vec<Template>,
}

/// Extra side padding a glyph carries beyond its ink. Digits are tabular (fixed advance),
/// so the narrow `1` sits in a wide cell: measured `61` and `19` gaps of 10–12px against
/// ordinary letter gaps of ≤8, which would otherwise read as word spaces.
pub fn side_bearing(label: &str) -> i32 {
    if label == "1" { 5 } else { 0 }
}

pub struct Reading {
    pub text: String,
    /// Lowest per-glyph score in the reading.
    pub conf: f32,
    pub unknown: usize,
}

/// Overlap (intersection over union) of two bitmaps, best over ±1px shifts.
fn score(a_bits: &[bool], aw: u32, ah: u32, b: &Template) -> f32 {
    let mut best = 0.0f32;
    for dy in -1i32..=1 {
        for dx in -1i32..=1 {
            let (mut inter, mut union) = (0u32, 0u32);
            let x_lo = 0.min(dx);
            let y_lo = 0.min(dy);
            let x_hi = (aw as i32).max(dx + b.w as i32);
            let y_hi = (ah as i32).max(dy + b.h as i32);
            for y in y_lo..y_hi {
                for x in x_lo..x_hi {
                    let pa = x >= 0 && y >= 0 && (x as u32) < aw && (y as u32) < ah
                        && a_bits[(y as u32 * aw + x as u32) as usize];
                    let (bx, by) = (x - dx, y - dy);
                    let pb = bx >= 0 && by >= 0 && (bx as u32) < b.w && (by as u32) < b.h
                        && b.bits[(by as u32 * b.w + bx as u32) as usize];
                    inter += (pa && pb) as u32;
                    union += (pa || pb) as u32;
                }
            }
            if union > 0 {
                best = best.max(inter as f32 / union as f32);
            }
        }
    }
    best
}

impl Atlas {
    /// Best-matching template for a glyph, as (label, score). Ties go to the earlier template,
    /// so results are deterministic for a given atlas file.
    pub fn classify(&self, g: &Glyph, baseline: i32) -> Option<(&str, f32)> {
        let top = g.y0 - baseline;
        let mut best: Option<(&str, f32)> = None;
        for t in &self.templates {
            if (t.w as i32 - g.w as i32).abs() > 3
                || (t.h as i32 - g.h as i32).abs() > 3
                || (t.top - top).abs() > 3
            {
                continue;
            }
            let s = score(&g.bits, g.w, g.h, t);
            if best.map_or(true, |(_, b)| s > b) {
                best = Some((&t.label, s));
            }
        }
        best
    }

    pub fn read_rows(&self, rows: &[Row]) -> Reading {
        let mut text = String::new();
        let mut conf = 1.0f32;
        let mut unknown = 0;
        for (ri, row) in rows.iter().enumerate() {
            if ri > 0 {
                text.push(' ');
            }
            let mut prev_label = "";
            for (gi, g) in row.glyphs.iter().enumerate() {
                let (label, s) = match self.classify(g, row.baseline) {
                    Some((label, s)) if s >= MIN_SCORE => (label, s),
                    other => {
                        unknown += 1;
                        ("?", other.map_or(0.0, |(_, s)| s))
                    }
                };
                if gi > 0 {
                    let gap = crate::text::ink_gap(&row.glyphs[gi - 1], g)
                        - side_bearing(prev_label)
                        - side_bearing(label);
                    if gap >= self.space_gap {
                        text.push(' ');
                    }
                }
                text.push_str(label);
                conf = conf.min(s);
                prev_label = label;
            }
        }
        Reading { text, conf, unknown }
    }

    /// Add a labelled sample unless an equivalent template for the same label exists.
    pub fn add(&mut self, label: &str, g: &Glyph, baseline: i32) -> bool {
        let top = g.y0 - baseline;
        let dup = self.templates.iter().any(|t| {
            t.label == label
                && (t.top - top).abs() <= 1
                && score(&g.bits, g.w, g.h, t) >= DEDUPE_SCORE
        });
        if dup {
            return false;
        }
        self.templates.push(Template {
            label: label.to_string(),
            top,
            w: g.w,
            h: g.h,
            bits: g.bits.clone(),
        });
        true
    }

    pub fn load(path: &Path) -> Result<Self> {
        let src = std::fs::read_to_string(path)
            .with_context(|| format!("reading atlas {}", path.display()))?;
        let mut atlas = Atlas::default();
        let mut lines = src.lines().enumerate().peekable();
        while let Some((n, line)) = lines.next() {
            let line = line.trim_end();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            if let Some(v) = line.strip_prefix("space_gap ") {
                atlas.space_gap = v.trim().parse().context("space_gap")?;
            } else if let Some(rest) = line.strip_prefix("glyph ") {
                // glyph "<json label>" top <n>
                let end = rest.rfind(" top ").with_context(|| format!("line {}", n + 1))?;
                let label: String = serde_json::from_str(&rest[..end])
                    .with_context(|| format!("label on line {}", n + 1))?;
                let top: i32 = rest[end + 5..].trim().parse()?;
                let mut rows: Vec<&str> = Vec::new();
                while let Some((_, l)) = lines.peek() {
                    let l = l.trim_end();
                    if !l.is_empty() && l.bytes().all(|b| b == b'#' || b == b'.') {
                        rows.push(l);
                        lines.next();
                    } else {
                        break;
                    }
                }
                let h = rows.len() as u32;
                let w = rows.first().map_or(0, |r| r.len()) as u32;
                if h == 0 || rows.iter().any(|r| r.len() as u32 != w) {
                    bail!("malformed bitmap for glyph on line {}", n + 1);
                }
                let bits = rows.iter().flat_map(|r| r.bytes().map(|b| b == b'#')).collect();
                atlas.templates.push(Template { label, top, w, h, bits });
            } else {
                bail!("unrecognised atlas line {}: {line}", n + 1);
            }
        }
        Ok(atlas)
    }

    pub fn save(&self, path: &Path) -> Result<()> {
        let mut out = String::from("# analyzer glyph atlas — generated by `analyzer atlas`\n");
        writeln!(out, "space_gap {}", self.space_gap)?;
        let mut sorted: Vec<&Template> = self.templates.iter().collect();
        sorted.sort_by(|a, b| a.label.cmp(&b.label));
        for t in sorted {
            writeln!(out, "glyph {} top {}", serde_json::to_string(&t.label)?, t.top)?;
            for y in 0..t.h {
                for x in 0..t.w {
                    out.push(if t.bits[(y * t.w + x) as usize] { '#' } else { '.' });
                }
                out.push('\n');
            }
        }
        std::fs::write(path, out).with_context(|| format!("writing {}", path.display()))
    }
}
