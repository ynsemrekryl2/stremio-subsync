//! Candidate subtitles (M3): list them for a title and language, rank them
//! by release name against the played file, download the best few and read
//! their timing.
//!
//! Candidate URLs and release names reveal what is being watched, and
//! subtitle text is never logged, so `Debug` here prints only counts, codes
//! and flags.

use reqwest::Url;
use serde::{Deserialize, Deserializer};
use serde_json::Value;
use std::future::Future;
use std::pin::Pin;
use std::time::{Duration, Instant};

use crate::extra::PlaybackHints;
use crate::rank::{self, Release};
use crate::ruler::Event;
use crate::srt::{self, SrtReport};

const FETCH_TIMEOUT: Duration = Duration::from_secs(15);
const OPENSUBTITLES_V3: &str = "https://opensubtitles-v3.strem.io/";
/// Larger than any real SRT (a feature film is 50–150 KB); stops a wrong
/// or hostile URL from filling memory.
pub const MAX_SUBTITLE_BYTES: usize = 2 * 1024 * 1024;

/// Why a list or a download failed. Carries no URL.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SourceError {
    Timeout,
    Connect,
    Status(u16),
    Decode,
    TooLarge,
    /// Not an `https` URL.
    BadUrl,
    Other,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Provider {
    OpenSubtitlesV3,
}

/// One subtitle a provider offers. `Debug` is by hand: URL and names stay out.
#[derive(Clone, PartialEq, Eq)]
pub struct Candidate {
    pub provider: Provider,
    /// ISO 639-2, as the provider gives it (`tur`, `eng`, `pob`…).
    pub lang: String,
    pub url: String,
    pub release: Release,
    /// The provider matched it by the file's hash, not only by title.
    pub hash_match: bool,
    pub fps_milli: Option<u32>,
}

impl std::fmt::Debug for Candidate {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Candidate")
            .field("provider", &self.provider)
            .field("lang", &self.lang)
            .field("has_release", &!self.release.is_empty())
            .field("hash_match", &self.hash_match)
            .field("fps_milli", &self.fps_milli)
            .finish()
    }
}

pub type ListFuture<'a> =
    Pin<Box<dyn Future<Output = Result<Vec<Candidate>, SourceError>> + Send + 'a>>;
pub type BodyFuture<'a> = Pin<Box<dyn Future<Output = Result<Vec<u8>, SourceError>> + Send + 'a>>;

/// A subtitle provider; faked in tests.
pub trait SubtitleSource: Send + Sync {
    /// All candidates for a Stremio id (`tt…` or `tt…:season:episode`).
    fn list<'a>(&'a self, kind: &'a str, id: &'a str, hints: &'a PlaybackHints) -> ListFuture<'a>;
    /// The subtitle file, at most [`MAX_SUBTITLE_BYTES`].
    fn download<'a>(&'a self, candidate: &'a Candidate) -> BodyFuture<'a>;
}

/// `tt123` or `tt123:1:2`. Anything else would be pasted into a URL path.
pub fn valid_id(id: &str) -> bool {
    let mut parts = id.split(':');
    let title = parts.next().and_then(|t| t.strip_prefix("tt"));
    let digits = |s: &str| !s.is_empty() && s.len() <= 12 && s.bytes().all(|b| b.is_ascii_digit());
    let rest: Vec<&str> = parts.collect();
    title.is_some_and(digits)
        && (rest.is_empty() || (rest.len() == 2 && rest.iter().all(|p| digits(p))))
}

#[derive(Deserialize)]
struct V3List {
    #[serde(default)]
    subtitles: Vec<V3Entry>,
}

#[derive(Deserialize)]
struct V3Entry {
    url: Option<String>,
    lang: Option<String>,
    m: Option<String>,
    #[serde(rename = "subtitleFileName")]
    file_name: Option<String>,
    #[serde(rename = "releaseGroup")]
    group: Option<String>,
    #[serde(rename = "releaseFormat")]
    format: Option<String>,
    #[serde(rename = "fpsMilli", default, deserialize_with = "lenient_u32")]
    fps_milli: Option<u32>,
}

/// Accepts a number or a numeric string; anything else is absent.
fn lenient_u32<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Option<u32>, D::Error> {
    Ok(match Value::deserialize(deserializer)? {
        Value::Number(n) => n.as_u64().and_then(|n| u32::try_from(n).ok()),
        Value::String(s) => s.trim().parse().ok(),
        _ => None,
    })
}

/// Entries without a URL or a language are dropped.
pub fn parse_v3_list(body: &[u8]) -> Result<Vec<Candidate>, SourceError> {
    let list: V3List = serde_json::from_slice(body).map_err(|_| SourceError::Decode)?;
    Ok(list
        .subtitles
        .into_iter()
        .filter_map(|entry| {
            let release = Release::parse(
                entry.file_name.as_deref().unwrap_or_default(),
                entry.group.as_deref(),
                entry.format.as_deref(),
            );
            Some(Candidate {
                provider: Provider::OpenSubtitlesV3,
                lang: entry.lang.filter(|l| !l.is_empty())?,
                url: entry.url.filter(|u| !u.is_empty())?,
                release,
                hash_match: entry.m.as_deref() == Some("h"),
                fps_milli: entry.fps_milli.filter(|&f| f > 0),
            })
        })
        .collect())
}

/// Stremio's keyless OpenSubtitles add-on. It hands out UTF-8 SRT whatever
/// the original encoding was (`docs/spikes/m3-sources.md`).
pub struct OpenSubtitlesV3 {
    base: Url,
    client: reqwest::Client,
}

impl OpenSubtitlesV3 {
    pub fn new() -> Option<Self> {
        let client = reqwest::Client::builder()
            .timeout(FETCH_TIMEOUT)
            .user_agent(concat!("stremio-subsync/", env!("CARGO_PKG_VERSION")))
            .build()
            .ok()?;
        Some(Self {
            base: Url::parse(OPENSUBTITLES_V3).ok()?,
            client,
        })
    }

    /// Hash and size go along so the add-on can mark hash matches; the
    /// filename does not, it is not needed for that.
    fn list_url(&self, kind: &str, id: &str, hints: &PlaybackHints) -> Option<Url> {
        if !matches!(kind, "movie" | "series") || !valid_id(id) {
            return None;
        }
        let mut extra = form_urlencoded::Serializer::new(String::new());
        if let Some(hash) = &hints.video_hash {
            extra.append_pair("videoHash", hash);
        }
        if let Some(size) = hints.video_size {
            extra.append_pair("videoSize", &size.to_string());
        }
        let extra = extra.finish();
        let path = if extra.is_empty() {
            format!("subtitles/{kind}/{id}.json")
        } else {
            format!("subtitles/{kind}/{id}/{extra}.json")
        };
        self.base.join(&path).ok()
    }
}

fn classify(err: &reqwest::Error) -> SourceError {
    if err.is_timeout() {
        SourceError::Timeout
    } else if err.is_connect() {
        SourceError::Connect
    } else if let Some(status) = err.status() {
        SourceError::Status(status.as_u16())
    } else if err.is_decode() {
        SourceError::Decode
    } else {
        SourceError::Other
    }
}

/// Reads the whole body, failing once it grows past `limit`.
async fn read_limited(
    response: &mut reqwest::Response,
    limit: usize,
) -> Result<Vec<u8>, SourceError> {
    let mut body = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(|err| classify(&err))? {
        if body.len() + chunk.len() > limit {
            return Err(SourceError::TooLarge);
        }
        body.extend_from_slice(&chunk);
    }
    Ok(body)
}

impl SubtitleSource for OpenSubtitlesV3 {
    fn list<'a>(&'a self, kind: &'a str, id: &'a str, hints: &'a PlaybackHints) -> ListFuture<'a> {
        Box::pin(async move {
            let url = self.list_url(kind, id, hints).ok_or(SourceError::BadUrl)?;
            let mut response = self
                .client
                .get(url)
                .send()
                .await
                .and_then(reqwest::Response::error_for_status)
                .map_err(|err| classify(&err))?;
            parse_v3_list(&read_limited(&mut response, MAX_SUBTITLE_BYTES).await?)
        })
    }

    fn download<'a>(&'a self, candidate: &'a Candidate) -> BodyFuture<'a> {
        Box::pin(async move {
            let url = Url::parse(&candidate.url).map_err(|_| SourceError::BadUrl)?;
            if url.scheme() != "https" {
                return Err(SourceError::BadUrl);
            }
            let mut response = self
                .client
                .get(url)
                .send()
                .await
                .and_then(reqwest::Response::error_for_status)
                .map_err(|err| classify(&err))?;
            read_limited(&mut response, MAX_SUBTITLE_BYTES).await
        })
    }
}

/// A downloaded candidate with its timing. `Debug` is by hand: no text.
#[derive(PartialEq, Eq)]
pub struct Fetched {
    pub candidate: Candidate,
    pub score: i32,
    pub text: String,
    pub events: Vec<Event>,
}

impl std::fmt::Debug for Fetched {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Fetched")
            .field("candidate", &self.candidate)
            .field("score", &self.score)
            .field("bytes", &self.text.len())
            .field("events", &self.events.len())
            .finish()
    }
}

/// What happened to one downloaded candidate.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DownloadReport {
    /// 0 is the best ranked.
    pub rank: usize,
    pub score: i32,
    pub hash_match: bool,
    pub bytes: usize,
    pub result: Result<SrtReport, SourceError>,
}

/// Counts for one title, safe to log.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct CollectReport {
    pub listed: usize,
    pub in_language: usize,
    /// Dropped because they cover only part of the film (`CD1`, `2CD`).
    pub multi_part: usize,
    pub hash_matches: usize,
    pub has_filename: bool,
    pub list_ms: u128,
    pub downloads: Vec<DownloadReport>,
    pub download_ms: u128,
}

#[derive(Debug, PartialEq, Eq)]
pub enum CollectOutcome {
    Collected {
        report: CollectReport,
        subtitles: Vec<Fetched>,
    },
    Failed {
        elapsed_ms: u128,
        error: SourceError,
    },
}

/// Candidates in `languages` for this title, best first: hash matches, then
/// release-name score; ties keep the provider's order.
pub fn rank_candidates(
    candidates: Vec<Candidate>,
    languages: &[String],
    hints: &PlaybackHints,
    report: &mut CollectReport,
) -> Vec<(i32, Candidate)> {
    let file = Release::parse(hints.filename.as_deref().unwrap_or_default(), None, None);
    report.listed = candidates.len();
    report.has_filename = hints.filename.is_some();
    let mut ranked: Vec<(i32, Candidate)> = candidates
        .into_iter()
        .filter(|c| languages.iter().any(|l| l.eq_ignore_ascii_case(&c.lang)))
        .inspect(|_| report.in_language += 1)
        .filter(|c| {
            let whole = !c.release.is_multi_part();
            report.multi_part += usize::from(!whole);
            whole
        })
        .map(|c| (rank::score(&file, &c.release), c))
        .collect();
    report.hash_matches = ranked.iter().filter(|(_, c)| c.hash_match).count();
    ranked.sort_by(|(sa, a), (sb, b)| b.hash_match.cmp(&a.hash_match).then(sb.cmp(sa)));
    ranked
}

/// Lists, ranks and downloads up to `limit` candidates, one after another.
pub async fn collect(
    source: &dyn SubtitleSource,
    kind: &str,
    id: &str,
    hints: &PlaybackHints,
    languages: &[String],
    limit: usize,
) -> CollectOutcome {
    let started = Instant::now();
    let candidates = match source.list(kind, id, hints).await {
        Ok(candidates) => candidates,
        Err(error) => {
            return CollectOutcome::Failed {
                elapsed_ms: started.elapsed().as_millis(),
                error,
            };
        }
    };
    let mut report = CollectReport {
        list_ms: started.elapsed().as_millis(),
        ..CollectReport::default()
    };
    let ranked = rank_candidates(candidates, languages, hints, &mut report);

    let started = Instant::now();
    let mut subtitles = Vec::new();
    for (rank, (score, candidate)) in ranked.into_iter().take(limit).enumerate() {
        let body = source.download(&candidate).await;
        let bytes = body.as_ref().map_or(0, Vec::len);
        let parsed = body.and_then(|body| String::from_utf8(body).map_err(|_| SourceError::Decode));
        let result = match parsed {
            Ok(text) => {
                let (srt, events) = srt::parse(&text);
                if !events.is_empty() {
                    subtitles.push(Fetched {
                        candidate: candidate.clone(),
                        score,
                        text,
                        events,
                    });
                }
                Ok(srt)
            }
            Err(error) => Err(error),
        };
        report.downloads.push(DownloadReport {
            rank,
            score,
            hash_match: candidate.hash_match,
            bytes,
            result,
        });
    }
    report.download_ms = started.elapsed().as_millis();
    CollectOutcome::Collected { report, subtitles }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::sync::Mutex;

    fn tur() -> Vec<String> {
        vec!["tur".to_owned()]
    }

    fn hints(filename: Option<&str>) -> PlaybackHints {
        PlaybackHints {
            filename: filename.map(str::to_owned),
            ..PlaybackHints::default()
        }
    }

    fn sample() -> Value {
        json!({ "subtitles": [
            { "id": "1", "url": "https://subs.test/1", "lang": "tur", "m": "i",
              "subtitleFileName": "The.Movie.1994.DVDRip.XviD-OLD.srt", "fpsMilli": 25000 },
            { "id": "2", "url": "https://subs.test/2", "lang": "eng", "m": "i",
              "subtitleFileName": "The.Movie.1994.1080p.BluRay.x264-AMIABLE.srt" },
            { "id": "3", "url": "https://subs.test/3", "lang": "tur", "m": "i",
              "subtitleFileName": "The.Movie.1994.1080p.BluRay.x264-AMIABLE.srt", "fpsMilli": "23976" },
            { "id": "4", "url": "https://subs.test/4", "lang": "tur", "m": "i",
              "subtitleFileName": "The_Movie_DVDrip-=Grp=-(CD1).srt" },
            { "id": "5", "url": "https://subs.test/5", "lang": "tur", "m": "h",
              "subtitleFileName": "whatever.srt" },
            { "id": "6", "lang": "tur", "subtitleFileName": "no-url.srt" },
            { "id": "7", "url": "https://subs.test/7", "subtitleFileName": "no-lang.srt" }
        ]})
    }

    fn candidates(value: &Value) -> Vec<Candidate> {
        parse_v3_list(value.to_string().as_bytes()).unwrap()
    }

    #[test]
    fn v3_list_keeps_entries_with_url_and_lang() {
        let list = candidates(&sample());
        assert_eq!(list.len(), 5);
        assert_eq!(list[0].fps_milli, Some(25_000));
        assert_eq!(list[2].fps_milli, Some(23_976));
        assert!(list[4].hash_match);
        assert_eq!(parse_v3_list(b"<html>").err(), Some(SourceError::Decode));
        assert!(parse_v3_list(b"{}").unwrap().is_empty());
    }

    #[test]
    fn ranking_filters_language_and_parts_and_puts_hash_first() {
        let mut report = CollectReport::default();
        let ranked = rank_candidates(
            candidates(&sample()),
            &tur(),
            &hints(Some("The.Movie.1994.2160p.BluRay.x265-AMIABLE.mkv")),
            &mut report,
        );
        let urls: Vec<&str> = ranked.iter().map(|(_, c)| c.url.as_str()).collect();
        assert_eq!(
            urls,
            [
                "https://subs.test/5",
                "https://subs.test/3",
                "https://subs.test/1"
            ]
        );
        assert_eq!(report.listed, 5);
        assert_eq!(report.in_language, 4);
        assert_eq!(report.multi_part, 1);
        assert_eq!(report.hash_matches, 1);
        assert!(report.has_filename);
    }

    #[test]
    fn without_filename_provider_order_is_kept() {
        let mut report = CollectReport::default();
        let list = candidates(&json!({ "subtitles": [
            { "url": "https://s.test/a", "lang": "tur", "subtitleFileName": "A.BluRay-X.srt" },
            { "url": "https://s.test/b", "lang": "TUR", "subtitleFileName": "B.WEB-DL-Y.srt" }
        ]}));
        let ranked = rank_candidates(list, &tur(), &hints(None), &mut report);
        let urls: Vec<&str> = ranked.iter().map(|(_, c)| c.url.as_str()).collect();
        assert_eq!(urls, ["https://s.test/a", "https://s.test/b"]);
        assert!(!report.has_filename);
    }

    #[test]
    fn ids_are_checked_before_reaching_a_url() {
        for ok in ["tt0111161", "tt0944947:1:2"] {
            assert!(valid_id(ok), "{ok}");
        }
        for bad in [
            "",
            "tt",
            "kitsu:1",
            "tt1:2",
            "tt1:a:b",
            "tt1/../x",
            "tt1:1:2:3",
            "tt1?x=1",
        ] {
            assert!(!valid_id(bad), "{bad}");
        }
    }

    #[test]
    fn list_url_carries_hash_and_size_not_filename() {
        let source = OpenSubtitlesV3::new().unwrap();
        let plain = source
            .list_url("series", "tt0944947:1:2", &hints(None))
            .unwrap();
        assert_eq!(
            plain.as_str(),
            "https://opensubtitles-v3.strem.io/subtitles/series/tt0944947:1:2.json"
        );
        let full = PlaybackHints {
            filename: Some("Secret.Title.mkv".to_owned()),
            video_size: Some(42),
            video_hash: Some("8e245d9679d31e12".to_owned()),
        };
        let url = source.list_url("movie", "tt1", &full).unwrap();
        assert_eq!(
            url.as_str(),
            "https://opensubtitles-v3.strem.io/subtitles/movie/tt1/videoHash=8e245d9679d31e12&videoSize=42.json"
        );
        assert!(source.list_url("movie", "tt1/../x", &full).is_none());
        assert!(source.list_url("tv", "tt1", &full).is_none());
    }

    struct FakeSource {
        list: Result<Value, SourceError>,
        bodies: Vec<(&'static str, Result<Vec<u8>, SourceError>)>,
        downloaded: Mutex<Vec<String>>,
    }

    impl SubtitleSource for FakeSource {
        fn list<'a>(
            &'a self,
            _kind: &'a str,
            _id: &'a str,
            _hints: &'a PlaybackHints,
        ) -> ListFuture<'a> {
            let result = self.list.clone().map(|v| candidates(&v));
            Box::pin(async move { result })
        }

        fn download<'a>(&'a self, candidate: &'a Candidate) -> BodyFuture<'a> {
            self.downloaded.lock().unwrap().push(candidate.url.clone());
            let body = self
                .bodies
                .iter()
                .find(|(url, _)| *url == candidate.url)
                .map_or(Err(SourceError::Status(404)), |(_, body)| body.clone());
            Box::pin(async move { body })
        }
    }

    const SRT: &str = "1\r\n00:00:01,000 --> 00:00:02,000\r\nSecret line\r\n";

    #[tokio::test]
    async fn collect_downloads_best_ranked_up_to_limit() {
        let source = FakeSource {
            list: Ok(sample()),
            bodies: vec![
                ("https://subs.test/5", Ok(SRT.as_bytes().to_vec())),
                ("https://subs.test/3", Ok(vec![0xff, 0xfe])),
                ("https://subs.test/1", Ok(SRT.as_bytes().to_vec())),
            ],
            downloaded: Mutex::new(Vec::new()),
        };
        let file = hints(Some("The.Movie.1994.1080p.BluRay.x264-AMIABLE.mkv"));
        let outcome = collect(&source, "movie", "tt1", &file, &tur(), 2).await;
        let CollectOutcome::Collected { report, subtitles } = outcome else {
            panic!("list failed");
        };
        assert_eq!(
            *source.downloaded.lock().unwrap(),
            ["https://subs.test/5", "https://subs.test/3"]
        );
        assert_eq!(report.downloads.len(), 2);
        assert_eq!(
            report.downloads[0].result,
            Ok(SrtReport {
                timing_lines: 1,
                invalid: 0,
                events: 1
            })
        );
        assert_eq!(report.downloads[1].result, Err(SourceError::Decode));
        assert_eq!(subtitles.len(), 1);
        assert_eq!(subtitles[0].events.len(), 1);
    }

    #[tokio::test]
    async fn collect_reports_list_failure() {
        let source = FakeSource {
            list: Err(SourceError::Status(503)),
            bodies: Vec::new(),
            downloaded: Mutex::new(Vec::new()),
        };
        let outcome = collect(&source, "movie", "tt1", &hints(None), &tur(), 3).await;
        assert!(matches!(
            outcome,
            CollectOutcome::Failed {
                error: SourceError::Status(503),
                ..
            }
        ));
    }

    #[tokio::test]
    async fn debug_hides_urls_names_and_text() {
        let source = FakeSource {
            list: Ok(json!({ "subtitles": [
                { "url": "https://secret-host.test/x", "lang": "tur",
                  "subtitleFileName": "Secret.Title.2020.srt" }
            ]})),
            bodies: vec![("https://secret-host.test/x", Ok(SRT.as_bytes().to_vec()))],
            downloaded: Mutex::new(Vec::new()),
        };
        let outcome = collect(&source, "movie", "tt1", &hints(None), &tur(), 3).await;
        let printed = format!("{outcome:?}");
        assert!(!printed.contains("Secret"), "{printed}");
        assert!(!printed.contains("secret-host"), "{printed}");
        assert!(printed.contains("events: 1"), "{printed}");
    }
}
