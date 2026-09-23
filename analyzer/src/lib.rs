pub mod atlas;
pub mod decode;
pub mod episode;
pub mod pngio;
pub mod progress;
pub mod text;
pub mod transcribe;

/// The committed glyph atlas, for runs from a source checkout. Deployed binaries point
/// `ANALYZER_ATLAS` at their own copy instead.
pub const DEFAULT_ATLAS: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/atlas/glyphs.txt");
