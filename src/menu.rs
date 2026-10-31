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

/// The menu name of every notice: Stremio lists it as a language of its own.
pub const NOTICE_LANG: &str = "SubSync Hata";
/// A notice's one subtitle line runs from the start to past any video's end.
const NOTICE_SPAN: &str = "00:00:00,000 --> 99:59:59,000";

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
    /// Every notice, for tests that tell them apart by their text.
    #[cfg(test)]
    pub const ALL: [Notice; 10] = [
        Self::NoEnglishTrack,
        Self::ImageOnly,
        Self::NotIndexed,
        Self::NoStreamMatch,
        Self::NoCandidates,
        Self::NoneAccepted,
        Self::NotMkv,
        Self::Unsupported,
        Self::Failed,
        Self::NotReady,
    ];

    /// What happened and what the viewer can do about it.
    fn text(self) -> (&'static str, &'static str) {
        match self {
            Self::NoEnglishTrack => (
                "Dosyada İngilizce altyazı yok, senkron için cetvel bulunamadı.",
                "Başka bir sürüm seçin; WEB-DL sürümlerinde genelde metin altyazı olur.",
            ),
            Self::ImageOnly => (
                "Dosyanın İngilizce altyazısı resim biçiminde ve zamanları okunamadı.",
                "WEB-DL ya da BluRay rip bir sürüm seçin.",
            ),
            Self::NotIndexed => (
                "Dosyanın İngilizce altyazısı dizinlenmemiş; okumak için dosyanın tamamı gerekir.",
                "Başka bir sürüm seçin.",
            ),
            Self::NoStreamMatch => (
                "Oynatılan dosya AIOStreams listesinde bulunamadı.",
                "Sürümü AIOStreams listesinden seçip açın.",
            ),
            Self::NoCandidates => (
                "Bu yapım için aranan dilde altyazı bulunamadı.",
                "Menüde çeviri varsa onu seçin; altyazı 1 gün sonra yeniden aranır.",
            ),
            Self::NoneAccepted => (
                "Bulunan altyazıların hiçbiri bu dosyaya baştan sona oturmadı.",
                "Menüde çeviri varsa onu seçin ya da başka bir sürüm deneyin.",
            ),
            Self::NotMkv => (
                "Dosya MKV değil ya da debrid servisi henüz hazırlamadı.",
                "Videoyu bir dakika sonra yeniden açın.",
            ),
            Self::Unsupported => (
                "Bu dosya biçiminde (AVI, TS ya da parçalı MP4) altyazı zamanları okunamıyor.",
                "MKV sürümünü seçin.",
            ),
            Self::Failed => (
                "Bir kaynağa ulaşılamadı.",
                "Videoyu bir dakika sonra yeniden açın.",
            ),
            Self::NotReady => (
                "Altyazı hâlâ hazırlanıyor.",
                "Videoyu birkaç saniye sonra yeniden açın.",
            ),
        }
    }

    /// The menu entry: named [`NOTICE_LANG`] whatever the reason; picked,
    /// it shows what happened and what to do for the whole video, so it is
    /// seen wherever the viewer picks it.
    pub fn entry(self) -> MenuEntry {
        let (what, todo) = self.text();
        MenuEntry {
            lang: NOTICE_LANG.to_owned(),
            srt: format!("1\n{NOTICE_SPAN}\nSubSync Hata: {what}\nNe yapmalı: {todo}\n\n"),
            translated: false,
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
    /// An automatic translation of `srt` (English) into the wanted
    /// language (SS-026): the served file comes from the translation job.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub translated: bool,
}

impl std::fmt::Debug for MenuEntry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MenuEntry")
            .field("lang", &self.lang)
            .field("srt_len", &self.srt.len())
            .field("translated", &self.translated)
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
            translated: false,
        });
        if out.len() == MAX_OFFERED {
            break;
        }
    }
    out
}

/// FNV-1a: stable across builds, enough to name files and spot copies.
pub fn fnv(text: &str) -> u64 {
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

    /// Menu entry `n` (from 1) of `entry`.
    pub fn get(&self, entry: u64, n: usize) -> Option<&MenuEntry> {
        self.entries.get(&entry)?.entries.get(n.checked_sub(1)?)
    }

    /// Forgets every menu (SS-058); how many went. Entry numbers go on
    /// from where they were, so an old link never names a new menu.
    pub fn clear(&mut self) -> usize {
        let cleared = self.entries.len();
        self.by_request.clear();
        self.entries.clear();
        self.order.clear();
        cleared
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

/// Deletes the saved menus right under `dir` (SS-058): only files named
/// like [`save`] names them, so nothing else there is touched, nor any
/// folder. How many went; a missing `dir` has none.
pub fn clear(dir: &Path) -> std::io::Result<usize> {
    let listing = match std::fs::read_dir(dir) {
        Ok(listing) => listing,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(0),
        Err(err) => return Err(err),
    };
    let mut cleared = 0;
    for item in listing {
        let item = item?;
        let name = item.file_name();
        let saved = name
            .to_str()
            .and_then(|n| n.strip_suffix(".json"))
            .is_some_and(|stem| {
                stem.len() == 16 && stem.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
            });
        if saved && item.file_type()?.is_file() {
            std::fs::remove_file(item.path())?;
            cleared += 1;
        }
    }
    Ok(cleared)
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
            translated: false,
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
        assert_eq!(cache.get(found, 1).map(|e| e.srt.as_str()), Some("x"));
        assert_eq!(cache.get(found, 2).map(|e| e.srt.as_str()), None);

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
        assert_eq!(
            cache.get(found, 1).map(|e| e.srt.as_str()),
            None,
            "replaced entry is gone"
        );
        assert_eq!(cache.get(again, 1).map(|e| e.srt.as_str()), Some("y"));
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
    fn clearing_forgets_menus_but_not_their_numbers_or_other_files() {
        let mut cache = MenuCache::default();
        let first = cache.insert("a".to_owned(), vec![entry("x")], 0, KEEP_FOUND_S);
        cache.insert("b".to_owned(), vec![entry("y")], 0, KEEP_FOUND_S);
        assert_eq!(cache.clear(), 2);
        assert!(cache.find("a", 1).is_none() && cache.get(first, 1).is_none());
        let next = cache.insert("a".to_owned(), vec![entry("z")], 0, KEEP_FOUND_S);
        assert!(next > first + 1, "an old link never names a new menu");

        let dir = std::env::temp_dir().join(format!(
            "subsync-menu-clear-{}-{}",
            std::process::id(),
            now_s()
        ));
        assert_eq!(clear(&dir).unwrap(), 0, "no folder yet");
        save(&dir, "a", &[entry("x")], 0, KEEP_FOUND_S).unwrap();
        save(&dir, "b", &[entry("y")], 0, KEEP_FOUND_S).unwrap();
        save(
            &dir.join("translations"),
            "c",
            &[entry("z")],
            0,
            KEEP_FOUND_S,
        )
        .unwrap();
        std::fs::write(dir.join("settings.json"), "{}").unwrap();
        std::fs::write(dir.join("0123456789abcdef.txt"), "").unwrap();
        assert_eq!(clear(&dir).unwrap(), 2);
        assert_eq!(load(&dir, "a", 1), None);
        assert!(
            load(&dir.join("translations"), "c", 1).is_some(),
            "folder kept"
        );
        assert!(dir.join("settings.json").exists() && dir.join("0123456789abcdef.txt").exists());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn notice_entry_is_one_name_and_explains_for_the_whole_video() {
        let entry = Notice::NoEnglishTrack.entry();
        let cues = srt::parse(&entry.srt);
        assert_eq!(cues.len(), 1);
        assert_eq!(cues[0].start_ms, 0);
        assert!(cues[0].end_ms >= 99 * 3_600_000);
        assert_eq!(
            cues[0].text,
            "SubSync Hata: Dosyada İngilizce altyazı yok, senkron için cetvel bulunamadı.\n\
             Ne yapmalı: Başka bir sürüm seçin; WEB-DL sürümlerinde genelde metin altyazı olur."
        );
        let mut texts = std::collections::HashSet::new();
        for notice in Notice::ALL {
            let entry = notice.entry();
            assert_eq!(entry.lang, NOTICE_LANG, "{notice:?}");
            let cues = srt::parse(&entry.srt);
            assert_eq!(cues.len(), 1, "{notice:?}");
            let lines: Vec<&str> = cues[0].text.lines().collect();
            assert_eq!(lines.len(), 2, "{notice:?}");
            assert!(lines[0].starts_with("SubSync Hata: "), "{notice:?}");
            assert!(lines[1].starts_with("Ne yapmalı: "), "{notice:?}");
            // Tests tell notices apart by their text.
            assert!(texts.insert(entry.srt), "{notice:?}");
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
