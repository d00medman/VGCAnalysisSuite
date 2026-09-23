//! Text isolation and glyph segmentation for the message line.
//!
//! Message text is near-white, low-saturation, anti-aliased, and drawn over a darkened band.
//! A pixel is "ink" when it is bright on every channel; a connected blob of ink is only kept
//! when its surroundings mostly contrast with it, which rejects bright white scenery (Hyper
//! Beam flashes, explosions) that would otherwise pass the brightness test. Rows are then
//! kept only when they start at the fixed left margin of the message line.
//!
//! **Non-Latin scripts.** Player names and nicknames can be Japanese or Korean, so nothing
//! here may decide "is this text" by whether the glyphs are *recognised* — that would silently
//! drop any message containing a character the atlas has not been taught. Every gate in this
//! file is geometric (brightness, contrast, size, position) and script-agnostic; recognition
//! happens later, in `atlas`, and an unrecognised glyph becomes `?`, never a dropped row.
//! Size limits are set in *em* (the font's full character width, ≈45–47px here), not from
//! Latin letter widths, because full-width CJK characters are about one em wide.
//!
//! TODO(cjk): no Japanese/Korean sample has been harvested yet, so none of this is verified
//! against real CJK text. Known gap: many kana, kanji and hangul are built from side-by-side
//! pieces (い, 川, 사). `segment` merges only *vertically* stacked blobs, so such a character
//! currently comes out as several glyphs. Fixing that needs real samples: a label syntax
//! naming how many pieces make one character, and composite templates the reader can try
//! against runs of adjacent unknown pieces.

use crate::decode::Rect;

/// Message line ROI in decoded landscape pixels. Text is left-aligned at x≈536 and a single
/// line spans y≈776–822 on the reference recording. Long messages run past x=2000, so the
/// ROI extends to the right edge of the 2436px frame.
pub const MESSAGE_ROI: Rect = Rect { x: 500, y: 740, w: 1936, h: 120 };

const INK_MIN: u8 = 160; // every channel at least this bright
const INK_MAX_CHROMA: u8 = 60; // max - min channel
/// A margin pixel "contrasts" with the ink when some channel is well below ink level.
/// Using the min channel (not luma) keeps saturated-but-bright scenery like cyan as contrast.
const CONTRAST_MIN_CHANNEL: u8 = 110;
const MIN_CONTRAST_FRACTION: f32 = 0.35;
const MAX_GLYPH_H: u32 = 60;
/// Widest single blob that can be one character: just over one em. Measured: the widest
/// Latin glyph is `W` at 32px with capitals 33px tall, putting the em at ≈45–47px; full-width
/// CJK characters are about one em. Scenery that passes the colour and contrast gates is
/// often a long thin highlight — Pyroar's tail tuft at sunroom 03:16–03:28 is a 72x25 rim
/// sitting exactly on the left margin, which produced a burst of `???` messages.
const MAX_GLYPH_W: u32 = 50;
/// Glyphs in one row have vertical centres within this distance of the row's first glyph.
const ROW_TOLERANCE: i32 = 25;
/// Messages are left-aligned: the first glyph starts at x≈536 (ROI x≈36), measured 35–37.
/// Nothing else that lands in the ROI (HP panels, iOS banners, specular highlights) does.
const MESSAGE_LEFT: i32 = 36;
const MESSAGE_LEFT_TOLERANCE: i32 = 6;
/// Word spaces are ~10px. A gap far larger than that ends the message; whatever lies
/// beyond it is scenery that happens to share the row.
const MAX_INTRA_LINE_GAP: i32 = 60;

/// Binary mask of one glyph, cropped to its bounding box. Coordinates are within the ROI.
#[derive(Clone, Debug)]
pub struct Glyph {
    pub x0: i32,
    pub y0: i32,
    pub w: u32,
    pub h: u32,
    pub bits: Vec<bool>,
}

impl Glyph {
    pub fn x1(&self) -> i32 {
        self.x0 + self.w as i32
    }
    pub fn y1(&self) -> i32 {
        self.y0 + self.h as i32
    }
}

/// Horizontal white space between two glyphs, measured on the ink rather than bounding
/// boxes: the smallest per-scanline distance from `a`'s rightmost ink to `b`'s leftmost ink.
/// Bounding boxes understate the gap next to overhanging glyphs (`f`, `T`, `y`), which is
/// exactly where word spaces were being lost.
pub fn ink_gap(a: &Glyph, b: &Glyph) -> i32 {
    let mut best: Option<i32> = None;
    for y in a.y0.max(b.y0)..a.y1().min(b.y1()) {
        let row = |g: &Glyph| -> Option<(i32, i32)> {
            let r = (y - g.y0) as u32;
            let xs = (0..g.w).filter(|&x| g.bits[(r * g.w + x) as usize]);
            let (mut lo, mut hi) = (None, None);
            for x in xs {
                lo.get_or_insert(x as i32);
                hi = Some(x as i32);
            }
            Some((g.x0 + lo?, g.x0 + hi?))
        };
        if let (Some((_, ar)), Some((bl, _))) = (row(a), row(b)) {
            let d = bl - ar - 1;
            best = Some(best.map_or(d, |v: i32| v.min(d)));
        }
    }
    best.unwrap_or(b.x0 - a.x1())
}

/// One line of text: glyphs left to right, plus the baseline used to place them vertically.
#[derive(Clone, Debug)]
pub struct Row {
    pub glyphs: Vec<Glyph>,
    pub baseline: i32,
}

pub struct Image<'a> {
    pub w: u32,
    pub h: u32,
    pub rgb: &'a [u8],
}

impl Image<'_> {
    fn px(&self, x: u32, y: u32) -> (u8, u8, u8) {
        let i = ((y * self.w + x) * 3) as usize;
        (self.rgb[i], self.rgb[i + 1], self.rgb[i + 2])
    }
}

pub fn ink_mask(img: &Image) -> Vec<bool> {
    let mut m = vec![false; (img.w * img.h) as usize];
    for y in 0..img.h {
        for x in 0..img.w {
            let (r, g, b) = img.px(x, y);
            let lo = r.min(g).min(b);
            let hi = r.max(g).max(b);
            m[(y * img.w + x) as usize] = lo >= INK_MIN && hi - lo <= INK_MAX_CHROMA;
        }
    }
    m
}

struct Blob {
    x0: u32,
    y0: u32,
    x1: u32, // exclusive
    y1: u32,
    pixels: Vec<(u32, u32)>,
}

fn blobs(mask: &[bool], w: u32, h: u32) -> Vec<Blob> {
    let mut seen = vec![false; mask.len()];
    let mut out = Vec::new();
    let mut stack = Vec::new();
    for sy in 0..h {
        for sx in 0..w {
            let si = (sy * w + sx) as usize;
            if !mask[si] || seen[si] {
                continue;
            }
            seen[si] = true;
            stack.push((sx, sy));
            let mut b = Blob { x0: sx, y0: sy, x1: sx + 1, y1: sy + 1, pixels: Vec::new() };
            while let Some((x, y)) = stack.pop() {
                b.pixels.push((x, y));
                b.x0 = b.x0.min(x);
                b.y0 = b.y0.min(y);
                b.x1 = b.x1.max(x + 1);
                b.y1 = b.y1.max(y + 1);
                for dy in -1i32..=1 {
                    for dx in -1i32..=1 {
                        let (nx, ny) = (x as i32 + dx, y as i32 + dy);
                        if nx < 0 || ny < 0 || nx >= w as i32 || ny >= h as i32 {
                            continue;
                        }
                        let ni = (ny as u32 * w + nx as u32) as usize;
                        if mask[ni] && !seen[ni] {
                            seen[ni] = true;
                            stack.push((nx as u32, ny as u32));
                        }
                    }
                }
            }
            out.push(b);
        }
    }
    out
}

/// Fraction of non-ink pixels in a 3px margin around the blob that contrast with ink.
fn contrast_fraction(img: &Image, mask: &[bool], b: &Blob) -> f32 {
    let x0 = b.x0.saturating_sub(3);
    let y0 = b.y0.saturating_sub(3);
    let x1 = (b.x1 + 3).min(img.w);
    let y1 = (b.y1 + 3).min(img.h);
    let (mut contrast, mut total) = (0u32, 0u32);
    for y in y0..y1 {
        for x in x0..x1 {
            if mask[(y * img.w + x) as usize] {
                continue;
            }
            let (r, g, b) = img.px(x, y);
            total += 1;
            if r.min(g).min(b) < CONTRAST_MIN_CHANNEL {
                contrast += 1;
            }
        }
    }
    if total == 0 {
        0.0
    } else {
        contrast as f32 / total as f32
    }
}

/// Segment an ROI image into rows of glyphs, top to bottom.
pub fn segment(img: &Image) -> Vec<Row> {
    let mask = ink_mask(img);
    let mut kept: Vec<Blob> = blobs(&mask, img.w, img.h)
        .into_iter()
        .filter(|b| b.pixels.len() >= 3)
        .filter(|b| b.y1 - b.y0 <= MAX_GLYPH_H && b.x1 - b.x0 <= MAX_GLYPH_W)
        .filter(|b| contrast_fraction(img, &mask, b) >= MIN_CONTRAST_FRACTION)
        .collect();
    kept.sort_by_key(|b| (b.x0, b.y0));

    // Merge blobs stacked in the same column: the dot of i/j, the point of !, colons.
    let mut merged: Vec<Blob> = Vec::new();
    for b in kept {
        if let Some(prev) = merged.iter_mut().rev().find(|p| overlaps_column(p, &b)) {
            prev.x0 = prev.x0.min(b.x0);
            prev.y0 = prev.y0.min(b.y0);
            prev.x1 = prev.x1.max(b.x1);
            prev.y1 = prev.y1.max(b.y1);
            prev.pixels.extend(b.pixels);
        } else {
            merged.push(b);
        }
    }

    let glyphs: Vec<Glyph> = merged
        .into_iter()
        .filter(|b| b.y1 - b.y0 <= MAX_GLYPH_H)
        .map(|b| {
            let (w, h) = (b.x1 - b.x0, b.y1 - b.y0);
            let mut bits = vec![false; (w * h) as usize];
            for (x, y) in b.pixels {
                bits[((y - b.y0) * w + (x - b.x0)) as usize] = true;
            }
            Glyph { x0: b.x0 as i32, y0: b.y0 as i32, w, h, bits }
        })
        .collect();

    rows(glyphs)
}

fn overlaps_column(a: &Blob, b: &Blob) -> bool {
    let lo = a.x0.max(b.x0);
    let hi = a.x1.min(b.x1);
    if hi <= lo {
        return false;
    }
    let narrower = (a.x1 - a.x0).min(b.x1 - b.x0);
    // Vertically disjoint (or nearly) — a stacked mark, not two letters leaning together.
    let v_overlap = a.y1.min(b.y1) as i32 - a.y0.max(b.y0) as i32;
    (hi - lo) * 2 >= narrower && v_overlap <= 2
}

fn rows(glyphs: Vec<Glyph>) -> Vec<Row> {
    let mut rows: Vec<(i32, Vec<Glyph>)> = Vec::new();
    for g in glyphs {
        let c = (g.y0 + g.y1()) / 2;
        match rows.iter_mut().find(|(rc, _)| (rc - c).abs() <= ROW_TOLERANCE) {
            Some((_, v)) => v.push(g),
            None => rows.push((c, vec![g])),
        }
    }
    rows.sort_by_key(|(c, _)| *c);
    rows.into_iter()
        .map(|(_, mut glyphs)| {
            glyphs.sort_by_key(|g| g.x0);
            let mut bottoms: Vec<i32> = glyphs.iter().map(|g| g.y1()).collect();
            bottoms.sort_unstable();
            let baseline = bottoms[bottoms.len() / 2];
            Row { glyphs, baseline }
        })
        .collect()
}

/// Rows of message text: left-aligned at the message margin, truncated at the first gap
/// too wide to be a space.
pub fn text_rows(img: &Image) -> Vec<Row> {
    segment(img)
        .into_iter()
        .filter(|r| (r.glyphs[0].x0 - MESSAGE_LEFT).abs() <= MESSAGE_LEFT_TOLERANCE)
        .map(|mut r| {
            let cut = r
                .glyphs
                .windows(2)
                .position(|w| w[1].x0 - w[0].x1() > MAX_INTRA_LINE_GAP)
                .map_or(r.glyphs.len(), |i| i + 1);
            r.glyphs.truncate(cut);
            r
        })
        .filter(|r| r.glyphs.len() >= 3)
        .collect()
}
