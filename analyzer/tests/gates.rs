//! The text gates in `text::text_rows`, checked against real frames: every labelled message
//! crop must be found as one row of text, and scenery that once leaked through as messages
//! must be found as none.

use analyzer::pngio;
use analyzer::text::{text_rows, Image};
use std::path::{Path, PathBuf};

fn rows_in(path: &Path) -> usize {
    let (w, h, rgb) = pngio::read_rgb(path).unwrap();
    text_rows(&Image { w, h, rgb: &rgb }).len()
}

fn pngs(dir: &Path) -> Vec<PathBuf> {
    let mut v: Vec<PathBuf> = std::fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.extension().is_some_and(|e| e == "png"))
        .collect();
    v.sort();
    v
}

#[test]
fn every_labelled_message_is_one_text_row() {
    let crops = pngs(&Path::new(env!("CARGO_MANIFEST_DIR")).join("atlas/crops"));
    assert!(!crops.is_empty());
    let wrong: Vec<_> = crops
        .iter()
        .map(|p| (p.file_name().unwrap().to_string_lossy().into_owned(), rows_in(p)))
        .filter(|(_, n)| *n != 1)
        .collect();
    assert!(wrong.is_empty(), "crops not read as exactly one row: {wrong:?}");
}

#[test]
fn scenery_at_the_margin_is_not_text() {
    // Pyroar's tail tuft sits on the message margin during sunroom's 03:16–03:28 move
    // selection; its specular rim produced a burst of `???` messages.
    let fixtures = pngs(&Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/scenery"));
    assert!(!fixtures.is_empty());
    let leaked: Vec<_> = fixtures
        .iter()
        .map(|p| (p.file_name().unwrap().to_string_lossy().into_owned(), rows_in(p)))
        .filter(|(_, n)| *n != 0)
        .collect();
    assert!(leaked.is_empty(), "scenery read as text: {leaked:?}");
}
