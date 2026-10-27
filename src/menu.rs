//! Delivery (M5): the subtitles offered to Stremio for one request, kept in
//! memory and on disk so a file opened again needs no probes.
//!
//! The request key (kind, IMDb id, file name, size) lives only in memory;
//! on disk a file is named after a hash of it. Subtitle text is stored, never
//! printed: `Debug` shows the menu text and length only.

use serde::{Deserialize, Serialize};
use std::collections::{HashMap, VecDeque};
use std::path::{Path, PathBuf};

use crate::align::Trial;
use crate::srt::{self, Cue};

/// Requests kept in memory.
const CAPACITY: usize = 32;
/// At most this many subtitles are offered for one file.
const MAX_OFFERED: usize = 3;
/// How long an answer stays good: one with subtitles, and "none fit".
const KEEP_FOUND_S: u64 = 30 * 24 * 3600;
const KEEP_NONE_S: u64 = 24 * 3600;

/// One menu entry: `lang` is what Stremio shows (a language code, or a
/// label in labeling mode).
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MenuEntry {
    pub lang: String,
    pub srt: String,
}

impl std::fmt::Debug for MenuEntry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MenuEntry")
            .field("lang", &self.lang)
            .field("srt_len", &self.srt.len())
            .finish()
    }
}

fn fresh(created: u64, entries: &[MenuEntry], now: u64) -> bool {
    let keep = if entries.is_empty() {
        KEEP_NONE_S
    } else {
        KEEP_FOUND_S
    };
    now.saturating_sub(created) < keep
}

/// Accepted candidates on `track`, best hit rate first, each moved by its
/// fix. Copies of one translation (same lines, other release) count once.
pub fn accepted(
    trials: &[Trial],
    track: u64,
    cues: &[Option<Vec<Cue>>],
    lang: &str,
) -> Vec<MenuEntry> {
    let mut chosen: Vec<&Trial> = trials
        .iter()
        .filter(|t| t.track == track && t.accepted)
        .collect();
    chosen.sort_by(|a, b| b.verdict.hit_rate.total_cmp(&a.verdict.hit_rate));
    let mut seen = std::collections::HashSet::new();
    let mut out = Vec::new();
    for trial in chosen {
        let (Some(cues), Some(fix)) = (
            cues.get(trial.candidate).and_then(Option::as_ref),
            trial.verdict.fix,
        ) else {
            continue;
        };
        let text = cues.iter().map(|c| c.text.as_str()).collect::<Vec<_>>();
        if !seen.insert(fnv(&text.join("\n"))) {
            continue;
        }
        out.push(MenuEntry {
            lang: lang.to_owned(),
            srt: srt::render(cues, fix),
        });
        if out.len() == MAX_OFFERED {
            break;
        }
    }
    out
}

/// FNV-1a: stable across builds, enough to name files and spot copies.
fn fnv(text: &str) -> u64 {
    text.bytes().fold(0xcbf2_9ce4_8422_2325, |hash, byte| {
        (hash ^ u64::from(byte)).wrapping_mul(0x0000_0100_0000_01b3)
    })
}

struct Kept {
    created: u64,
    entries: Vec<MenuEntry>,
}

/// Menus of recent requests. URLs use the entry number, never the request.
#[derive(Default)]
pub struct MenuCache {
    by_request: HashMap<String, u64>,
    entries: HashMap<u64, Kept>,
    order: VecDeque<u64>,
    next: u64,
}

impl MenuCache {
    /// The entry number and menu of `request`, if still fresh at `now`.
    pub fn find(&self, request: &str, now: u64) -> Option<(u64, &[MenuEntry])> {
        let entry = *self.by_request.get(request)?;
        let kept = self.entries.get(&entry)?;
        fresh(kept.created, &kept.entries, now).then_some((entry, kept.entries.as_slice()))
    }

    /// Stores `entries` for `request` as made at `created`, dropping the
    /// oldest request when full.
    pub fn insert(&mut self, request: String, entries: Vec<MenuEntry>, created: u64) -> u64 {
        let entry = self.next;
        self.next += 1;
        if let Some(old) = self.by_request.insert(request, entry) {
            self.entries.remove(&old);
            self.order.retain(|&e| e != old);
        }
        self.entries.insert(entry, Kept { created, entries });
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
        let kept = self.entries.get(&entry)?;
        kept.entries.get(n.checked_sub(1)?).map(|e| e.srt.as_str())
    }
}

#[derive(Serialize, Deserialize)]
struct Stored {
    created: u64,
    entries: Vec<MenuEntry>,
}

fn stored_path(dir: &Path, request: &str) -> PathBuf {
    dir.join(format!("{:016x}.json", fnv(request)))
}

/// A fresh menu for `request` saved under `dir`, with when it was made.
pub fn load(dir: &Path, request: &str, now: u64) -> Option<(u64, Vec<MenuEntry>)> {
    let bytes = std::fs::read(stored_path(dir, request)).ok()?;
    let stored: Stored = serde_json::from_slice(&bytes).ok()?;
    fresh(stored.created, &stored.entries, now).then_some((stored.created, stored.entries))
}

pub fn save(dir: &Path, request: &str, entries: &[MenuEntry], now: u64) -> std::io::Result<()> {
    std::fs::create_dir_all(dir)?;
    let stored = Stored {
        created: now,
        entries: entries.to_vec(),
    };
    let body = serde_json::to_vec(&stored).map_err(std::io::Error::other)?;
    // Write then rename, so a reader never sees half a file.
    let path = stored_path(dir, request);
    let temp = path.with_extension("tmp");
    std::fs::write(&temp, body)?;
    std::fs::rename(temp, path)
}

pub fn now_s() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::align::{Fix, Reject, Verdict};

    fn entry(text: &str) -> MenuEntry {
        MenuEntry {
            lang: "tur".to_owned(),
            srt: text.to_owned(),
        }
    }

    fn trial(candidate: usize, accepted: bool, hit_rate: f64, shift_ms: i64) -> Trial {
        Trial {
            track: 5,
            candidate,
            accepted,
            elapsed_ms: 1,
            verdict: Verdict {
                fix: Some(Fix {
                    num: 1,
                    den: 1,
                    shift_ms,
                }),
                hit_rate,
                windows: Vec::new(),
                reject: (!accepted).then_some(Reject::LowHits(0)),
            },
        }
    }

    fn cues(texts: &[&str]) -> Option<Vec<Cue>> {
        Some(
            texts
                .iter()
                .enumerate()
                .map(|(i, t)| Cue {
                    start_ms: 1_000 * (i as u64 + 1),
                    end_ms: 1_000 * (i as u64 + 1) + 500,
                    text: (*t).to_owned(),
                })
                .collect(),
        )
    }

    #[test]
    fn accepted_keeps_best_first_and_drops_rejects_and_copies() {
        let trials = [
            trial(0, true, 0.6, 0),
            trial(1, false, 0.9, 0),
            trial(2, true, 0.9, 1_000),
            trial(3, true, 0.8, -500),
            Trial {
                track: 3,
                ..trial(4, true, 1.0, 0)
            },
        ];
        // Candidate 3 is candidate 0's translation, timed for another release.
        let all = [
            cues(&["a", "b"]),
            cues(&["x"]),
            cues(&["c"]),
            cues(&["a", "b"]),
            cues(&["z"]),
        ];
        let offered = accepted(&trials, 5, &all, "tur");
        assert_eq!(
            offered.iter().map(|e| e.srt.as_str()).collect::<Vec<_>>(),
            vec![
                "1\n00:00:02,000 --> 00:00:02,500\nc\n\n",
                "1\n00:00:00,500 --> 00:00:01,000\na\n\n2\n00:00:01,500 --> 00:00:02,000\nb\n\n",
            ]
        );
        assert!(offered.iter().all(|e| e.lang == "tur"));
        assert!(!format!("{offered:?}").contains("00:00"));
    }

    #[test]
    fn cache_honours_freshness_and_evicts() {
        let mut cache = MenuCache::default();
        let found = cache.insert("a".to_owned(), vec![entry("x")], 1_000);
        assert_eq!(
            cache.find("a", 1_000 + KEEP_FOUND_S - 1).map(|f| f.0),
            Some(found)
        );
        assert!(cache.find("a", 1_000 + KEEP_FOUND_S).is_none());
        assert_eq!(cache.srt(found, 1), Some("x"));
        assert_eq!(cache.srt(found, 2), None);

        cache.insert("none".to_owned(), Vec::new(), 1_000);
        assert!(cache.find("none", 1_000 + KEEP_NONE_S - 1).is_some());
        assert!(cache.find("none", 1_000 + KEEP_NONE_S).is_none());

        let again = cache.insert("a".to_owned(), vec![entry("y")], 2_000);
        assert_eq!(cache.srt(found, 1), None, "replaced entry is gone");
        assert_eq!(cache.srt(again, 1), Some("y"));
        for i in 0..CAPACITY {
            cache.insert(format!("r{i}"), vec![entry("z")], 2_000);
        }
        assert!(cache.find("a", 2_000).is_none(), "oldest request evicted");
        assert_eq!(cache.entries.len(), CAPACITY);
        assert_eq!(cache.by_request.len(), CAPACITY);
    }

    #[test]
    fn disk_cache_round_trips_by_hashed_name() {
        let dir = std::env::temp_dir().join(format!(
            "subsync-menu-test-{}-{}",
            std::process::id(),
            now_s()
        ));
        let request = "movie/tt1/Some.Film.2020.mkv/123";
        save(&dir, request, &[entry("x")], 5_000).unwrap();
        let names: Vec<String> = std::fs::read_dir(&dir)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        assert_eq!(names.len(), 1);
        assert!(
            !names[0].contains("tt1") && !names[0].contains("Film"),
            "{names:?}"
        );
        assert_eq!(load(&dir, request, 5_001), Some((5_000, vec![entry("x")])));
        assert_eq!(load(&dir, request, 5_000 + KEEP_FOUND_S), None);
        assert_eq!(load(&dir, "other", 5_001), None);
        save(&dir, request, &[], 6_000).unwrap();
        assert_eq!(load(&dir, request, 6_001), Some((6_000, Vec::new())));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn fnv_is_stable() {
        assert_eq!(fnv(""), 0xcbf2_9ce4_8422_2325);
        assert_eq!(fnv("a"), 0xaf63_dc4c_8601_ec8c);
    }
}
