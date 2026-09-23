use analyzer::atlas::{side_bearing, Atlas};
use analyzer::decode::{ffmpeg_path, Decoder};
use analyzer::episode::Tracker;
use analyzer::text::{self, Image, Row, MESSAGE_ROI};
use analyzer::pngio;
use analyzer::progress::{clock, Progress};
use anyhow::{bail, Context, Result};
use clap::{Parser, Subcommand};
use std::io::Write;
use std::path::{Path, PathBuf};

#[derive(Parser)]
#[command(name = "analyzer", about = "Battle transcripts from Pokémon Champions screen recordings")]
struct Cli {
    /// ffmpeg binary. Defaults to the pinned vendor/ffmpeg, then PATH.
    #[arg(long, env = "ANALYZER_FFMPEG", global = true)]
    ffmpeg: Option<PathBuf>,

    /// Glyph atlas.
    #[arg(long, env = "ANALYZER_ATLAS", global = true,
          default_value = concat!(env!("CARGO_MANIFEST_DIR"), "/atlas/glyphs.txt"))]
    atlas: PathBuf,

    /// Suppress progress output on stderr.
    #[arg(long, short, global = true)]
    quiet: bool,

    #[command(subcommand)]
    command: Command,
}

#[derive(clap::Args)]
struct Range {
    /// Start time in seconds.
    #[arg(long)]
    ss: Option<f64>,
    /// Duration in seconds.
    #[arg(long)]
    t: Option<f64>,
}

#[derive(Subcommand)]
enum Command {
    /// Print the battle's message-line text, one message per line.
    Transcript {
        video: PathBuf,
        #[command(flatten)]
        range: Range,
        /// Emit JSONL (t0, t1, text, conf, clean) instead of human-readable text.
        #[arg(long)]
        jsonl: bool,
    },
    /// Save one message-ROI crop per distinct stable line of text, for labelling.
    Harvest {
        video: PathBuf,
        #[command(flatten)]
        range: Range,
        /// Output directory for PNG crops.
        #[arg(long)]
        out: PathBuf,
        /// Only keep crops the current atlas cannot read cleanly.
        #[arg(long)]
        unknown_only: bool,
    },
    /// Build the atlas from labelled crops. `labels` is TSV: `<png path>\t<exact text>`,
    /// with png paths relative to the TSV's directory.
    Atlas { labels: PathBuf },
    /// Segment and read a single ROI crop (debugging).
    Read { png: PathBuf },
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    let ffmpeg = ffmpeg_path(cli.ffmpeg.as_deref());
    match cli.command {
        Command::Transcript { video, range, jsonl } => {
            let atlas = Atlas::load(&cli.atlas)?;
            transcript(&ffmpeg, &atlas, &video, &range, jsonl, !cli.quiet)
        }
        Command::Harvest { video, range, out, unknown_only } => {
            let atlas = if unknown_only { Some(Atlas::load(&cli.atlas)?) } else { None };
            harvest(&ffmpeg, atlas.as_ref(), &video, &range, &out, !cli.quiet)
        }
        Command::Atlas { labels } => build_atlas(&labels, &cli.atlas),
        Command::Read { png } => read_one(&cli.atlas, &png),
    }
}

fn open(ffmpeg: &Path, video: &Path, range: &Range) -> Result<Decoder> {
    Decoder::open(ffmpeg, video, MESSAGE_ROI, range.ss, range.t)
}

fn roi_image(rgb: &[u8]) -> Image<'_> {
    Image { w: MESSAGE_ROI.w, h: MESSAGE_ROI.h, rgb }
}

/// Start the decoder and a progress reporter covering the requested range.
fn start(ffmpeg: &Path, video: &Path, range: &Range, verbose: bool) -> Result<(Decoder, Progress)> {
    let t_start = range.ss.unwrap_or(0.0);
    let mut progress = Progress::new(verbose, t_start, range.t);
    progress.log(&format!("video  {}", video.display()));
    progress.log(&format!("ffmpeg {}", ffmpeg.display()));
    let dec = open(ffmpeg, video, range)?;
    progress.log("decoding (first status line in ~2s)");
    // The file duration arrives on ffmpeg's stderr as the input opens; it is used on the
    // first progress tick if `--t` did not already fix the span.
    progress.set_span_if_unknown(dec.duration().map(|d| d - t_start));
    Ok((dec, progress))
}

fn transcript(
    ffmpeg: &Path,
    atlas: &Atlas,
    video: &Path,
    range: &Range,
    jsonl: bool,
    verbose: bool,
) -> Result<()> {
    let (mut dec, mut progress) = start(ffmpeg, video, range, verbose)?;
    let mut tracker = Tracker::default();
    // Unbuffered in effect: each message is flushed as soon as it is final, so the transcript
    // grows live instead of appearing all at once when the run ends.
    let mut out = std::io::stdout().lock();
    let mut count = 0;
    let emit = |m: analyzer::episode::Message, out: &mut dyn Write, count: &mut usize| -> Result<()> {
        if jsonl {
            writeln!(out, "{}", serde_json::to_string(&m)?)?;
        } else {
            let flag = if m.clean { "" } else { "  [unclear]" };
            writeln!(out, "[{}] {}{flag}", clock(m.t0), m.text)?;
        }
        out.flush()?;
        *count += 1;
        Ok(())
    };
    while let Some(f) = dec.next_frame()? {
        progress.set_span_if_unknown(dec.duration().map(|d| d - range.ss.unwrap_or(0.0)));
        let rows = text::text_rows(&roi_image(&f.rgb));
        let reading = (!rows.is_empty()).then(|| atlas.read_rows(&rows));
        if let Some(m) = tracker.push(f.t, reading.as_ref()) {
            emit(m, &mut out, &mut count)?;
        }
        progress.frame(f.t, count);
    }
    for m in tracker.flush() {
        emit(m, &mut out, &mut count)?;
    }
    progress.finish(count);
    Ok(())
}

/// Coarse layout signature of a text line: glyph x-positions and widths.
fn signature(rows: &[Row]) -> Vec<(i32, u32)> {
    rows.iter().flat_map(|r| r.glyphs.iter().map(|g| (g.x0, g.w))).collect()
}

fn same_layout(a: &[(i32, u32)], b: &[(i32, u32)]) -> bool {
    a.len() == b.len()
        && a.iter().zip(b).all(|(p, q)| (p.0 - q.0).abs() <= 1 && (p.1 as i32 - q.1 as i32).abs() <= 1)
}

fn harvest(
    ffmpeg: &Path,
    atlas: Option<&Atlas>,
    video: &Path,
    range: &Range,
    out: &Path,
    verbose: bool,
) -> Result<()> {
    std::fs::create_dir_all(out)?;
    let stem = video.file_stem().and_then(|s| s.to_str()).unwrap_or("video");
    let (mut dec, mut progress) = start(ffmpeg, video, range, verbose)?;
    let mut prev: Vec<(i32, u32)> = Vec::new();
    let mut stable = 0;
    let mut saved: Vec<(i32, u32)> = Vec::new();
    let mut count = 0;
    while let Some(f) = dec.next_frame()? {
        progress.set_span_if_unknown(dec.duration().map(|d| d - range.ss.unwrap_or(0.0)));
        progress.frame(f.t, count);
        let rows = text::text_rows(&roi_image(&f.rgb));
        let sig = signature(&rows);
        if !sig.is_empty() && same_layout(&sig, &prev) {
            stable += 1;
        } else {
            stable = 0;
        }
        prev = sig;
        // Four identical frames: fully typed out (or a long pause mid-typing — harmless).
        if stable == 3 && !same_layout(&prev, &saved) {
            saved = prev.clone();
            if let Some(a) = atlas {
                if a.read_rows(&rows).unknown == 0 {
                    continue;
                }
            }
            let path = out.join(format!("{stem}_{:08.3}.png", f.t));
            pngio::write_rgb(&path, MESSAGE_ROI.w, MESSAGE_ROI.h, &f.rgb)?;
            println!("{}", path.display());
            count += 1;
        }
    }
    progress.finish(count);
    eprintln!("harvested {count} crops");
    Ok(())
}

fn build_atlas(labels: &Path, atlas_path: &Path) -> Result<()> {
    let base = labels.parent().unwrap_or(Path::new("."));
    let tsv = std::fs::read_to_string(labels).with_context(|| format!("reading {}", labels.display()))?;
    let mut atlas = Atlas::default();
    let (mut space_gaps, mut glyph_gaps) = (Vec::new(), Vec::new());
    let (mut used, mut skipped) = (0, 0);
    for (n, line) in tsv.lines().enumerate() {
        if line.trim().is_empty() || line.starts_with('#') {
            continue;
        }
        let Some((file, label)) = line.split_once('\t') else {
            bail!("{}:{}: expected <png>\\t<text>", labels.display(), n + 1);
        };
        let (w, h, rgb) = pngio::read_rgb(&base.join(file))?;
        let rows = text::text_rows(&Image { w, h, rgb: &rgb });
        let glyphs: Vec<_> = rows.iter().flat_map(|r| r.glyphs.iter().map(move |g| (g, r.baseline))).collect();
        // Rows are joined by a space in readings; labels follow the same convention.
        let chars: Vec<char> = label.chars().filter(|c| *c != ' ').collect();
        if glyphs.len() != chars.len() {
            eprintln!("skip {file}: {} glyphs vs {} chars in {label:?}", glyphs.len(), chars.len());
            skipped += 1;
            continue;
        }
        used += 1;
        for (i, ((g, base), c)) in glyphs.iter().zip(&chars).enumerate() {
            atlas.add(&c.to_string(), g, *base);
            if i + 1 < glyphs.len() && glyphs[i + 1].1 == *base {
                let gap = text::ink_gap(g, glyphs[i + 1].0)
                    - side_bearing(&c.to_string())
                    - side_bearing(&chars[i + 1].to_string());
                // Is there a space in the label between char i and i+1?
                let pos = nth_nonspace_index(label, i);
                let spaced = label[pos..].chars().nth(1) == Some(' ');
                if spaced { space_gaps.push(gap) } else { glyph_gaps.push(gap) }
            }
        }
    }
    let max_glyph = glyph_gaps.iter().copied().max().unwrap_or(0);
    let min_space = space_gaps.iter().copied().min().unwrap_or(max_glyph + 2);
    if min_space <= max_glyph {
        eprintln!("warning: gaps overlap — largest intra-word gap {max_glyph}, smallest space {min_space}");
    }
    atlas.space_gap = (max_glyph + min_space + 1) / 2;
    space_gaps.sort_unstable();
    glyph_gaps.sort_unstable();
    eprintln!("intra-word gaps (top 10): {:?}", &glyph_gaps[glyph_gaps.len().saturating_sub(10)..]);
    eprintln!("word gaps (bottom 10): {:?}", &space_gaps[..space_gaps.len().min(10)]);
    atlas.save(atlas_path)?;
    let mut labels: Vec<_> = atlas.templates.iter().map(|t| t.label.as_str()).collect();
    labels.sort_unstable();
    labels.dedup();
    eprintln!(
        "atlas: {} templates, {} labels from {used} crops ({skipped} skipped); gaps: word ≤{max_glyph}, space ≥{min_space} → {}",
        atlas.templates.len(), labels.len(), atlas.space_gap
    );
    eprintln!("labels: {}", labels.concat());
    Ok(())
}

fn nth_nonspace_index(s: &str, n: usize) -> usize {
    s.char_indices().filter(|(_, c)| *c != ' ').nth(n).map(|(i, _)| i).unwrap_or(s.len())
}

fn read_one(atlas_path: &Path, png: &Path) -> Result<()> {
    let (w, h, rgb) = pngio::read_rgb(png)?;
    let rows = text::text_rows(&Image { w, h, rgb: &rgb });
    let atlas = if atlas_path.exists() { Atlas::load(atlas_path)? } else { Atlas::default() };
    for row in &rows {
        println!("row baseline {} ({} glyphs)", row.baseline, row.glyphs.len());
        for (i, g) in row.glyphs.iter().enumerate() {
            let gap = if i > 0 { g.x0 - row.glyphs[i - 1].x1() } else { 0 };
            let c = atlas.classify(g, row.baseline);
            println!("  x{:4} top{:4} {:2}x{:2} gap{:3} → {:?}", g.x0, g.y0 - row.baseline, g.w, g.h, gap, c);
        }
    }
    let r = atlas.read_rows(&rows);
    println!("{:?} conf {:.2} unknown {}", r.text, r.conf, r.unknown);
    Ok(())
}
