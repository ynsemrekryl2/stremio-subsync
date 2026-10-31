//! Translation jobs behind the menu's translation entry (SS-026).
//!
//! When nothing in the wanted language fits but an English subtitle does
//! (SS-036), the menu gets a translation entry holding that English SRT.
//! The first list or fetch of it starts one background job per English
//! text; every fetch serves the job's current state: translated lines so
//! far, then the rest in English with a note on the first one saying how
//! far it is and to pick the subtitle again for more (Stremio fetches the
//! file on every pick, SS-042). A finished translation is kept on disk for
//! [`KEEP_DONE_S`], so a later open, or a restart, serves it whole.
//!
//! Logs carry counts only; the English and translated text stay in memory,
//! in served files and in the cache. The dashboard sees each job's progress
//! and the played file's name (SS-047), never the text.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Instant;

use serde::Serialize;
use tokio::sync::Semaphore;

use crate::align::Fix;
use crate::events::{event, now_ms};
use crate::menu::{self, MenuEntry};
use crate::srt::{self, Cue};
use crate::translate::{self, Langs, Translator};

/// A finished translation is good this long (the menu answer around it
/// lasts a day, since a real subtitle may appear later).
pub const KEEP_DONE_S: u64 = menu::KEEP_FOUND_S;
/// Jobs running at once; the rest wait their turn.
const PARALLEL: usize = 2;
/// A failed job is not started again sooner than this.
const RETRY_AFTER_S: u64 = 10 * 60;

const SAME: Fix = Fix {
    num: 1,
    den: 1,
    shift_ms: 0,
};

/// Jobs the dashboard lists, newest first.
const SHOWN: usize = 20;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Phase {
    /// Waiting for one of the [`PARALLEL`] slots.
    Queued,
    Running,
    Done,
    Failed,
}

struct Job {
    phase: Phase,
    /// The file to serve now: translated so far, then English.
    srt: String,
    /// The played file's name, for the dashboard only.
    label: Option<String>,
    lines: usize,
    lines_done: usize,
    blocks: usize,
    queued_ms: u64,
    started_ms: Option<u64>,
    finished_ms: Option<u64>,
    /// A finished translation found in the cache.
    from_disk: bool,
    failure: Option<String>,
}

/// One job as the dashboard sees it: counts and times, no text.
#[derive(Debug, Serialize)]
pub struct JobView {
    pub phase: &'static str,
    pub label: Option<String>,
    pub lines: usize,
    pub lines_done: usize,
    pub blocks: usize,
    pub queued_ms: u64,
    pub started_ms: Option<u64>,
    pub finished_ms: Option<u64>,
    pub from_disk: bool,
    pub failure: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct Status {
    pub lang: String,
    pub model: String,
    pub parallel: usize,
    pub jobs: Vec<JobView>,
}

pub struct Translations {
    translator: Arc<dyn Translator>,
    /// ISO 639-2 target language.
    lang: String,
    /// Part of the job key: another model or prompt is another translation.
    model: String,
    cache_dir: Option<PathBuf>,
    jobs: Mutex<HashMap<u64, Job>>,
    permits: Semaphore,
}

impl Translations {
    pub fn new(
        translator: Arc<dyn Translator>,
        lang: String,
        model: String,
        cache_dir: Option<PathBuf>,
    ) -> Self {
        Self {
            translator,
            lang,
            model,
            cache_dir,
            jobs: Mutex::new(HashMap::new()),
            permits: Semaphore::new(PARALLEL),
        }
    }

    /// The menu entry offering the translation of `english` (an SRT).
    pub fn entry(&self, english: &str) -> MenuEntry {
        MenuEntry {
            lang: label(&self.lang),
            srt: english.to_owned(),
            translated: true,
        }
    }

    fn key(&self, english: &str) -> u64 {
        menu::fnv(&format!(
            "{}\0{}\0{}\0{english}",
            self.lang,
            self.model,
            crate::openrouter::PROMPT_VERSION
        ))
    }

    fn cache_key(key: u64) -> String {
        format!("translation/{key:016x}")
    }

    /// Starts the job for `english` unless it runs, is done, or failed
    /// recently. A translation already on disk is loaded instead. `label`
    /// names the played file on the dashboard; a job started without one
    /// takes the first one given.
    pub fn ensure(self: &Arc<Self>, english: &str, label: Option<&str>) {
        let key = self.key(english);
        let now = now_ms();
        {
            let Ok(mut jobs) = self.jobs.lock() else {
                return;
            };
            if let Some(job) = jobs.get_mut(&key) {
                if job.label.is_none() {
                    job.label = label.map(str::to_owned);
                }
                let failed_at = job.finished_ms.unwrap_or(now);
                let wait_ms = RETRY_AFTER_S * 1_000;
                if job.phase != Phase::Failed || now.saturating_sub(failed_at) < wait_ms {
                    return;
                }
            }
            let cues = srt::parse(english);
            jobs.insert(
                key,
                Job {
                    phase: Phase::Queued,
                    srt: partial(&cues, &[]),
                    label: label.map(str::to_owned),
                    lines: cues.len(),
                    lines_done: 0,
                    blocks: translate::layout(cues.len()).len(),
                    queued_ms: now,
                    started_ms: None,
                    finished_ms: None,
                    from_disk: false,
                    failure: None,
                },
            );
        }
        let this = self.clone();
        let english = english.to_owned();
        tokio::spawn(async move { this.run(key, english).await });
    }

    /// The file to serve for the translation of `english` now, if a job
    /// for it exists.
    pub fn current(&self, english: &str) -> Option<String> {
        let jobs = self.jobs.lock().ok()?;
        jobs.get(&self.key(english)).map(|job| job.srt.clone())
    }

    /// The latest jobs for the dashboard, newest first.
    pub fn status(&self) -> Status {
        let mut jobs: Vec<JobView> = self
            .jobs
            .lock()
            .map(|jobs| {
                jobs.values()
                    .map(|job| JobView {
                        phase: match job.phase {
                            Phase::Queued => "queued",
                            Phase::Running => "running",
                            Phase::Done => "done",
                            Phase::Failed => "failed",
                        },
                        label: job.label.clone(),
                        lines: job.lines,
                        lines_done: job.lines_done,
                        blocks: job.blocks,
                        queued_ms: job.queued_ms,
                        started_ms: job.started_ms,
                        finished_ms: job.finished_ms,
                        from_disk: job.from_disk,
                        failure: job.failure.clone(),
                    })
                    .collect()
            })
            .unwrap_or_default();
        jobs.sort_by_key(|job| std::cmp::Reverse(job.queued_ms));
        jobs.truncate(SHOWN);
        Status {
            lang: self.lang.clone(),
            model: self.model.clone(),
            parallel: PARALLEL,
            jobs,
        }
    }

    fn update(&self, key: u64, change: impl FnOnce(&mut Job)) {
        if let Ok(mut jobs) = self.jobs.lock()
            && let Some(job) = jobs.get_mut(&key)
        {
            change(job);
        }
    }

    async fn run(self: Arc<Self>, key: u64, english: String) {
        if let Some(done) = self.load(key).await {
            event!("translate", "translation served from=disk");
            let lines = srt::parse(&done).len();
            self.update(key, |job| {
                job.phase = Phase::Done;
                job.srt = done;
                (job.lines, job.lines_done) = (lines, lines);
                job.finished_ms = Some(now_ms());
                job.from_disk = true;
            });
            return;
        }
        let Ok(_permit) = self.permits.acquire().await else {
            return;
        };
        self.update(key, |job| {
            job.phase = Phase::Running;
            job.started_ms = Some(now_ms());
        });
        let cues = srt::parse(&english);
        let langs = Langs {
            source: "eng".to_owned(),
            target: self.lang.clone(),
        };
        let started = Instant::now();
        event!(
            "translate",
            "translation started lines={} blocks={}",
            cues.len(),
            translate::layout(cues.len()).len()
        );
        let mut reached = partial(&cues, &[]);
        let result = translate::translate(&*self.translator, &cues, &langs, |done| {
            reached = partial(&cues, done);
            self.update(key, |job| {
                job.srt = reached.clone();
                job.lines_done = done.len();
            });
            event!(
                "translate",
                "translation progress lines={}/{}",
                done.len(),
                cues.len()
            );
        })
        .await;
        let elapsed_s = started.elapsed().as_secs();
        match result {
            Ok(all) => {
                let done = srt::render(&all, SAME);
                self.update(key, |job| {
                    job.phase = Phase::Done;
                    job.srt = done.clone();
                    job.lines_done = all.len();
                    job.finished_ms = Some(now_ms());
                });
                event!(
                    "translate",
                    "translation done lines={} s={elapsed_s}",
                    all.len()
                );
                self.save(key, done).await;
            }
            Err(failure) => {
                event!(
                    "translate",
                    "translation failed failure={failure:?} s={elapsed_s}"
                );
                self.update(key, |job| {
                    job.phase = Phase::Failed;
                    job.srt = reached;
                    job.finished_ms = Some(now_ms());
                    job.failure = Some(format!("{failure:?}"));
                });
            }
        }
    }

    async fn load(&self, key: u64) -> Option<String> {
        let dir = self.cache_dir.clone()?;
        let found = tokio::task::spawn_blocking(move || {
            menu::load(&dir, &Self::cache_key(key), menu::now_s())
        })
        .await
        .ok()??;
        found.2.into_iter().next().map(|entry| entry.srt)
    }

    async fn save(&self, key: u64, srt: String) {
        let Some(dir) = self.cache_dir.clone() else {
            return;
        };
        let entry = MenuEntry {
            lang: self.lang.clone(),
            srt,
            translated: false,
        };
        let written = tokio::task::spawn_blocking(move || {
            menu::save(
                &dir,
                &Self::cache_key(key),
                &[entry],
                menu::now_s(),
                KEEP_DONE_S,
            )
        })
        .await;
        if let Ok(Err(err)) = written {
            event!(
                "cache",
                "translation cache write failed error={:?}",
                err.kind()
            );
        }
    }
}

/// The menu name of a translation into `lang`.
fn label(lang: &str) -> String {
    let name = match lang {
        "tur" => "Türkçe",
        other => other,
    };
    format!("{name} çeviri")
}

/// The file for a translation that has reached `done` (a prefix of
/// `english`, translated): those lines, then the rest in English, the first
/// of them opening with how far it is and how to get more.
pub fn partial(english: &[Cue], done: &[Cue]) -> String {
    let mut out = done.to_vec();
    let rest = english.get(done.len()..).unwrap_or_default();
    if let Some((first, more)) = rest.split_first() {
        let percent = done.len() * 100 / english.len().max(1);
        out.push(Cue {
            text: format!(
                "[SubSync: çeviri %{percent}, devamı için altyazıyı yeniden seçin]\n{}",
                first.text
            ),
            ..first.clone()
        });
        out.extend_from_slice(more);
    }
    srt::render(&out, SAME)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cue(i: u64, text: &str) -> Cue {
        Cue {
            start_ms: 1_000 + i * 2_000,
            end_ms: 2_000 + i * 2_000,
            text: text.to_owned(),
        }
    }

    #[test]
    fn partial_file_is_translated_then_english_with_a_note() {
        let english: Vec<Cue> = (0..4).map(|i| cue(i, &format!("en {i}"))).collect();
        let done = vec![cue(0, "tr 0")];
        let served = srt::parse(&partial(&english, &done));
        let texts: Vec<&str> = served.iter().map(|c| c.text.as_str()).collect();
        assert_eq!(
            texts,
            vec![
                "tr 0",
                "[SubSync: çeviri %25, devamı için altyazıyı yeniden seçin]\nen 1",
                "en 2",
                "en 3"
            ]
        );
        let times = |cues: &[Cue]| {
            cues.iter()
                .map(|c| (c.start_ms, c.end_ms))
                .collect::<Vec<_>>()
        };
        assert_eq!(times(&served), times(&english), "times unchanged");

        let all: Vec<Cue> = (0..4).map(|i| cue(i, &format!("tr {i}"))).collect();
        assert!(
            !partial(&english, &all).contains("SubSync"),
            "no note when done"
        );
        assert!(partial(&english, &[]).contains("çeviri %0"));
    }

    /// Fails every analysis; counts them.
    struct Down(Mutex<usize>);

    impl Translator for Down {
        fn analyze<'a>(
            &'a self,
            _request: &'a translate::AnalysisRequest,
        ) -> translate::AnalyzeFuture<'a> {
            *self.0.lock().unwrap() += 1;
            Box::pin(async { Err(translate::TranslateError::Transient) })
        }

        fn translate<'a>(
            &'a self,
            _request: &'a translate::BlockRequest,
        ) -> translate::TranslateFuture<'a> {
            Box::pin(async { Err(translate::TranslateError::Transient) })
        }
    }

    #[tokio::test]
    async fn a_failed_job_waits_before_starting_again() {
        let down = Arc::new(Down(Mutex::new(0)));
        let translations = Arc::new(Translations::new(
            down.clone(),
            "tur".to_owned(),
            "test/model".to_owned(),
            None,
        ));
        let english = srt::render(&[cue(0, "en 0"), cue(1, "en 1")], SAME);
        translations.ensure(&english, None);
        tokio::time::timeout(std::time::Duration::from_secs(5), async {
            while translations.status().jobs[0].phase != "failed" {
                tokio::time::sleep(std::time::Duration::from_millis(5)).await;
            }
        })
        .await
        .unwrap();
        translations.ensure(&english, Some("Late.Name.mkv"));
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        assert_eq!(*down.0.lock().unwrap(), 1, "not started again yet");
        // What it served before failing is still served: the English.
        let served = translations.current(&english).unwrap();
        assert!(served.contains("çeviri %0") && served.contains("en 1"));
        let job = &translations.status().jobs[0];
        assert_eq!(job.failure.as_deref(), Some("Translator(Transient)"));
        assert_eq!(
            job.label.as_deref(),
            Some("Late.Name.mkv"),
            "label filled in"
        );
        assert!(job.finished_ms.is_some());
    }

    #[test]
    fn label_names_the_language() {
        assert_eq!(label("tur"), "Türkçe çeviri");
        assert_eq!(label("deu"), "deu çeviri");
    }
}
