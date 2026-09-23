//! Minimal RGB PNG read/write for harvested crops.

use anyhow::{bail, Context, Result};
use std::fs::File;
use std::io::BufWriter;
use std::path::Path;

pub fn write_rgb(path: &Path, w: u32, h: u32, rgb: &[u8]) -> Result<()> {
    let file = File::create(path).with_context(|| format!("creating {}", path.display()))?;
    let mut enc = png::Encoder::new(BufWriter::new(file), w, h);
    enc.set_color(png::ColorType::Rgb);
    enc.set_depth(png::BitDepth::Eight);
    enc.write_header()?.write_image_data(rgb)?;
    Ok(())
}

/// Returns (width, height, packed RGB24).
pub fn read_rgb(path: &Path) -> Result<(u32, u32, Vec<u8>)> {
    let file = File::open(path).with_context(|| format!("opening {}", path.display()))?;
    let mut dec = png::Decoder::new(file);
    dec.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);
    let mut reader = dec.read_info()?;
    let mut buf = vec![0; reader.output_buffer_size()];
    let info = reader.next_frame(&mut buf)?;
    let px = (info.width * info.height) as usize;
    let rgb = match info.color_type {
        png::ColorType::Rgb => buf[..px * 3].to_vec(),
        png::ColorType::Rgba => buf[..px * 4].chunks(4).flat_map(|p| [p[0], p[1], p[2]]).collect(),
        png::ColorType::Grayscale => buf[..px].iter().flat_map(|&v| [v, v, v]).collect(),
        other => bail!("{}: unsupported PNG colour type {other:?}", path.display()),
    };
    Ok((info.width, info.height, rgb))
}
