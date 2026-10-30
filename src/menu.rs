//! Delivery (M5): the subtitles offered to Stremio for one request, kept in
//! memory and on disk so a file opened again needs no probes.
//!
//! The request key (kind, IMDb id, file name, size) lives only in memory;
//! on disk a file is named after a hash of it. Subtitle text is stored, never
//! printed: `Debug` shows the menu text and length only.

use serde::{Deserialize, Serialize};
use std::collections::{HashMap, VecDeque};
use std::path::{Path, PathBuf};

use crate::align::{self, Trial};
use crate::ruler::Event;
use crate::srt::{self, Cue};

/// Requests kept in memory.
const CAPACITY: usize = 32;
/// At most this many subtitles are offered for one file.
const MAX_OFFERED: usize = 3;
/// How long an answer stays good: one with subtitles; one that says why
/// there are none (the file has no English track, nothing fits...); and a
/// passing one (a timeout, a source down), kept only so its notice can be
/// fetched.
pub const KEEP_FOUND_S: u64 = 30 * 24 * 3600;
pub const KEEP_NONE_S: u64 = 24 * 3600;
pub const KEEP_PASSING_S: u64 = 60;

/// Why no subtitle is offered. Shown in Stremio's menu as one entry, so the
/// viewer is not left waiting for nothing; the entry's "subtitle" explains.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Notice {
    /// The file has no English subtitle to align against.
    NoEnglishTrack,
    /// It has one, but as images whose times Cues do not give (or too few
    /// of them for a ruler).
    ImageOnly,
    /// It has one, but Cues do not index it; reading it means the whole file.
    NotIndexed,
    /// The played file was not found in the stream list.
    NoStreamMatch,
    /// No subtitle in the wanted language exists for this title.
    NoCandidates,
    /// Some exist, none fits this file from start to end.
    NoneAccepted,
    /// Not an MKV, or the debrid service is not serving it yet.
    NotMkv,
    /// A container whose subtitle times cannot be read cheaply: AVI,
    /// MPEG-TS, or a fragmented MP4 (SS-032).
    Unsupported,
    /// A source or the file could not be reached this time.
    Failed,
    /// Still working; the next open will have the answer.
    NotReady,
}

impl Notice {
    /// Menu name (short: Stremio's menu cuts long names) and explanation.
    fn text(self) -> (&'static str, &'static str) {
        match self {
            Self::NoEnglishTrack => (
                "⚠ İng. iz yok",
                "SubSync: Bu dosyada İngilizce altyazı yok, senkron için cetvel \
                 bulunamadı. Başka bir sürüm deneyin; WEB-DL sürümlerinde genelde metin \
                 altyazı olur.",
            ),
            Self::ImageOnly => (
                "⚠ İng. resim",
                "SubSync: Bu dosyanın İngilizce altyazısı resim biçiminde ve \
                 zamanları okunamadı, cetvel çıkarılamadı. WEB-DL ya da BluRay rip \
                 bir sürüm deneyin.",
            ),
            Self::NotIndexed => (
                "⚠ İz okunmuyor",
                "SubSync: Bu dosyanın İngilizce altyazısı dizinlenmemiş; okumak için \
                 dosyanın tamamı gerekir. Başka bir sürüm deneyin.",
            ),
            Self::NoStreamMatch => (
                "⚠ Dosya yok",
                "SubSync: Oynatılan dosya AIOStreams listesinde bulunamadı.",
            ),
            Self::NoCandidates => (
                "⚠ Aday yok",
                "SubSync: Bu yapım için aranan dilde altyazı bulunamadı.",
            ),
            Self::NoneAccepted => (
                "⚠ Uyan yok",
                "SubSync: Bulunan altyazıların hiçbiri bu dosyaya baştan sona oturmadı.",
            ),
            Self::NotMkv => (
                "⚠ MKV değil",
                "SubSync: Dosya MKV değil ya da debrid servisi henüz hazırlamadı. \
                 Hazırsa videoyu biraz sonra yeniden açın.",
            ),
            Self::Unsupported => (
                "⚠ Biçim dışı",
                "SubSync: Bu dosya biçiminde (AVI, TS ya da parçalı MP4) gömülü \
                 altyazı zamanları okunamıyor, senkron için cetvel yok. MKV sürümü deneyin.",
            ),
            Self::Failed => (
                "⚠ Geçici hata",
                "SubSync: Bir kaynağa ulaşılamadı. Videoyu biraz sonra yeniden açın.",
            ),
            Self::NotReady => (
                "⚠ Hazırlanıyor",
                "SubSync: Altyazı hâlâ hazırlanıyor. Videoyu birkaç saniye sonra yeniden açın.",
            ),
        }
    }

    /// The menu entry: the reason as its name, the explanation as a
    /// subtitle shown for the first ten seconds.
    pub fn entry(self) -> MenuEntry {
        let (lang, message) = self.text();
        MenuEntry {
            lang: lang.to_owned(),
            srt: format!("1\n00:00:01,000 --> 00:00:10,000\n{message}\n\n"),
        }
    }

    /// How long this answer stays good.
    pub fn keep_s(self) -> u64 {
        match self {
            Self::NotMkv | Self::Failed | Self::NotReady => KEEP_PASSING_S,
            _ => KEEP_NONE_S,
        }
    }
}

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

/// Candidates accepted against any of the file's English tracks, best hit
/// rate first, each moved by the fix of its best trial. Copies of one
/// translation (same lines, other release) count once. One accepted only
/// on the second, local pass (SS-039) comes after every globally accepted
/// one and is moved line by line against its ruler track.
///
/// Any track will do: a file's second English track (often SDH) can trip a
/// window check the other one passes.
pub fn accepted(
    trials: &[Trial],
    rulers: &[(u64, Vec<Event>)],
    cues: &[Option<Vec<Cue>>],
    lang: &str,
) -> Vec<MenuEntry> {
    let mut chosen: Vec<&Trial> = trials.iter().filter(|t| t.accepted).collect();
    chosen.sort_by(|a, b| {
        let global = |t: &Trial| t.verdict.accepted().is_some();
        global(b)
            .cmp(&global(a))
            .then(b.verdict.hit_rate.total_cmp(&a.verdict.hit_rate))
    });
    let mut seen = std::collections::HashSet::new();
    let mut taken = std::collections::HashSet::new();
    let mut out = Vec::new();
    for trial in chosen {
        if !taken.insert(trial.candidate) {
            continue;
        }
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
        let srt = if trial.verdict.accepted().is_some() {
            srt::render(cues, fix)
        } else {
            let Some((_, ruler)) = rulers.iter().find(|(track, _)| *track == trial.track) else {
                continue;
            };
            let extra = align::local_shifts(ruler, &srt::events(cues), fix);
            srt::render_local(cues, fix, &extra)
        };
        out.push(MenuEntry {
            lang: lang.to_owned(),
            srt,
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
    keep_s: u64,
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
        (now.saturating_sub(kept.created) < kept.keep_s).then_some((entry, kept.entries.as_slice()))
    }

    /// Stores `entries` for `request` as made at `created`, good for
    /// `keep_s`, dropping the oldest request when full.
    pub fn insert(
        &mut self,
        request: String,
        entries: Vec<MenuEntry>,
        created: u64,
        keep_s: u64,
    ) -> u64 {
        let entry = self.next;
        self.next += 1;
        if let Some(old) = self.by_request.insert(request, entry) {
            self.entries.remove(&old);
            self.order.retain(|&e| e != old);
        }
        self.entries.insert(
            entry,
            Kept {
                created,
                keep_s,
                entries,
            },
        );
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
    keep_s: u64,
    entries: Vec<MenuEntry>,
}

fn stored_path(dir: &Path, request: &str) -> PathBuf {
    dir.join(format!("{:016x}.json", fnv(request)))
}

/// A fresh menu for `request` saved under `dir`: when it was made, how long
/// it is good for, and its entries.
pub fn load(dir: &Path, request: &str, now: u64) -> Option<(u64, u64, Vec<MenuEntry>)> {
    let bytes = std::fs::read(stored_path(dir, request)).ok()?;
    let stored: Stored = serde_json::from_slice(&bytes).ok()?;
    (now.saturating_sub(stored.created) < stored.keep_s).then_some((
        stored.created,
        stored.keep_s,
        stored.entries,
    ))
}

pub fn save(
    dir: &Path,
    request: &str,
    entries: &[MenuEntry],
    now: u64,
    keep_s: u64,
) -> std::io::Result<()> {
    std::fs::create_dir_all(dir)?;
    let stored = Stored {
        created: now,
        keep_s,
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
                local: None,
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
    fn locally_accepted_comes_last_and_moves_line_by_line() {
        // Ruler lines every 3 s; the candidate's second half runs 400 ms late.
        let ruler: Vec<Event> = (0..40)
            .map(|i| Event {
                start_ms: 10_000 + 3_000 * i,
                end_ms: 11_000 + 3_000 * i,
            })
            .collect();
        let late: Vec<Cue> = ruler
            .iter()
            .enumerate()
            .map(|(i, e)| {
                let late = if i < 20 { 0 } else { 400 };
                Cue {
                    start_ms: e.start_ms + late,
                    end_ms: e.end_ms + late,
                    text: format!("l{i}"),
                }
            })
            .collect();
        let local = |candidate: usize, hit_rate: f64| Trial {
            verdict: Verdict {
                reject: Some(Reject::Disagree(1)),
                local: Some(align::LocalCheck {
                    hit_rates: vec![1.0; 3],
                    span_ms: 400,
                }),
                ..trial(candidate, true, hit_rate, 0).verdict
            },
            ..trial(candidate, true, hit_rate, 0)
        };
        let trials = [
            local(0, 0.9),
            trial(1, true, 0.6, 0),
            Trial {
                track: 9,
                ..local(2, 0.95)
            },
        ];
        let all = [Some(late.clone()), cues(&["g"]), cues(&["n"])];
        let rulers = [(5, ruler.clone())];
        let offered = accepted(&trials, &rulers, &all, "tur");
        let same = Fix {
            num: 1,
            den: 1,
            shift_ms: 0,
        };
        let moved = srt::render_local(
            &late,
            same,
            &align::local_shifts(&ruler, &srt::events(&late), same),
        );
        assert_ne!(moved, srt::render(&late, same));
        // Global first despite its lower hit rate; the one on a track with
        // no ruler is dropped.
        assert_eq!(
            offered.iter().map(|e| e.srt.as_str()).collect::<Vec<_>>(),
            vec!["1\n00:00:01,000 --> 00:00:01,500\ng\n\n", moved.as_str()]
        );
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
        let offered = accepted(&trials, &[], &all, "tur");
        assert_eq!(
            offered.iter().map(|e| e.srt.as_str()).collect::<Vec<_>>(),
            vec![
                // Accepted only against the other track, still offered.
                "1\n00:00:01,000 --> 00:00:01,500\nz\n\n",
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
        let found = cache.insert("a".to_owned(), vec![entry("x")], 1_000, KEEP_FOUND_S);
        assert_eq!(
            cache.find("a", 1_000 + KEEP_FOUND_S - 1).map(|f| f.0),
            Some(found)
        );
        assert!(cache.find("a", 1_000 + KEEP_FOUND_S).is_none());
        assert_eq!(cache.srt(found, 1), Some("x"));
        assert_eq!(cache.srt(found, 2), None);

        let notice = Notice::NoEnglishTrack;
        cache.insert(
            "none".to_owned(),
            vec![notice.entry()],
            1_000,
            notice.keep_s(),
        );
        assert!(cache.find("none", 1_000 + KEEP_NONE_S - 1).is_some());
        assert!(cache.find("none", 1_000 + KEEP_NONE_S).is_none());
        let passing = Notice::Failed;
        cache.insert(
            "down".to_owned(),
            vec![passing.entry()],
            1_000,
            passing.keep_s(),
        );
        assert!(cache.find("down", 1_000 + KEEP_PASSING_S).is_none());

        let again = cache.insert("a".to_owned(), vec![entry("y")], 2_000, KEEP_FOUND_S);
        assert_eq!(cache.srt(found, 1), None, "replaced entry is gone");
        assert_eq!(cache.srt(again, 1), Some("y"));
        for i in 0..CAPACITY {
            cache.insert(format!("r{i}"), vec![entry("z")], 2_000, KEEP_FOUND_S);
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
        save(&dir, request, &[entry("x")], 5_000, KEEP_FOUND_S).unwrap();
        let names: Vec<String> = std::fs::read_dir(&dir)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        assert_eq!(names.len(), 1);
        assert!(
            !names[0].contains("tt1") && !names[0].contains("Film"),
            "{names:?}"
        );
        assert_eq!(
            load(&dir, request, 5_001),
            Some((5_000, KEEP_FOUND_S, vec![entry("x")]))
        );
        assert_eq!(load(&dir, request, 5_000 + KEEP_FOUND_S), None);
        assert_eq!(load(&dir, "other", 5_001), None);
        let none = vec![Notice::NoneAccepted.entry()];
        save(&dir, request, &none, 6_000, KEEP_NONE_S).unwrap();
        assert_eq!(load(&dir, request, 6_001), Some((6_000, KEEP_NONE_S, none)));
        assert_eq!(load(&dir, request, 6_000 + KEEP_NONE_S), None);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn notice_entry_names_the_reason_and_explains() {
        let entry = Notice::NoEnglishTrack.entry();
        assert_eq!(entry.lang, "⚠ İng. iz yok");
        let cues = srt::parse(&entry.srt);
        assert_eq!(cues.len(), 1);
        assert_eq!((cues[0].start_ms, cues[0].end_ms), (1_000, 10_000));
        assert!(
            cues[0]
                .text
                .starts_with("SubSync: Bu dosyada İngilizce altyazı yok")
        );
        // Short enough for Stremio's menu.
        for notice in [
            Notice::NoEnglishTrack,
            Notice::ImageOnly,
            Notice::NotIndexed,
            Notice::NoStreamMatch,
            Notice::NoCandidates,
            Notice::NoneAccepted,
            Notice::NotMkv,
            Notice::Unsupported,
            Notice::Failed,
            Notice::NotReady,
        ] {
            assert!(notice.entry().lang.chars().count() <= 14, "{notice:?}");
        }
        assert_eq!(Notice::NotReady.keep_s(), KEEP_PASSING_S);
        assert_eq!(Notice::NoneAccepted.keep_s(), KEEP_NONE_S);
    }

    #[test]
    fn fnv_is_stable() {
        assert_eq!(fnv(""), 0xcbf2_9ce4_8422_2325);
        assert_eq!(fnv("a"), 0xaf63_dc4c_8601_ec8c);
    }
}
