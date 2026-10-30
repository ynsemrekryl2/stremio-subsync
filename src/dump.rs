//! Probe timings saved for offline calibration (SS-014). A dump holds only
//! numbers: start and end times of the ruler tracks' lines and of each
//! downloaded candidate's lines, and the candidates' ranking scores. No ids,
//! names, links or text; the file is named after the time it was written.
//!
//! `stremio-subsync calibrate <dir>` lays every saved dump out again with
//! the current alignment code, one line per trial.

use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::align::{self, Trial};
use crate::pgs::PgsTrack;
use crate::ruler::Event;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Dump {
    pub kind: String,
    pub rulers: Vec<RulerTrack>,
    /// In download order, best ranked first; `None` where a download failed.
    pub candidates: Vec<Option<CandidateTimes>>,
    /// English PGS cue times (SS-025); only in PGS dumps.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub pgs: Vec<PgsTrack>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RulerTrack {
    pub track: u64,
    pub events: Vec<[u64; 2]>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CandidateTimes {
    pub score: Option<i32>,
    pub events: Vec<[u64; 2]>,
}

fn pairs(events: &[Event]) -> Vec<[u64; 2]> {
    events.iter().map(|e| [e.start_ms, e.end_ms]).collect()
}

fn events(pairs: &[[u64; 2]]) -> Vec<Event> {
    pairs
        .iter()
        .map(|&[start_ms, end_ms]| Event { start_ms, end_ms })
        .collect()
}

impl Dump {
    pub fn new(
        kind: &str,
        rulers: &[(u64, Vec<Event>)],
        candidates: &[Option<Vec<Event>>],
        scores: &[Option<i32>],
    ) -> Self {
        Self {
            kind: kind.to_owned(),
            rulers: rulers
                .iter()
                .map(|(track, events)| RulerTrack {
                    track: *track,
                    events: pairs(events),
                })
                .collect(),
            candidates: candidates
                .iter()
                .enumerate()
                .map(|(i, times)| {
                    times.as_ref().map(|times| CandidateTimes {
                        score: scores.get(i).copied().flatten(),
                        events: pairs(times),
                    })
                })
                .collect(),
            pgs: Vec::new(),
        }
    }

    pub fn rulers(&self) -> Vec<(u64, Vec<Event>)> {
        self.rulers
            .iter()
            .map(|r| (r.track, events(&r.events)))
            .collect()
    }

    pub fn candidates(&self) -> Vec<Option<Vec<Event>>> {
        self.candidates
            .iter()
            .map(|c| c.as_ref().map(|c| events(&c.events)))
            .collect()
    }
}

/// Writes `dump` as `<unix ms>.json` under `dir` and returns the file name.
pub fn write(dir: &Path, dump: &Dump) -> std::io::Result<String> {
    std::fs::create_dir_all(dir)?;
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_millis());
    let name = format!("{millis}.json");
    let body = serde_json::to_vec(dump).map_err(std::io::Error::other)?;
    std::fs::write(dir.join(&name), body)?;
    Ok(name)
}

/// How far local sync (SS-021) moves lines: share moved at all, and the
/// median and largest move.
fn local_summary(ruler: &[Event], candidate: &[Event], fix: align::Fix) -> String {
    let mut moves: Vec<i64> = align::local_shifts(ruler, candidate, fix)
        .into_iter()
        .map(i64::abs)
        .collect();
    moves.sort_unstable();
    let moved = moves.iter().filter(|&&m| m > 0).count();
    let check = align::local_check(ruler, candidate, fix);
    let hits: Vec<String> = check
        .hit_rates
        .iter()
        .map(|h| format!("h{:.0}%", h * 100.0))
        .collect();
    format!(
        "[moved {}% median {}ms max {}ms span {}ms {}]",
        100 * moved / moves.len().max(1),
        moves.get(moves.len() / 2).copied().unwrap_or(0),
        moves.last().copied().unwrap_or(0),
        check.span_ms,
        hits.join(" ")
    )
}

/// One trial as a single readable line.
pub fn trial_line(name: &str, dump: &Dump, trial: &Trial) -> String {
    let ruler_lines = dump
        .rulers
        .iter()
        .find(|r| r.track == trial.track)
        .map_or(0, |r| r.events.len());
    let candidate = dump
        .candidates
        .get(trial.candidate)
        .and_then(Option::as_ref);
    let verdict = &trial.verdict;
    let mut line = format!(
        "{name} {} track={} cand={} score={} lines={}/{} {}",
        dump.kind,
        trial.track,
        trial.candidate,
        candidate
            .and_then(|c| c.score)
            .map_or("-".to_owned(), |s| s.to_string()),
        ruler_lines,
        candidate.map_or(0, |c| c.events.len()),
        match (trial.accepted, verdict.accepted()) {
            (true, Some(_)) => "ACCEPT",
            (true, None) => "ACCEPT-local",
            (false, _) => "reject",
        },
    );
    if let Some(fix) = verdict.fix {
        let _ = write!(
            line,
            " fix={}/{}{:+}ms hit={:.2} windows=[",
            fix.num, fix.den, fix.shift_ms, verdict.hit_rate
        );
        for (i, w) in verdict.windows.iter().enumerate() {
            let shift = w
                .best_shift_ms
                .map_or("none".to_owned(), |s| format!("{:+}ms", s - fix.shift_ms));
            let _ = write!(
                line,
                "{}{shift} h{:.0}% c{:.0}%",
                if i > 0 { ", " } else { "" },
                w.hit_rate * 100.0,
                w.coverage * 100.0
            );
        }
        line.push(']');
        if let (Some(ruler), Some(candidate)) = (
            dump.rulers.iter().find(|r| r.track == trial.track),
            candidate,
        ) {
            let _ = write!(
                line,
                " local={}",
                local_summary(&events(&ruler.events), &events(&candidate.events), fix)
            );
        }
    }
    if let Some(reject) = verdict.reject {
        let _ = write!(line, " reason={reject:?}");
    }
    line
}

/// Aligns every dump in `dir` again and prints one line per trial. The same
/// file opened twice in Stremio leaves identical dumps; those count once.
pub fn calibrate(dir: &Path) -> std::io::Result<()> {
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
        for trial in align::trials(&dump.rulers(), &dump.candidates()) {
            println!("{}", trial_line(&name, &dump, &trial));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::synth;

    type Sample = (Vec<(u64, Vec<Event>)>, Vec<Option<Vec<Event>>>);

    fn sample() -> Sample {
        let mut rng = synth::Rng::new(11);
        let speech = synth::speech(&mut rng, 90);
        let ruler = synth::lines(&mut rng, &speech, &synth::Style::RULER);
        let good = synth::lines(&mut rng, &speech, &synth::Style::TRANSLATED);
        (vec![(5, ruler)], vec![Some(good), None])
    }

    #[test]
    fn dump_round_trips_through_json() {
        let (rulers, candidates) = sample();
        let dump = Dump::new("movie", &rulers, &candidates, &[Some(40), None]);
        let json = serde_json::to_string(&dump).unwrap();
        let back: Dump = serde_json::from_str(&json).unwrap();
        assert_eq!(back, dump);
        assert_eq!(back.rulers(), rulers);
        assert_eq!(back.candidates(), candidates);
        assert_eq!(back.candidates[0].as_ref().unwrap().score, Some(40));
        // Only numbers and the kind: no text fields to leak.
        assert!(!json.contains("\"name\"") && !json.contains("\"url\""));
        // Alignment dumps carry no PGS field, and older dumps still read.
        assert!(!json.contains("pgs"), "{json}");
        let old = r#"{"kind":"movie","rulers":[],"candidates":[]}"#;
        assert!(serde_json::from_str::<Dump>(old).unwrap().pgs.is_empty());
    }

    #[test]
    fn trial_line_reads_at_a_glance() {
        let (rulers, candidates) = sample();
        let dump = Dump::new("series", &rulers, &candidates, &[Some(10), None]);
        let trials = align::trials(&dump.rulers(), &dump.candidates());
        assert_eq!(trials.len(), 1);
        let line = trial_line("1.json", &dump, &trials[0]);
        assert!(
            line.starts_with("1.json series track=5 cand=0 score=10 lines="),
            "{line}"
        );
        assert!(line.contains(" ACCEPT fix=1/1"), "{line}");
        assert!(
            line.contains("windows=[") && !line.contains("reason="),
            "{line}"
        );
    }

    #[test]
    fn write_names_file_by_time_and_calibrate_reads_it() {
        let dir = std::env::temp_dir().join(format!(
            "subsync-dump-test-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let (rulers, candidates) = sample();
        let dump = Dump::new("movie", &rulers, &candidates, &[None, None]);
        let name = write(&dir, &dump).unwrap();
        assert!(name.ends_with(".json") && name[..name.len() - 5].parse::<u128>().is_ok());
        let saved: Dump = serde_json::from_slice(&std::fs::read(dir.join(&name)).unwrap()).unwrap();
        assert_eq!(saved, dump);
        calibrate(&dir).unwrap();
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
