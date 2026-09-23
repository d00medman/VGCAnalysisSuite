//! Message episodes: collapse per-frame readings into one event per on-screen message.
//!
//! Text types out character by character, so consecutive readings of one message are
//! prefix-extensions of each other; "extension" is judged up to a small edit distance, since
//! `!` can lose its point and read `l`, or a tight word gap lose its space.
//!
//! Single frames also *drop glyphs* — over a bright move effect, or while the line fades out.
//! A dropped glyph is simply absent, not `?`, so the reading looks clean. Such a reading is
//! recognised as a *fragment*: it can be made from the message by deleting glyphs, with at
//! most a couple of other edits (a space appearing where glyphs vanished). Fragments at least
//! `FRAGMENT_MIN_RATIO` of the message's length stay in the episode.
//!
//! A reading ends the episode when it is neither an extension nor a fragment. In particular a
//! reading much shorter than the message means a new message started typing — which is what
//! splits back-to-back messages that share a prefix (`...used Hyper Beam!` then `...used
//! Hyper Voice!`). A blank stretch longer than `BLANK_GAP` also ends it.
//!
//! Readings are compared against the episode's best reading so far, not the latest one, so a
//! corrupted frame never becomes the reference. The emitted text is the most frequent
//! near-full-length reading, which outvotes flicker.
//!
//! Short fade-out tails (`Porks61 sen`) still split off as their own episode. An episode whose
//! text is a fragment of the message just before it, with no blank between, is folded back
//! into that message. The cost: a message repeated back-to-back with *no* blank frame between
//! would be reported once.
//! Readings containing unknown glyphs (fades, partial occlusion) keep the episode alive but
//! never decide its text, unless nothing clean was ever read — then the episode is still
//! emitted, flagged, rather than silently dropped.

use crate::atlas::Reading;
use serde::Serialize;
use std::collections::BTreeMap;

/// Seconds of no text after which the current message is considered gone.
const BLANK_GAP: f64 = 0.25;
/// Readings shorter than this fraction of the message are a new message typing, never a
/// fragment of the current one.
const FRAGMENT_MIN_RATIO: f64 = 0.7;
/// Non-deletion edits a fragment may carry: a space appearing where glyphs vanished, or a
/// glyph misread while its neighbours dropped.
const FRAGMENT_EXTRA_EDITS: usize = 2;

#[derive(Serialize, Debug, Clone)]
pub struct Message {
    pub t0: f64,
    pub t1: f64,
    pub text: String,
    pub conf: f32,
    /// False when every reading had unknown glyphs; `text` then contains `?`.
    pub clean: bool,
}

#[derive(Default)]
struct Tally {
    frames: u32,
    conf: f32,
}

struct Episode {
    t0: f64,
    t1: f64,
    clean: BTreeMap<String, Tally>,
    noisy: BTreeMap<String, Tally>,
}

impl Episode {
    fn new(t: f64) -> Self {
        Self { t0: t, t1: t, clean: BTreeMap::new(), noisy: BTreeMap::new() }
    }

    fn record(&mut self, t: f64, r: &Reading) {
        self.t1 = t;
        let clean = r.unknown == 0;
        let map = if clean { &mut self.clean } else { &mut self.noisy };
        let e = map.entry(r.text.clone()).or_default();
        e.frames += 1;
        e.conf = e.conf.max(r.conf);
    }

    /// Among clean readings within 10% of the longest, the one held for the most frames.
    /// Ties go to the longer, then lexically first, so the choice is deterministic.
    fn best(&self) -> Option<(&String, &Tally)> {
        let max_len = self.clean.keys().map(|s| s.chars().count()).max().unwrap_or(0);
        self.clean
            .iter()
            .filter(|(s, _)| s.chars().count() * 10 >= max_len * 9)
            .max_by(|a, b| {
                (a.1.frames, a.0.chars().count())
                    .cmp(&(b.1.frames, b.0.chars().count()))
                    .then(b.0.cmp(a.0))
            })
    }

    fn accepts(&self, text: &str) -> bool {
        let Some((best, _)) = self.best() else { return true };
        let c: Vec<char> = best.chars().collect();
        let r: Vec<char> = text.chars().collect();
        is_extension(&r, &c) || is_fragment(&r, &c)
    }

    fn finish(self) -> Message {
        if let Some((text, t)) = self.best() {
            return Message { t0: self.t0, t1: self.t1, text: text.clone(), conf: t.conf, clean: true };
        }
        let (text, conf) = self
            .noisy
            .iter()
            .max_by(|a, b| a.1.frames.cmp(&b.1.frames).then(b.0.cmp(a.0)))
            .map(|(s, t)| (s.clone(), t.conf))
            .unwrap_or_default();
        Message { t0: self.t0, t1: self.t1, text, conf, clean: false }
    }
}

/// `r` continues typing `c` (or is `c`), up to a small edit distance on the shared prefix.
fn is_extension(r: &[char], c: &[char]) -> bool {
    r.len() >= c.len() && levenshtein(&r[..c.len()], c) <= (c.len() / 10).max(2)
}

/// `r` is `c` with glyphs dropped: no shorter than `FRAGMENT_MIN_RATIO` of it, and its edit
/// distance is explained by deletions plus at most `FRAGMENT_EXTRA_EDITS`.
fn is_fragment(r: &[char], c: &[char]) -> bool {
    r.len() <= c.len()
        && r.len() as f64 >= c.len() as f64 * FRAGMENT_MIN_RATIO
        && levenshtein(r, c) <= (c.len() - r.len()) + FRAGMENT_EXTRA_EDITS
}

fn levenshtein(a: &[char], b: &[char]) -> usize {
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    let mut cur = vec![0; b.len() + 1];
    for (i, ca) in a.iter().enumerate() {
        cur[0] = i + 1;
        for (j, cb) in b.iter().enumerate() {
            cur[j + 1] = (prev[j] + (ca != cb) as usize).min(prev[j + 1] + 1).min(cur[j] + 1);
        }
        std::mem::swap(&mut prev, &mut cur);
    }
    prev[b.len()]
}

#[derive(Default)]
pub struct Tracker {
    ep: Option<Episode>,
    /// Finished message held back one step, in case the next episode is a fade-out tail of it.
    held: Option<Message>,
}

impl Tracker {
    /// Feed one frame. `reading` is `None` when no text is on screen.
    pub fn push(&mut self, t: f64, reading: Option<&Reading>) -> Option<Message> {
        let Some(r) = reading else {
            if self.ep.as_ref().is_some_and(|e| t - e.t1 > BLANK_GAP) {
                return self.close();
            }
            return None;
        };
        let mut done = None;
        if let Some(ep) = &self.ep {
            let expired = t - ep.t1 > BLANK_GAP;
            let diverged = r.unknown == 0 && !ep.accepts(&r.text);
            if expired || diverged {
                done = self.close();
            }
        }
        self.ep.get_or_insert_with(|| Episode::new(t)).record(t, r);
        done
    }

    /// End the current episode. Returns the previously held message if this one does not
    /// fold into it.
    fn close(&mut self) -> Option<Message> {
        let m = self.ep.take()?.finish();
        if let Some(h) = &mut self.held {
            let c: Vec<char> = h.text.chars().collect();
            let r: Vec<char> = m.text.chars().collect();
            let contiguous = m.t0 - h.t1 <= BLANK_GAP;
            // Any clean remnant of the held message: a fragment, or a prefix of it.
            let remnant = m.clean
                && r.len() <= c.len()
                && levenshtein(&r, &c) <= (c.len() - r.len()) + FRAGMENT_EXTRA_EDITS;
            if contiguous && remnant {
                h.t1 = h.t1.max(m.t1);
                return None;
            }
        }
        self.held.replace(m)
    }

    /// End of stream: every message still held or in progress, in order.
    pub fn flush(&mut self) -> Vec<Message> {
        let mut out: Vec<Message> = self.close().into_iter().collect();
        out.extend(self.held.take());
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn r(text: &str) -> Reading {
        let unknown = text.matches('?').count();
        Reading { text: text.into(), conf: 0.9, unknown }
    }

    fn run(frames: &[(f64, Option<&str>)]) -> Vec<Message> {
        let mut tr = Tracker::default();
        let mut out = Vec::new();
        for (t, s) in frames {
            let rd = s.map(r);
            out.extend(tr.push(*t, rd.as_ref()));
        }
        out.extend(tr.flush());
        out
    }

    #[test]
    fn typewriter_prefixes_collapse_to_one_message() {
        let m = run(&[
            (0.0, Some("The")),
            (0.1, Some("The opp")),
            (0.2, Some("The opposing Sylveon")),
            (0.3, Some("The opposing Sylveon used Hyper Beam!")),
            (0.4, Some("The opposing Sylveon used Hyper Beam!")),
            (0.5, None),
            (1.0, None),
        ]);
        assert_eq!(m.len(), 1);
        assert_eq!(m[0].text, "The opposing Sylveon used Hyper Beam!");
        assert_eq!((m[0].t0, m[0].t1), (0.0, 0.4));
    }

    #[test]
    fn back_to_back_messages_split_without_a_blank() {
        let m = run(&[
            (0.0, Some("It's super effective!")),
            (0.1, Some("It's super effective!")),
            (0.2, Some("A critical hit!")),
            (0.3, Some("A critical hit!")),
        ]);
        let texts: Vec<_> = m.iter().map(|m| m.text.as_str()).collect();
        assert_eq!(texts, ["It's super effective!", "A critical hit!"]);
    }

    #[test]
    fn noisy_fade_frames_do_not_split_or_win() {
        let m = run(&[
            (0.0, Some("Pyroar fainted!")),
            (0.1, Some("Pyroar fainted!")),
            (0.2, Some("Py?oar fa?nted!")),
            (0.3, Some("P?r?ar")),
        ]);
        assert_eq!(m.len(), 1);
        assert_eq!(m[0].text, "Pyroar fainted!");
        assert!(m[0].clean);
    }

    #[test]
    fn single_frame_flicker_does_not_split() {
        let m = run(&[
            (0.0, Some("Pyroar used Heat Wave!")),
            (0.1, Some("Pyroar used H at Wave!")),
            (0.2, Some("Pyroar used Heat Wavel")),
            (0.3, Some("Pyroar used Heat Wave!")),
            (0.4, Some("Pyroar used Heat Wave!")),
        ]);
        assert_eq!(m.len(), 1);
        assert_eq!(m[0].text, "Pyroar used Heat Wave!");
    }

    #[test]
    fn shared_prefix_message_splits_when_typing_restarts() {
        let m = run(&[
            (0.0, Some("The opposing Sylveon used Hyper Beam!")),
            (0.1, Some("The opposing Sylveon used Hyper Beam!")),
            (0.2, Some("The opp")),
            (0.3, Some("The opposing Sylveon used Hyper Voice!")),
            (0.4, Some("The opposing Sylveon used Hyper Voice!")),
        ]);
        let texts: Vec<_> = m.iter().map(|m| m.text.as_str()).collect();
        assert_eq!(texts, ["The opposing Sylveon used Hyper Beam!", "The opposing Sylveon used Hyper Voice!"]);
    }

    /// Frames at ~30fps, as the decoder delivers them.
    fn frames<'a>(readings: &[&'a str]) -> Vec<(f64, Option<&'a str>)> {
        readings.iter().enumerate().map(|(i, s)| (i as f64 * 0.033, Some(*s))).collect()
    }

    #[test]
    fn fade_out_with_dropped_glyphs_is_one_message() {
        // sunroom 04:15–04:17: the fade-out drops single glyphs without producing `?`,
        // which previously emitted this message ten times.
        let full = "Porks619 sent out STAR PLTNM!";
        let m = run(&frames(&[
            "Porks6", "Porks619 sen", "Porks619 sent out ST", full, full, full, full, full,
            "Porks619 sent out TAR PLTNM!",
            "Porks619 sent ou ST R PLTNM!",
            full,
            "Porks619 s nt t STAR PLTNM!",
            "Porks61 s t out STAR PLTNM!",
            "Porks619 ent ut STAR PLTNM!",
            "Porks619 se t o t STAR PLTNM!",
            "Porks619 sen o STAR PLTNM!",
            "Porks619 sent ut ST R PLTN",
            "Porks61 sen",
            "Porks6",
        ]));
        let texts: Vec<_> = m.iter().map(|m| m.text.as_str()).collect();
        assert_eq!(texts, [full]);
    }

    #[test]
    fn flicker_over_a_bright_move_effect_is_one_message() {
        // sunroom 03:47–03:49: Overheat's flames knock out glyphs mid-message.
        let full = "Pyroar used Overheat!";
        let m = run(&frames(&[
            "Pyroar", "Pyroar used Ove", full, full, full,
            "Pyroar used Ove at!", full, full,
            "Pyroar used O rheat!", full,
            "P oar use Overheat", full, full,
        ]));
        let texts: Vec<_> = m.iter().map(|m| m.text.as_str()).collect();
        assert_eq!(texts, [full]);
    }

    #[test]
    fn identical_message_after_a_real_blank_is_emitted_twice() {
        // sunroom 05:48 and 05:57: the same line for two different turns.
        let m = run(&[
            (0.0, Some("Farigiraf protected itself!")),
            (0.1, Some("Farigiraf protected itself!")),
            (0.5, None),
            (9.0, Some("Farigiraf protected itself!")),
            (9.1, Some("Farigiraf protected itself!")),
        ]);
        assert_eq!(m.len(), 2);
    }

    #[test]
    fn unreadable_message_is_still_emitted() {
        let m = run(&[(0.0, Some("W?at")), (0.1, Some("W?at")), (0.2, Some("?"))]);
        assert_eq!(m.len(), 1);
        assert!(!m[0].clean);
        assert_eq!(m[0].text, "W?at");
    }
}
