//! Labeling mode (SS-014): every downloaded candidate shows up in Stremio's
//! subtitle menu, numbered in rank order, marked ✓ (accepted) or ✗
//! (rejected) and moved by its best fix, so the user can watch and say
//! which ones really are in sync. Subtitles are kept in memory only.

use std::collections::{HashMap, VecDeque};

use crate::align::{Fix, Trial};
use crate::srt::{self, Cue};

/// Requests whose subtitles are kept.
const CAPACITY: usize = 32;

const SAME: Fix = Fix {
    num: 1,
    den: 1,
    shift_ms: 0,
};

/// One menu entry. `Debug` shows the label only; it carries no title.
pub struct LabelSub {
    pub label: String,
    pub srt: String,
}

impl std::fmt::Debug for LabelSub {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LabelSub")
            .field("label", &self.label)
            .field("srt_len", &self.srt.len())
            .finish()
    }
}

/// Menu text, e.g. `SubSync 2 ✓ +1.60s fps · 7530`: rank (1 = best named),
/// verdict, shift, a frame-rate fix if any, and the last digits of the dump
/// file so a label can be matched to `calibrate` output.
pub fn label(trial: &Trial, dump: Option<&str>) -> String {
    let mark = if trial.accepted { "✓" } else { "✗" };
    let mut text = format!("SubSync {} {mark}", trial.candidate + 1);
    if let Some(fix) = trial.verdict.fix {
        text.push_str(&format!(" {:+.2}s", fix.shift_ms as f64 / 1_000.0));
        if fix.num != fix.den {
            text.push_str(" fps");
        }
    }
    if let Some(name) = dump {
        let stem = name.trim_end_matches(".json");
        let tail = &stem[stem.len().saturating_sub(4)..];
        text.push_str(&format!(" · {tail}"));
    }
    text
}

/// Menu entries for the first ruler track's trials, each candidate moved by
/// its fix; a rejected one too, so a wrong reject can be seen in sync.
pub fn build(trials: &[Trial], cues: &[Option<Vec<Cue>>], dump: Option<&str>) -> Vec<LabelSub> {
    let Some(track) = trials.first().map(|t| t.track) else {
        return Vec::new();
    };
    trials
        .iter()
        .filter(|t| t.track == track)
        .filter_map(|trial| {
            let cues = cues.get(trial.candidate)?.as_ref()?;
            Some(LabelSub {
                label: label(trial, dump),
                srt: srt::render(cues, trial.verdict.fix.unwrap_or(SAME)),
            })
        })
        .collect()
}

/// Labeled subtitles of recent requests. The request key (kind, id, file
/// name) lives only here and is never printed; URLs use the entry number.
#[derive(Default)]
pub struct LabelCache {
    by_request: HashMap<String, u64>,
    entries: HashMap<u64, Vec<LabelSub>>,
    order: VecDeque<u64>,
    next: u64,
}

impl LabelCache {
    pub fn find(&self, request: &str) -> Option<(u64, &[LabelSub])> {
        let entry = *self.by_request.get(request)?;
        Some((entry, self.entries.get(&entry)?.as_slice()))
    }

    /// Stores `subs` for `request`, dropping the oldest entry when full.
    pub fn insert(&mut self, request: String, subs: Vec<LabelSub>) -> u64 {
        let entry = self.next;
        self.next += 1;
        if let Some(old) = self.by_request.insert(request, entry) {
            self.entries.remove(&old);
            self.order.retain(|&e| e != old);
        }
        self.entries.insert(entry, subs);
        self.order.push_back(entry);
        while self.order.len() > CAPACITY {
            if let Some(oldest) = self.order.pop_front() {
                self.entries.remove(&oldest);
                self.by_request.retain(|_, e| *e != oldest);
            }
        }
        entry
    }

    /// SRT text of menu entry `n` (from 1) of `entry`.
    pub fn srt(&self, entry: u64, n: usize) -> Option<&str> {
        let subs = self.entries.get(&entry)?;
        subs.get(n.checked_sub(1)?).map(|s| s.srt.as_str())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::align::{Reject, Verdict};

    fn trial(track: u64, candidate: usize, accepted: bool, fix: Option<Fix>) -> Trial {
        Trial {
            track,
            candidate,
            accepted,
            elapsed_ms: 1,
            verdict: Verdict {
                fix,
                hit_rate: 0.9,
                windows: Vec::new(),
                reject: (!accepted).then_some(Reject::LowHits(0)),
            },
        }
    }

    fn cue(start_ms: u64, end_ms: u64, text: &str) -> Cue {
        Cue {
            start_ms,
            end_ms,
            text: text.to_owned(),
        }
    }

    #[test]
    fn label_reads_rank_mark_shift_fps_and_dump() {
        let pal = Fix {
            num: 1_001,
            den: 1_000,
            shift_ms: 1_597,
        };
        assert_eq!(
            label(&trial(4, 2, true, Some(pal)), Some("1793127057530.json")),
            "SubSync 3 ✓ +1.60s fps · 7530"
        );
        let back = Fix {
            num: 1,
            den: 1,
            shift_ms: -34,
        };
        assert_eq!(
            label(&trial(4, 0, false, Some(back)), None),
            "SubSync 1 ✗ -0.03s"
        );
        assert_eq!(label(&trial(4, 1, false, None), None), "SubSync 2 ✗");
    }

    #[test]
    fn build_uses_first_track_and_moves_every_candidate() {
        let shift = Fix {
            num: 1,
            den: 1,
            shift_ms: 1_000,
        };
        let trials = [
            trial(5, 0, true, Some(shift)),
            trial(5, 1, false, None),
            trial(5, 2, false, Some(shift)),
            trial(3, 0, true, Some(SAME)),
        ];
        let cues = [
            Some(vec![cue(0, 500, "a")]),
            None,
            Some(vec![cue(2_000, 2_500, "c")]),
        ];
        let subs = build(&trials, &cues, None);
        let labels: Vec<&str> = subs.iter().map(|s| s.label.as_str()).collect();
        assert_eq!(labels, vec!["SubSync 1 ✓ +1.00s", "SubSync 3 ✗ +1.00s"]);
        assert_eq!(subs[0].srt, "1\n00:00:01,000 --> 00:00:01,500\na\n\n");
        assert_eq!(subs[1].srt, "1\n00:00:03,000 --> 00:00:03,500\nc\n\n");
        assert!(!format!("{:?}", subs[0]).contains("00:00"));
        assert!(build(&[], &cues, None).is_empty());
    }

    fn one(text: &str) -> Vec<LabelSub> {
        vec![LabelSub {
            label: text.to_owned(),
            srt: text.to_owned(),
        }]
    }

    #[test]
    fn cache_finds_replaces_and_evicts() {
        let mut cache = LabelCache::default();
        let first = cache.insert("a".to_owned(), one("x"));
        assert_eq!(cache.find("a").map(|(e, s)| (e, s.len())), Some((first, 1)));
        assert_eq!(cache.srt(first, 1), Some("x"));
        assert_eq!(cache.srt(first, 0), None);
        assert_eq!(cache.srt(first, 2), None);

        let again = cache.insert("a".to_owned(), one("y"));
        assert_ne!(again, first);
        assert_eq!(cache.srt(first, 1), None, "replaced entry is gone");
        assert_eq!(cache.srt(again, 1), Some("y"));

        for i in 0..CAPACITY {
            cache.insert(format!("r{i}"), one("z"));
        }
        assert!(cache.find("a").is_none(), "oldest request evicted");
        assert_eq!(cache.entries.len(), CAPACITY);
        assert_eq!(cache.by_request.len(), CAPACITY);
    }
}
