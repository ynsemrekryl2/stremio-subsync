//! PGS ruler measurement (SS-025): cue times of the file's English image
//! subtitle tracks, saved so they can be checked against the same file's
//! text ruler offline. Only times are kept; no image is ever read.
//!
//! Cues give PGS points without duration. Each line is assumed to be a
//! "show" display set followed by a "clear" one, so consecutive points are
//! paired. Which point opens a pair is not known from Cues alone; both
//! phases are tried. `stremio-subsync pgs-check <dir>` lays each phase over
//! the text rulers saved with it, one line per pair of tracks.

use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use crate::align;
use crate::dump::Dump;
use crate::ruler::Event;

/// A pair longer than this is unlikely to be one subtitle line.
const LONG_MS: u64 = 10_000;

/// One English PGS track's cue times in milliseconds, in Cues order with
/// repeats kept.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PgsTrack {
    pub track: u64,
    pub forced: bool,
    pub times: Vec<u64>,
}

/// `Debug` prints only the track, the forced flag and the point count.
impl std::fmt::Debug for PgsTrack {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "({}{}, {})",
            self.track,
            if self.forced { " forced" } else { "" },
            self.times.len()
        )
    }
}

/// How cue points become lines.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Phase {
    /// Points 0–1, 2–3, …: the first point shows a line.
    Even,
    /// Points 1–2, 3–4, …: the first point clears something.
    Odd,
    /// Every point shows a line that lasts until the next point.
    Each,
}

impl Phase {
    pub const ALL: [Phase; 3] = [Phase::Even, Phase::Odd, Phase::Each];
}

/// Sorted distinct times; a display set indexed twice counts once.
fn distinct(times: &[u64]) -> Vec<u64> {
    let mut out = times.to_vec();
    out.sort_unstable();
    out.dedup();
    out
}

/// Lines built from `times` under `phase`.
pub fn lines(times: &[u64], phase: Phase) -> Vec<Event> {
    let times = distinct(times);
    let pair = |w: &[u64; 2]| Event {
        start_ms: w[0],
        end_ms: w[1],
    };
    match phase {
        Phase::Even => times.as_chunks::<2>().0.iter().map(pair).collect(),
        Phase::Odd => times
            .get(1..)
            .unwrap_or_default()
            .as_chunks::<2>()
            .0
            .iter()
            .map(pair)
            .collect(),
        Phase::Each => times.array_windows::<2>().map(pair).collect(),
    }
}

/// Median line length and the share of lines longer than `LONG_MS`.
fn shape(lines: &[Event]) -> (u64, usize) {
    let mut lengths: Vec<u64> = lines.iter().map(|e| e.end_ms - e.start_ms).collect();
    lengths.sort_unstable();
    let long = lengths.iter().filter(|&&l| l > LONG_MS).count();
    (
        lengths.get(lengths.len() / 2).copied().unwrap_or(0),
        100 * long / lengths.len().max(1),
    )
}

/// One line per PGS track and phase: its shape, then the verdict against
/// each text ruler of the same dump.
pub fn check_lines(name: &str, dump: &Dump) -> Vec<String> {
    let rulers = dump.rulers();
    let mut out = Vec::new();
    for pgs in &dump.pgs {
        let repeats = pgs.times.len() - distinct(&pgs.times).len();
        for phase in Phase::ALL {
            let lines = lines(&pgs.times, phase);
            let (median, long) = shape(&lines);
            let mut line = format!(
                "{name} {} pgs={}{} points={} repeats={repeats} phase={phase:?} lines={} \
                 median={median}ms long={long}%",
                dump.kind,
                pgs.track,
                if pgs.forced { " forced" } else { "" },
                pgs.times.len(),
                lines.len(),
            );
            if rulers.is_empty() {
                line.push_str(" text=none");
            }
            for (track, ruler) in &rulers {
                let verdict = align::align(ruler, &lines);
                let _ = write!(
                    line,
                    " | text={track} lines={} {}",
                    ruler.len(),
                    if verdict.accepted().is_some() {
                        "ACCEPT"
                    } else {
                        "reject"
                    }
                );
                if let Some(fix) = verdict.fix {
                    let _ = write!(
                        line,
                        " fix={}/{}{:+}ms hit={:.2}",
                        fix.num, fix.den, fix.shift_ms, verdict.hit_rate
                    );
                }
                if let Some(reject) = verdict.reject {
                    let _ = write!(line, " reason={reject:?}");
                }
            }
            out.push(line);
        }
    }
    out
}

/// Checks every PGS dump in `dir`. The same file opened twice leaves
/// identical dumps; those count once.
pub fn check(dir: &Path) -> std::io::Result<()> {
    let mut paths: Vec<PathBuf> = std::fs::read_dir(dir)?
        .filter_map(|entry| entry.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|x| x == "json"))
        .collect();
    paths.sort();
    let mut seen = HashSet::new();
    for path in paths {
        let name = path
            .file_name()
            .map_or(String::new(), |n| n.to_string_lossy().into_owned());
        let bytes = std::fs::read(&path)?;
        if !seen.insert(bytes.clone()) {
            continue;
        }
        let Ok(dump) = serde_json::from_slice::<Dump>(&bytes) else {
            println!("{name} unreadable");
            continue;
        };
        for line in check_lines(&name, &dump) {
            println!("{line}");
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::synth;

    fn ev(start_ms: u64, end_ms: u64) -> Event {
        Event { start_ms, end_ms }
    }

    #[test]
    fn phases_pair_distinct_points() {
        let times = [3_000, 1_000, 2_000, 2_000, 4_000, 5_000];
        assert_eq!(
            lines(&times, Phase::Even),
            vec![ev(1_000, 2_000), ev(3_000, 4_000)]
        );
        assert_eq!(
            lines(&times, Phase::Odd),
            vec![ev(2_000, 3_000), ev(4_000, 5_000)]
        );
        assert_eq!(lines(&times, Phase::Each).len(), 4);
        assert!(lines(&[], Phase::Odd).is_empty());
        assert!(lines(&[7], Phase::Odd).is_empty());
    }

    #[test]
    fn debug_prints_counts_only() {
        let track = PgsTrack {
            track: 4,
            forced: true,
            times: vec![123_456, 234_567],
        };
        assert_eq!(format!("{track:?}"), "(4 forced, 2)");
    }

    /// Show and clear points of synthetic lines that never overlap.
    fn show_and_clear(lines: &[Event]) -> Vec<u64> {
        let mut out = Vec::new();
        let mut last_end = 0;
        for line in lines {
            if line.start_ms <= last_end {
                continue;
            }
            out.extend([line.start_ms, line.end_ms]);
            last_end = line.end_ms;
        }
        out
    }

    #[test]
    fn show_and_clear_pairs_match_the_text_ruler() {
        let mut rng = synth::Rng::new(25);
        let speech = synth::speech(&mut rng, 45);
        let text = synth::lines(&mut rng, &speech, &synth::Style::RULER);
        let pgs = PgsTrack {
            track: 5,
            forced: false,
            times: show_and_clear(&text),
        };
        let mut dump = Dump::new("movie", &[(3, text)], &[], &[]);
        dump.pgs = vec![pgs];
        let out = check_lines("1.json", &dump);
        assert_eq!(out.len(), 3);
        let even = &out[0];
        assert!(
            even.contains("phase=Even") && even.contains("repeats=0"),
            "{even}"
        );
        assert!(even.contains("| text=3 lines="), "{even}");
        assert!(even.contains(" ACCEPT fix=1/1+0ms hit=1.00"), "{even}");
        // Odd pairs are the gaps between lines: a different, wrong shape.
        assert!(!out[1].contains("fix=1/1+0ms hit=1.00"), "{}", out[1]);
    }

    #[test]
    fn dump_without_text_ruler_still_reports_shape() {
        let mut dump = Dump::new("series", &[], &[], &[]);
        dump.pgs = vec![PgsTrack {
            track: 2,
            forced: false,
            times: vec![0, 2_000, 2_000, 20_000, 40_000],
        }];
        let out = check_lines("2.json", &dump);
        assert_eq!(
            out[0],
            "2.json series pgs=2 points=5 repeats=1 phase=Even lines=2 median=20000ms \
             long=50% text=none"
        );
    }

    #[test]
    fn check_reads_a_directory_of_dumps() {
        let dir = std::env::temp_dir().join(format!(
            "subsync-pgs-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let mut dump = Dump::new("movie", &[], &[], &[]);
        dump.pgs = vec![PgsTrack {
            track: 2,
            forced: false,
            times: vec![0, 1_000],
        }];
        crate::dump::write(&dir, &dump).unwrap();
        std::fs::write(dir.join("broken.json"), b"{").unwrap();
        check(&dir).unwrap();
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
