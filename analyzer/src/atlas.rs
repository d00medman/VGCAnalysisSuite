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
use std::collections::HashMap;
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
    packed: Packed,
}

impl Template {
    pub fn new(label: String, top: i32, w: u32, h: u32, bits: Vec<bool>) -> Self {
        let packed = Packed::new(&bits, w, h);
        Self { label, top, w, h, bits, packed }
    }
}

/// A bitmap as one `u128` per scanline (bit `x` = column `x`) plus its ink count, so overlap
/// is a popcount per row instead of a test per pixel. `None` when a bitmap is too wide to
/// shift by a pixel within 128 bits; scoring then falls back to the per-pixel path.
#[derive(Clone, Debug)]
struct Packed(Option<(Vec<u128>, u32)>);

impl Packed {
    fn new(bits: &[bool], w: u32, h: u32) -> Self {
        if w >= 128 {
            return Packed(None);
        }
        let rows: Vec<u128> = (0..h)
            .map(|y| {
                let row = &bits[(y * w) as usize..((y + 1) * w) as usize];
                row.iter().enumerate().fold(0u128, |acc, (x, &b)| acc | ((b as u128) << x))
            })
            .collect();
        let ones = rows.iter().map(|r| r.count_ones()).sum();
        Packed(Some((rows, ones)))
    }
}

/// Classification results for glyph bitmaps already seen. The same glyphs recur frame after
/// frame while a message is typed and held, so most lookups skip template matching. Owned by
/// one reader (one per worker thread); `classify` is pure, so caching never changes a reading.
#[derive(Default)]
pub struct GlyphCache(HashMap<(i32, u32, u32, Vec<u128>), Option<(usize, f32)>>);

impl GlyphCache {
    /// Scenery and fades produce endless unique fragments; drop everything past this.
    const MAX_ENTRIES: usize = 100_000;
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
fn score(a_bits: &[bool], a_packed: &Packed, aw: u32, ah: u32, b: &Template) -> f32 {
    match (&a_packed.0, &b.packed.0) {
        (Some((a, a_ones)), Some((bp, b_ones))) => score_packed(a, *a_ones, bp, *b_ones),
        _ => score_scalar(a_bits, aw, ah, b),
    }
}

/// `score` on packed rows. Bits outside either bitmap are zero, so the union over the shared
/// bounding box is `|a| + |b| - |a ∩ b|` and only the intersection needs counting. The integer
/// counts equal the per-pixel path's, so the scores are bit-identical.
fn score_packed(a: &[u128], a_ones: u32, b: &[u128], b_ones: u32) -> f32 {
    let mut best = 0.0f32;
    for dy in -1i32..=1 {
        for dx in -1i32..=1 {
            let mut inter = 0u32;
            for (y, &ar) in a.iter().enumerate() {
                let by = y as i32 - dy;
                if by < 0 || by as usize >= b.len() {
                    continue;
                }
                let br = b[by as usize];
                let br = if dx >= 0 { br << dx } else { br >> -dx };
                inter += (ar & br).count_ones();
            }
            let union = a_ones + b_ones - inter;
            if union > 0 {
                best = best.max(inter as f32 / union as f32);
            }
        }
    }
    best
}

fn score_scalar(a_bits: &[bool], aw: u32, ah: u32, b: &Template) -> f32 {
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
        let packed = Packed::new(&g.bits, g.w, g.h);
        self.classify_packed(g, &packed, baseline).map(|(i, s)| (self.templates[i].label.as_str(), s))
    }

    /// `classify`, returning the template index.
    fn classify_packed(&self, g: &Glyph, packed: &Packed, baseline: i32) -> Option<(usize, f32)> {
        let top = g.y0 - baseline;
        let mut best: Option<(usize, f32)> = None;
        for (i, t) in self.templates.iter().enumerate() {
            if (t.w as i32 - g.w as i32).abs() > 3
                || (t.h as i32 - g.h as i32).abs() > 3
                || (t.top - top).abs() > 3
            {
                continue;
            }
            let s = score(&g.bits, packed, g.w, g.h, t);
            if best.map_or(true, |(_, b)| s > b) {
                best = Some((i, s));
            }
        }
        best
    }

    pub fn read_rows(&self, rows: &[Row]) -> Reading {
        self.read_rows_cached(rows, &mut GlyphCache::default())
    }

    /// `classify` through `cache`.
    fn classify_cached(&self, g: &Glyph, baseline: i32, cache: &mut GlyphCache) -> Option<(&str, f32)> {
        let packed = Packed::new(&g.bits, g.w, g.h);
        let hit = match &packed.0 {
            Some((rows, _)) => {
                let key = (g.y0 - baseline, g.w, g.h, rows.clone());
                match cache.0.get(&key) {
                    Some(hit) => *hit,
                    None => {
                        let r = self.classify_packed(g, &packed, baseline);
                        if cache.0.len() >= GlyphCache::MAX_ENTRIES {
                            cache.0.clear();
                        }
                        cache.0.insert(key, r);
                        r
                    }
                }
            }
            None => self.classify_packed(g, &packed, baseline),
        };
        hit.map(|(i, s)| (self.templates[i].label.as_str(), s))
    }

    /// `read_rows`, reusing classifications from `cache`.
    pub fn read_rows_cached(&self, rows: &[Row], cache: &mut GlyphCache) -> Reading {
        let mut text = String::new();
        let mut conf = 1.0f32;
        let mut unknown = 0;
        for (ri, row) in rows.iter().enumerate() {
            if ri > 0 {
                text.push(' ');
            }
            let mut prev_label = "";
            for (gi, g) in row.glyphs.iter().enumerate() {
                let (label, s) = match self.classify_cached(g, row.baseline, cache) {
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
        let packed = Packed::new(&g.bits, g.w, g.h);
        let dup = self.templates.iter().any(|t| {
            t.label == label
                && (t.top - top).abs() <= 1
                && score(&g.bits, &packed, g.w, g.h, t) >= DEDUPE_SCORE
        });
        if dup {
            return false;
        }
        self.templates.push(Template::new(label.to_string(), top, g.w, g.h, g.bits.clone()));
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
                atlas.templates.push(Template::new(label, top, w, h, bits));
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

#[cfg(test)]
mod tests {
    use super::*;

    /// Deterministic pseudo-random bitmap with roughly `density` ink.
    fn bitmap(seed: &mut u64, w: u32, h: u32, density: u64) -> Vec<bool> {
        (0..w * h)
            .map(|_| {
                *seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
                (*seed >> 33) % 100 < density
            })
            .collect()
    }

    #[test]
    fn packed_score_matches_per_pixel_score() {
        let mut seed = 7;
        for i in 0..2000u32 {
            let (aw, ah) = (1 + i % 50, 1 + (i / 3) % 60);
            let (bw, bh) = (1 + (i / 7) % 50, 1 + (i / 11) % 60);
            let density = [5, 30, 60, 95][(i % 4) as usize];
            let a = bitmap(&mut seed, aw, ah, density);
            let t = Template::new("x".into(), 0, bw, bh, bitmap(&mut seed, bw, bh, density));
            let packed = Packed::new(&a, aw, ah);
            assert!(packed.0.is_some());
            assert_eq!(
                score(&a, &packed, aw, ah, &t).to_bits(),
                score_scalar(&a, aw, ah, &t).to_bits(),
                "{aw}x{ah} vs {bw}x{bh}"
            );
        }
    }
}
