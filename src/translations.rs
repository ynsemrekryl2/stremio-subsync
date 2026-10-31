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
//! and the played file's name (SS-047), never the text. It can change the
//! model and its reasoning effort (SS-048); jobs started before keep theirs.
//! With cleanup on (SS-057, also a dashboard choice), the English loses
//! adverts and marks for the hard of hearing before it is translated, and
//! the job shows how many were taken out.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, RwLock};
use std::time::Instant;

use serde::Serialize;
use tokio::sync::Semaphore;

use crate::align::Fix;
use crate::clean::{self, Cleaned};
use crate::events::{event, now_ms};
use crate::menu::{self, MenuEntry};
use crate::settings::{self, Invalid, TranslateSettings};
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
    /// The model and effort it was started with.
    model: String,
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
    /// What cleanup took out; `None` when it was off.
    cleaned: Option<Cleaned>,
}

/// One job as the dashboard sees it: counts and times, no text.
#[derive(Debug, Serialize)]
pub struct JobView {
    pub phase: &'static str,
    pub model: String,
    pub label: Option<String>,
    pub lines: usize,
    pub lines_done: usize,
    pub blocks: usize,
    pub queued_ms: u64,
    pub started_ms: Option<u64>,
    pub finished_ms: Option<u64>,
    pub from_disk: bool,
    pub failure: Option<String>,
    pub cleaned: Option<Cleaned>,
}

#[derive(Debug, Serialize)]
pub struct Status {
    pub lang: String,
    /// What new jobs use, as [`TranslateSettings::key_label`].
    pub model: String,
    pub settings: TranslateSettings,
    pub parallel: usize,
    pub jobs: Vec<JobView>,
}

/// Builds the translator for a choice; `None` when it cannot.
pub type Build = Box<dyn Fn(&TranslateSettings) -> Option<Arc<dyn Translator>> + Send + Sync>;

/// The translator new jobs get, and the choice it was built from.
struct Engine {
    translator: Arc<dyn Translator>,
    settings: TranslateSettings,
}

/// Why a new choice was not taken; the old one stays.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Refused {
    Invalid(Invalid),
    /// The translator could not be built for it.
    Unbuilt,
    /// It could not be written to the settings file.
    Save(std::io::ErrorKind),
}

pub struct Translations {
    build: Build,
    engine: RwLock<Engine>,
    /// ISO 639-2 target language.
    lang: String,
    cache_dir: Option<PathBuf>,
    /// Where a choice made on the dashboard is kept across restarts.
    settings_file: Option<PathBuf>,
    jobs: Mutex<HashMap<u64, Job>>,
    permits: Semaphore,
}

impl Translations {
    /// `None` when `build` cannot make a translator for `settings`.
    pub fn new(
        build: Build,
        settings: TranslateSettings,
        lang: String,
        cache_dir: Option<PathBuf>,
    ) -> Option<Self> {
        let translator = build(&settings)?;
        Some(Self {
            build,
            engine: RwLock::new(Engine {
                translator,
                settings,
            }),
            lang,
            cache_dir,
            settings_file: None,
            jobs: Mutex::new(HashMap::new()),
            permits: Semaphore::new(PARALLEL),
        })
    }

    /// Saves choices made on the dashboard to `path`.
    pub fn with_settings_file(mut self, path: PathBuf) -> Self {
        self.settings_file = Some(path);
        self
    }

    /// The choice new jobs use.
    pub fn settings(&self) -> TranslateSettings {
        self.engine
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .settings
            .clone()
    }

    /// Makes `settings` the choice for new jobs, saved first so a restart
    /// keeps it. Running jobs finish with the translator they started with.
    pub async fn change(&self, settings: TranslateSettings) -> Result<(), Refused> {
        settings.check().map_err(Refused::Invalid)?;
        let translator = (self.build)(&settings).ok_or(Refused::Unbuilt)?;
        if let Some(path) = self.settings_file.clone() {
            let saved = settings.clone();
            tokio::task::spawn_blocking(move || settings::save(&path, &saved))
                .await
                .map_err(|_| Refused::Save(std::io::ErrorKind::Other))?
                .map_err(|err| Refused::Save(err.kind()))?;
        }
        event!(
            "translate",
            "translation settings changed model={}",
            settings.key_label()
        );
        let mut engine = self
            .engine
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        *engine = Engine {
            translator,
            settings,
        };
        Ok(())
    }

    /// The translator new jobs get, the model label in their key and
    /// whether they clean the English first.
    fn engine(&self) -> (Arc<dyn Translator>, String, bool) {
        let engine = self
            .engine
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        (
            engine.translator.clone(),
            engine.settings.key_label(),
            engine.settings.clean,
        )
    }

    /// The menu entry offering the translation of `english` (an SRT).
    pub fn entry(&self, english: &str) -> MenuEntry {
        MenuEntry {
            lang: label(&self.lang),
            srt: english.to_owned(),
            translated: true,
        }
    }

    /// Another model or prompt is another translation.
    fn key(&self, english: &str, model: &str) -> u64 {
        menu::fnv(&format!(
            "{}\0{model}\0{}\0{english}",
            self.lang,
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
        let (translator, model, cleanup) = self.engine();
        let key = self.key(english, &model);
        let now = now_ms();
        let cues = {
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
            let (cues, cleaned) = source(english, cleanup);
            jobs.insert(
                key,
                Job {
                    phase: Phase::Queued,
                    model,
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
                    cleaned,
                },
            );
            if let Some(c) = cleaned {
                event!(
                    "translate",
                    "translation source cleaned ads={} marks={} tags={} emptied={}",
                    c.ads,
                    c.marks,
                    c.tags,
                    c.emptied
                );
            }
            cues
        };
        let this = self.clone();
        tokio::spawn(async move { this.run(key, cues, translator).await });
    }

    /// The file to serve for the translation of `english` now, if a job
    /// for it exists.
    pub fn current(&self, english: &str) -> Option<String> {
        let key = self.key(english, &self.engine().1);
        let jobs = self.jobs.lock().ok()?;
        jobs.get(&key).map(|job| job.srt.clone())
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
                        model: job.model.clone(),
                        label: job.label.clone(),
                        lines: job.lines,
                        lines_done: job.lines_done,
                        blocks: job.blocks,
                        queued_ms: job.queued_ms,
                        started_ms: job.started_ms,
                        finished_ms: job.finished_ms,
                        from_disk: job.from_disk,
                        failure: job.failure.clone(),
                        cleaned: job.cleaned,
                    })
                    .collect()
            })
            .unwrap_or_default();
        jobs.sort_by_key(|job| std::cmp::Reverse(job.queued_ms));
        jobs.truncate(SHOWN);
        let settings = self.settings();
        Status {
            lang: self.lang.clone(),
            model: settings.key_label(),
            settings,
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

    async fn run(self: Arc<Self>, key: u64, cues: Vec<Cue>, translator: Arc<dyn Translator>) {
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
        let result = translate::translate(&*translator, &cues, &langs, |done| {
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

/// The cues to translate from `english`, cleaned when `cleanup` is on.
fn source(english: &str, cleanup: bool) -> (Vec<Cue>, Option<Cleaned>) {
    let cues = srt::parse(english);
    if cleanup {
        let (cues, cleaned) = clean::clean(&cues);
        (cues, Some(cleaned))
    } else {
        (cues, None)
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

    fn test_settings(model: &str) -> TranslateSettings {
        TranslateSettings {
            model: model.to_owned(),
            reasoning: settings::MODEL_DEFAULT.to_owned(),
            clean: false,
        }
    }

    /// Always `translator`, whatever the choice.
    fn fixed(translator: Arc<dyn Translator>) -> Translations {
        Translations::new(
            Box::new(move |_| Some(translator.clone())),
            test_settings("test/model"),
            "tur".to_owned(),
            None,
        )
        .unwrap()
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
        let translations = Arc::new(fixed(down.clone()));
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

    /// Translates each line as `<model> <text>`, so a test sees which
    /// translator did a job.
    struct Named(String);

    impl Translator for Named {
        fn analyze<'a>(
            &'a self,
            _request: &'a translate::AnalysisRequest,
        ) -> translate::AnalyzeFuture<'a> {
            Box::pin(async {
                Ok(translate::Analysis {
                    summary: "A film".to_owned(),
                    characters: Vec::new(),
                    glossary: Vec::new(),
                })
            })
        }

        fn translate<'a>(
            &'a self,
            request: &'a translate::BlockRequest,
        ) -> translate::TranslateFuture<'a> {
            Box::pin(async move {
                Ok(request
                    .context
                    .iter()
                    .filter(|l| request.output_ids.contains(&l.id))
                    .map(|l| translate::Line {
                        id: l.id,
                        text: format!("{} {}", self.0, l.text),
                    })
                    .collect())
            })
        }
    }

    async fn finished(translations: &Translations, english: &str) -> String {
        tokio::time::timeout(std::time::Duration::from_secs(5), async {
            loop {
                match translations.current(english) {
                    Some(srt) if !srt.contains("SubSync") => break srt,
                    _ => tokio::time::sleep(std::time::Duration::from_millis(5)).await,
                }
            }
        })
        .await
        .unwrap()
    }

    #[tokio::test]
    async fn a_new_choice_translates_new_jobs_and_is_saved() {
        let file = std::env::temp_dir()
            .join(format!(
                "subsync-translations-settings-{}-{}",
                std::process::id(),
                now_ms()
            ))
            .join("settings.json");
        let translations = Arc::new(
            Translations::new(
                Box::new(|s: &TranslateSettings| {
                    // A model the translator cannot be built for.
                    (s.model != "broken/model")
                        .then(|| Arc::new(Named(s.model.clone())) as Arc<dyn Translator>)
                }),
                test_settings("old/model"),
                "tur".to_owned(),
                None,
            )
            .unwrap()
            .with_settings_file(file.clone()),
        );
        let english = srt::render(&[cue(0, "en 0"), cue(1, "en 1")], SAME);
        translations.ensure(&english, Some("A.mkv"));
        assert!(
            finished(&translations, &english)
                .await
                .contains("old/model en 1")
        );

        let refused = [
            (test_settings("no-vendor"), Refused::Invalid(Invalid::Model)),
            (
                TranslateSettings {
                    model: "new/model".to_owned(),
                    reasoning: "huge".to_owned(),
                    clean: false,
                },
                Refused::Invalid(Invalid::Reasoning),
            ),
            (test_settings("broken/model"), Refused::Unbuilt),
        ];
        for (choice, why) in refused {
            assert_eq!(translations.change(choice).await, Err(why));
        }
        assert_eq!(translations.settings(), test_settings("old/model"), "kept");
        assert!(!file.exists(), "nothing saved");

        let chosen = TranslateSettings {
            model: "new/model".to_owned(),
            reasoning: "high".to_owned(),
            clean: false,
        };
        translations.change(chosen.clone()).await.unwrap();
        assert_eq!(settings::load(&file), Ok(Some(chosen.clone())));
        let status = translations.status();
        assert_eq!(status.model, "new/model reasoning=high");
        assert_eq!(status.settings, chosen);

        // Same English, new model: a new job, translated by the new one.
        translations.ensure(&english, Some("A.mkv"));
        assert!(
            finished(&translations, &english)
                .await
                .contains("new/model en 1")
        );
        let models: Vec<String> = translations
            .status()
            .jobs
            .into_iter()
            .map(|job| job.model)
            .collect();
        assert_eq!(models.len(), 2);
        assert!(models.contains(&"old/model".to_owned()));
        assert!(models.contains(&"new/model reasoning=high".to_owned()));
        let _ = std::fs::remove_dir_all(file.parent().unwrap());
    }

    #[tokio::test]
    async fn cleanup_takes_adverts_and_marks_out_before_translating() {
        let translations = Arc::new(
            Translations::new(
                Box::new(|s: &TranslateSettings| {
                    Some(Arc::new(Named(s.model.clone())) as Arc<dyn Translator>)
                }),
                TranslateSettings {
                    clean: true,
                    ..test_settings("m/x")
                },
                "tur".to_owned(),
                None,
            )
            .unwrap(),
        );
        let english = srt::render(
            &[
                cue(0, "Subtitles by someone"),
                cue(1, "[door creaks]"),
                cue(2, "JOHN: (sighs) Hello."),
                cue(3, "Goodbye."),
            ],
            SAME,
        );
        translations.ensure(&english, None);
        let served = srt::parse(&finished(&translations, &english).await);
        let texts: Vec<&str> = served.iter().map(|c| c.text.as_str()).collect();
        assert_eq!(texts, vec!["m/x Hello.", "m/x Goodbye."]);
        assert_eq!(served[0].start_ms, cue(2, "").start_ms, "source times");
        let job = &translations.status().jobs[0];
        assert_eq!(job.lines, 2);
        assert_eq!(
            job.cleaned,
            Some(Cleaned {
                ads: 1,
                marks: 3,
                tags: 0,
                emptied: 1,
            })
        );
        assert_eq!(job.model, format!("m/x clean={}", clean::VERSION));

        // Off: the same English is another translation, line for line.
        let off = TranslateSettings {
            clean: false,
            ..test_settings("m/x")
        };
        translations.change(off).await.unwrap();
        translations.ensure(&english, None);
        let served = srt::parse(&finished(&translations, &english).await);
        assert_eq!(served.len(), 4);
        assert_eq!(served[1].text, "m/x [door creaks]");
        let jobs = translations.status().jobs;
        assert!(
            jobs.iter()
                .any(|job| job.model == "m/x" && job.cleaned.is_none())
        );
    }

    #[test]
    fn label_names_the_language() {
        assert_eq!(label("tur"), "Türkçe çeviri");
        assert_eq!(label("deu"), "deu çeviri");
    }
}
