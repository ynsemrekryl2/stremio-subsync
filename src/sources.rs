//! Candidate subtitles (M3): OpenSubtitles v3 (Stremio's keyless add-on)
//! and The Subtitle Database (keyless), SubDL, AltyaziDB, SubSource and
//! Wyzie (free keys) for one title and language, and one candidate's bytes.
//!
//! IMDb ids, release names, download links and the source keys never reach
//! the log: `Debug` on these types prints only kinds, counts and flags.

use reqwest::Url;
use serde::{Deserialize, Deserializer};
use serde_json::Value;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::formats;
use crate::rank;
use crate::srt::{self, Cue};

const FETCH_TIMEOUT: Duration = Duration::from_secs(15);
/// Largest list, subtitle file or archive read from a source.
const MAX_DOWNLOAD: usize = 4 * 1024 * 1024;
/// Candidates the probe downloads.
const PROBE_DOWNLOADS: usize = 3;
/// Candidates downloaded once more when none of the first fits (SS-040).
const MORE_DOWNLOADS: usize = 3;
/// Two downloads with the same number of lines, each starting and ending
/// this close to its pair, are copies of one timing (SS-071).
const SAME_TIMES_MS: u64 = 50;

const OPENSUBTITLES_V3: &str = "https://opensubtitles-v3.strem.io/";
const SUBDL_API: &str = "https://api.subdl.com/api/v1/subtitles";
const SUBDL_DOWNLOAD: &str = "https://dl.subdl.com";
const ALTYAZIDB_SEARCH: &str = "https://altyazidb.com/api/v1/search";
const ALTYAZIDB_SUBTITLE: &str = "https://altyazidb.com/api/v1/subtitle";
const SUBSOURCE_API: &str = "https://api.subsource.net/api/v1";
const WYZIE_SEARCH: &str = "https://sub.wyzie.io/search";
const SUBTITLEDB_API: &str = "https://api.thesubtitledb.org";
/// How long a source whose key was refused (401, 403) is not asked again.
const REFUSED_FOR: Duration = Duration::from_secs(30 * 60);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Movie,
    Series,
}

/// One title, episode and language to find subtitles for.
#[derive(Clone, PartialEq, Eq)]
pub struct Query {
    pub kind: Kind,
    pub imdb: String,
    pub season: Option<u32>,
    pub episode: Option<u32>,
    /// ISO 639-2, as OpenSubtitles names languages (`tur`, `eng`, `pob`).
    pub lang: String,
}

impl std::fmt::Debug for Query {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Query")
            .field("kind", &self.kind)
            .field("has_episode", &self.episode.is_some())
            .field("lang", &self.lang)
            .finish()
    }
}

impl Query {
    /// From the route's `kind` and `id`: `tt…` for a movie, `tt…:season:episode`
    /// for a series.
    pub fn new(kind: &str, id: &str, lang: &str) -> Option<Self> {
        let mut parts = id.split(':');
        let imdb = parts.next()?;
        let digits = imdb.strip_prefix("tt")?;
        if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
            return None;
        }
        let kind = match kind {
            "movie" => Kind::Movie,
            "series" => Kind::Series,
            _ => return None,
        };
        let (season, episode) = match (kind, parts.next(), parts.next(), parts.next()) {
            (Kind::Movie, None, None, None) => (None, None),
            (Kind::Series, Some(s), Some(e), None) => {
                (Some(s.parse().ok()?), Some(e.parse().ok()?))
            }
            _ => return None,
        };
        Some(Self {
            kind,
            imdb: imdb.to_owned(),
            season,
            episode,
            lang: lang.to_owned(),
        })
    }

    fn stremio_id(&self) -> String {
        match (self.season, self.episode) {
            (Some(s), Some(e)) => format!("{}:{s}:{e}", self.imdb),
            _ => self.imdb.clone(),
        }
    }

    /// False when the source says the subtitle is for another episode.
    fn fits(&self, season: Option<u32>, episode: Option<u32>) -> bool {
        let same = |asked: Option<u32>, given: Option<u32>| match (asked, given) {
            (Some(a), Some(g)) if g > 0 => a == g,
            _ => true,
        };
        same(self.season, season) && same(self.episode, episode)
    }

    /// False when release names mark episodes and none is the asked one;
    /// names with no mark (season packs) fit.
    fn names_fit<'n>(&self, names: impl IntoIterator<Item = &'n str>) -> bool {
        let (Some(season), Some(episode)) = (self.season, self.episode) else {
            return true;
        };
        let marks: Vec<_> = names.into_iter().flat_map(episode_marks).collect();
        marks.is_empty() || marks.contains(&(season, episode))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, serde::Serialize)]
pub enum SourceKind {
    OpenSubtitles,
    SubDl,
    AltyaziDb,
    SubSource,
    Wyzie,
    SubtitleDb,
}

/// One subtitle a source offers. `Debug` shows no name and no link.
#[derive(Clone, PartialEq, Eq)]
pub struct Candidate {
    pub source: SourceKind,
    /// Release the subtitle was made for, as the source names it.
    pub release: String,
    pub fps_milli: Option<u32>,
    pub hearing_impaired: Option<bool>,
    link: String,
}

#[cfg(test)]
impl Candidate {
    /// A candidate for fake sources in other modules' tests.
    pub fn fake(source: SourceKind, release: &str) -> Self {
        Self {
            source,
            release: release.to_owned(),
            fps_milli: None,
            hearing_impaired: None,
            link: String::new(),
        }
    }
}

impl std::fmt::Debug for Candidate {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Candidate")
            .field("source", &self.source)
            .field("has_release", &!self.release.is_empty())
            .field("fps_milli", &self.fps_milli)
            .field("hearing_impaired", &self.hearing_impaired)
            .finish()
    }
}

/// Why a source gave nothing. Carries no URL.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SourceError {
    Timeout,
    Connect,
    Status(u16),
    Decode,
    TooLarge,
    /// The archive has no SRT for this episode, or several with no way to choose.
    NoSubtitle,
    Other,
}

pub type SearchFuture<'a> =
    Pin<Box<dyn Future<Output = Result<Vec<Candidate>, SourceError>> + Send + 'a>>;
pub type DownloadFuture<'a> =
    Pin<Box<dyn Future<Output = Result<Vec<u8>, SourceError>> + Send + 'a>>;

/// A subtitle site; faked in tests.
pub trait SubtitleSource: Send + Sync {
    fn kind(&self) -> SourceKind;
    fn search<'a>(&'a self, query: &'a Query) -> SearchFuture<'a>;
    /// Bytes of `candidate` as the site gives them: a subtitle file or an
    /// archive, which the probe opens (see [`crate::unpack`]).
    fn download<'a>(&'a self, query: &'a Query, candidate: &'a Candidate) -> DownloadFuture<'a>;
}

/// Accepts a number or a numeric string; anything else is absent.
fn lenient_u32<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Option<u32>, D::Error> {
    Ok(match Value::deserialize(deserializer)? {
        Value::Number(n) => n.as_u64().and_then(|n| u32::try_from(n).ok()),
        Value::String(s) => s.trim().parse().ok(),
        _ => None,
    })
}

fn word_count(name: &str) -> usize {
    name.split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .count()
}

#[derive(Deserialize)]
struct OsList {
    #[serde(default)]
    subtitles: Vec<OsEntry>,
}

#[derive(Deserialize)]
struct OsEntry {
    url: Option<String>,
    lang: Option<String>,
    #[serde(rename = "subtitleFileName")]
    file_name: Option<String>,
    #[serde(rename = "movieReleaseName")]
    release_name: Option<String>,
    #[serde(rename = "fpsMilli", default, deserialize_with = "lenient_u32")]
    fps_milli: Option<u32>,
    #[serde(default, deserialize_with = "lenient_u32")]
    season: Option<u32>,
    #[serde(default, deserialize_with = "lenient_u32")]
    episode: Option<u32>,
}

/// OpenSubtitles v3 list, kept to `query`'s language and episode.
pub fn parse_opensubtitles(body: &[u8], query: &Query) -> Result<Vec<Candidate>, SourceError> {
    let list: OsList = serde_json::from_slice(body).map_err(|_| SourceError::Decode)?;
    Ok(list
        .subtitles
        .into_iter()
        .filter(|e| e.lang.as_deref() == Some(query.lang.as_str()))
        .filter(|e| query.fits(e.season, e.episode))
        .filter_map(|e| {
            let link = e.url.filter(|u| u.starts_with("https://"))?;
            // The file name usually carries the full release; the release
            // field is sometimes just the title.
            let file = e.file_name.map(|n| rank::strip_extension(&n).to_owned());
            let release = [file, e.release_name]
                .into_iter()
                .flatten()
                .max_by_key(|n| word_count(n))
                .unwrap_or_default();
            Some(Candidate {
                source: SourceKind::OpenSubtitles,
                release,
                fps_milli: e.fps_milli.filter(|&f| f > 0),
                hearing_impaired: None,
                link,
            })
        })
        .collect())
}

#[derive(Deserialize)]
struct SdList {
    #[serde(default)]
    subtitles: Vec<SdEntry>,
}

#[derive(Deserialize)]
struct SdEntry {
    release_name: Option<String>,
    name: Option<String>,
    url: Option<String>,
    #[serde(default, deserialize_with = "lenient_u32")]
    season: Option<u32>,
    #[serde(default, deserialize_with = "lenient_u32")]
    episode: Option<u32>,
    hi: Option<bool>,
}

/// SubDL list (already filtered by language on the server), kept to the episode.
pub fn parse_subdl(body: &[u8], query: &Query) -> Result<Vec<Candidate>, SourceError> {
    let list: SdList = serde_json::from_slice(body).map_err(|_| SourceError::Decode)?;
    Ok(list
        .subtitles
        .into_iter()
        .filter(|e| query.fits(e.season, e.episode))
        .filter_map(|e| {
            let path = e.url.filter(|u| u.starts_with('/'))?;
            Some(Candidate {
                source: SourceKind::SubDl,
                release: e.release_name.or(e.name).unwrap_or_default(),
                fps_milli: None,
                hearing_impaired: e.hi,
                link: format!("{SUBDL_DOWNLOAD}{path}"),
            })
        })
        .collect())
}

/// SubDL's language code for an ISO 639-2 one.
pub fn subdl_language(lang: &str) -> Option<&'static str> {
    Some(match lang {
        "tur" => "TR",
        "eng" => "EN",
        "ger" | "deu" => "DE",
        "fre" | "fra" => "FR",
        "spa" => "ES",
        "ita" => "IT",
        "por" => "PT",
        "pob" => "BR_PT",
        "rus" => "RU",
        "ara" => "AR",
        "dut" | "nld" => "NL",
        "pol" => "PL",
        "ell" | "gre" => "EL",
        "swe" => "SV",
        "dan" => "DA",
        "fin" => "FI",
        "nor" => "NO",
        "cze" | "ces" => "CS",
        "hun" => "HU",
        "rum" | "ron" => "RO",
        "bul" => "BG",
        "hrv" => "HR",
        "srp" => "SR",
        "heb" => "HE",
        "per" | "fas" => "FA",
        "jpn" => "JA",
        "kor" => "KO",
        "chi" | "zho" => "ZH",
        "ind" => "ID",
        "vie" => "VI",
        "ukr" => "UK",
        _ => return None,
    })
}

#[derive(Deserialize)]
struct AdbList {
    #[serde(default, alias = "subtitles", alias = "results")]
    data: Vec<AdbEntry>,
}

#[derive(Deserialize)]
struct AdbEntry {
    id: Value,
    language: Option<String>,
    #[serde(default, deserialize_with = "lenient_u32")]
    season: Option<u32>,
    /// A number, or `PAKET` for a season pack (read as absent).
    #[serde(default, deserialize_with = "lenient_u32")]
    episode: Option<u32>,
    fps: Option<String>,
    #[serde(default)]
    versions: Vec<AdbVersion>,
    #[serde(default)]
    releases: Vec<String>,
    #[serde(default, deserialize_with = "lenient_u32")]
    hearing_impaired: Option<u32>,
    #[serde(default, deserialize_with = "lenient_u32")]
    forced: Option<u32>,
    /// 1 for a machine translation.
    #[serde(default, deserialize_with = "lenient_u32")]
    ai_ceviri: Option<u32>,
}

#[derive(Deserialize)]
struct AdbVersion {
    file: Option<String>,
}

/// AltyaziDB list (filtered by language and episode on the server), kept to
/// the language and episode again. Forced-only subtitles and machine
/// translations are left out: our own translation of a fitting English
/// subtitle reads better (SS-052).
pub fn parse_altyazidb(body: &[u8], query: &Query) -> Result<Vec<Candidate>, SourceError> {
    let list: AdbList = serde_json::from_slice(body).map_err(|_| SourceError::Decode)?;
    let lang = altyazidb_language(&query.lang);
    Ok(list
        .data
        .into_iter()
        .filter(|e| lang.is_some() && e.language.as_deref() == lang)
        .filter(|e| query.fits(e.season, e.episode))
        .filter(|e| e.forced != Some(1))
        .filter(|e| e.ai_ceviri != Some(1))
        .filter_map(|e| {
            let id = match e.id {
                Value::Number(n) => n.as_u64()?.to_string(),
                Value::String(s) if !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit()) => s,
                _ => return None,
            };
            // A subtitle lists every release it fits; the fullest name ranks best.
            let release = e
                .versions
                .into_iter()
                .filter_map(|v| v.file)
                .chain(e.releases)
                .max_by_key(|n| word_count(n))
                .unwrap_or_default();
            let fps_milli = e
                .fps
                .and_then(|f| f.trim().parse::<f64>().ok())
                .filter(|f| *f > 0.0 && *f < 1000.0)
                .map(|f| (f * 1000.0).round() as u32);
            Some(Candidate {
                source: SourceKind::AltyaziDb,
                release,
                fps_milli,
                hearing_impaired: e.hearing_impaired.map(|h| h == 1),
                link: format!("{ALTYAZIDB_SUBTITLE}?sub_id={id}"),
            })
        })
        .collect())
}

/// AltyaziDB's language code for an ISO 639-2 one; it has only these two.
pub fn altyazidb_language(lang: &str) -> Option<&'static str> {
    match lang {
        "tur" => Some("tr"),
        "eng" => Some("en"),
        _ => None,
    }
}

#[derive(Deserialize)]
struct SsTitles {
    #[serde(default)]
    data: Vec<SsTitle>,
}

#[derive(Deserialize)]
struct SsTitle {
    #[serde(rename = "movieId")]
    movie_id: Value,
    #[serde(rename = "imdbId")]
    imdb_id: Option<String>,
    #[serde(default, deserialize_with = "lenient_u32")]
    season: Option<u32>,
}

/// SubSource's own id for `query`'s title: a series has one per season.
pub fn parse_subsource_title(body: &[u8], query: &Query) -> Result<Option<u64>, SourceError> {
    let list: SsTitles = serde_json::from_slice(body).map_err(|_| SourceError::Decode)?;
    Ok(list
        .data
        .into_iter()
        .filter(|t| t.imdb_id.as_deref().is_none_or(|id| id == query.imdb))
        .filter(|t| match query.kind {
            Kind::Movie => true,
            Kind::Series => query.season.is_none() || t.season == query.season,
        })
        .find_map(|t| t.movie_id.as_u64()))
}

#[derive(Deserialize)]
struct SsList {
    #[serde(default)]
    data: Vec<SsEntry>,
}

#[derive(Deserialize)]
struct SsEntry {
    #[serde(rename = "subtitleId")]
    subtitle_id: Value,
    language: Option<String>,
    #[serde(rename = "releaseInfo", default)]
    release_info: Vec<String>,
    #[serde(rename = "hearingImpaired")]
    hearing_impaired: Option<bool>,
    #[serde(rename = "foreignParts")]
    foreign_parts: Option<bool>,
    framerate: Option<String>,
}

/// SubSource list for one title (a series season), kept to the language and,
/// when a release names episodes, to `query`'s; foreign-parts-only subtitles
/// are left out. Releases with no episode mark (season packs) stay.
pub fn parse_subsource(body: &[u8], query: &Query) -> Result<Vec<Candidate>, SourceError> {
    let list: SsList = serde_json::from_slice(body).map_err(|_| SourceError::Decode)?;
    let lang = subsource_language(&query.lang);
    Ok(list
        .data
        .into_iter()
        .filter(|e| {
            lang.is_some_and(|l| {
                e.language
                    .as_deref()
                    .is_some_and(|g| g.eq_ignore_ascii_case(l))
            })
        })
        .filter(|e| e.foreign_parts != Some(true))
        .filter(|e| query.names_fit(e.release_info.iter().map(String::as_str)))
        .filter_map(|e| {
            let id = e.subtitle_id.as_u64()?;
            let release = e
                .release_info
                .into_iter()
                .max_by_key(|n| word_count(n))
                .unwrap_or_default();
            let fps_milli = e
                .framerate
                .and_then(|f| f.trim().parse::<f64>().ok())
                .filter(|f| *f > 0.0 && *f < 1000.0)
                .map(|f| (f * 1000.0).round() as u32);
            Some(Candidate {
                source: SourceKind::SubSource,
                release,
                fps_milli,
                hearing_impaired: e.hearing_impaired,
                link: format!("{SUBSOURCE_API}/subtitles/{id}/download"),
            })
        })
        .collect())
}

/// SubSource's language name for an ISO 639-2 code.
pub fn subsource_language(lang: &str) -> Option<&'static str> {
    Some(match lang {
        "tur" => "turkish",
        "eng" => "english",
        "ger" | "deu" => "german",
        "fre" | "fra" => "french",
        "spa" => "spanish",
        "ita" => "italian",
        "por" => "portuguese",
        "rus" => "russian",
        "ara" => "arabic",
        "dut" | "nld" => "dutch",
        "pol" => "polish",
        "ell" | "gre" => "greek",
        "swe" => "swedish",
        "dan" => "danish",
        "fin" => "finnish",
        "nor" => "norwegian",
        "cze" | "ces" => "czech",
        "hun" => "hungarian",
        "rum" | "ron" => "romanian",
        "bul" => "bulgarian",
        "hrv" => "croatian",
        "srp" => "serbian",
        "heb" => "hebrew",
        "jpn" => "japanese",
        "kor" => "korean",
        "ind" => "indonesian",
        "vie" => "vietnamese",
        "ukr" => "ukrainian",
        _ => return None,
    })
}

#[derive(Deserialize)]
struct WzEntry {
    url: Option<String>,
    language: Option<String>,
    format: Option<String>,
    release: Option<String>,
    releases: Option<Vec<String>>,
    #[serde(rename = "fileName")]
    file_name: Option<String>,
    #[serde(rename = "isHearingImpaired")]
    hearing_impaired: Option<bool>,
    /// True for a machine translation.
    ai: Option<bool>,
}

/// Wyzie list (filtered by language, format and episode on the server), kept
/// to the language, SRT and, when a release names episodes, `query`'s.
/// Machine translations are left out, as on AltyaziDB (SS-052).
pub fn parse_wyzie(body: &[u8], query: &Query) -> Result<Vec<Candidate>, SourceError> {
    let list: Vec<WzEntry> = serde_json::from_slice(body).map_err(|_| SourceError::Decode)?;
    let lang = wyzie_language(&query.lang);
    Ok(list
        .into_iter()
        .filter(|e| lang.is_some() && e.language.as_deref() == lang.as_deref())
        .filter(|e| {
            e.format
                .as_deref()
                .is_none_or(|f| f.eq_ignore_ascii_case("srt"))
        })
        .filter(|e| e.ai != Some(true))
        .filter_map(|e| {
            let link = e.url.filter(|u| u.starts_with("https://"))?;
            let file = e.file_name.map(|n| rank::strip_extension(&n).to_owned());
            let names: Vec<String> = [e.release, file]
                .into_iter()
                .flatten()
                .chain(e.releases.unwrap_or_default())
                .collect();
            if !query.names_fit(names.iter().map(String::as_str)) {
                return None;
            }
            Some(Candidate {
                source: SourceKind::Wyzie,
                release: names
                    .into_iter()
                    .max_by_key(|n| word_count(n))
                    .unwrap_or_default(),
                fps_milli: None,
                hearing_impaired: e.hearing_impaired,
                link,
            })
        })
        .collect())
}

/// Wyzie's language code (ISO 639-1) for an ISO 639-2 one.
pub fn wyzie_language(lang: &str) -> Option<String> {
    subdl_language(lang)
        .filter(|c| c.len() == 2)
        .map(str::to_ascii_lowercase)
}

#[derive(Deserialize)]
struct TsdbList {
    subtitles: Option<TsdbPage>,
}

#[derive(Deserialize)]
struct TsdbPage {
    #[serde(default)]
    items: Vec<TsdbEntry>,
}

#[derive(Deserialize)]
struct TsdbEntry {
    #[serde(default)]
    id: Value,
    language: Option<String>,
    format: Option<String>,
    release_name: Option<String>,
    hearing_impaired: Option<bool>,
    /// True for a machine transcription.
    ai_transcribed: Option<bool>,
    #[serde(default)]
    fps: Value,
}

/// Subtitle formats the probe reads (see [`crate::formats`]).
fn readable_format(format: Option<&str>) -> bool {
    format.is_none_or(|f| ["srt", "ass", "ssa", "vtt"].contains(&f.to_ascii_lowercase().as_str()))
}

/// The Subtitle Database list (filtered by title, episode and language on
/// the server), kept to the language, readable formats and, when a release
/// names episodes, `query`'s. Machine transcriptions are left out.
pub fn parse_subtitledb(body: &[u8], query: &Query) -> Result<Vec<Candidate>, SourceError> {
    let list: TsdbList = serde_json::from_slice(body).map_err(|_| SourceError::Decode)?;
    let lang = subtitledb_language(&query.lang);
    Ok(list
        .subtitles
        .map(|page| page.items)
        .unwrap_or_default()
        .into_iter()
        .filter(|e| lang.is_some() && e.language.as_deref().map(str::to_ascii_lowercase) == lang)
        .filter(|e| readable_format(e.format.as_deref()))
        .filter(|e| e.ai_transcribed != Some(true))
        .filter_map(|e| {
            let id = match &e.id {
                Value::Number(n) => n.as_u64()?,
                Value::String(s) => s.trim().parse().ok()?,
                _ => return None,
            };
            let release = e.release_name.unwrap_or_default();
            if !query.names_fit([release.as_str()]) {
                return None;
            }
            let fps = match &e.fps {
                Value::Number(n) => n.as_f64(),
                Value::String(s) => s.trim().parse().ok(),
                _ => None,
            };
            Some(Candidate {
                source: SourceKind::SubtitleDb,
                release,
                fps_milli: fps
                    .filter(|f| *f > 1.0 && *f < 1000.0)
                    .map(|f| (f * 1000.0).round() as u32),
                hearing_impaired: e.hearing_impaired,
                link: format!("{SUBTITLEDB_API}/get/{id}"),
            })
        })
        .collect())
}

/// The Subtitle Database's language code (ISO 639-1, Brazilian `pb`).
pub fn subtitledb_language(lang: &str) -> Option<String> {
    match lang {
        "pob" => Some("pb".to_owned()),
        _ => wyzie_language(lang),
    }
}

/// `(season, episode)` pairs written in a file name: `S01E02`, `s1e2`, `1x02`.
pub fn episode_marks(name: &str) -> Vec<(u32, u32)> {
    let b = name.to_ascii_lowercase().into_bytes();
    let number = |from: usize| -> Option<(u32, usize)> {
        let len = b[from.min(b.len())..]
            .iter()
            .take_while(|c| c.is_ascii_digit())
            .count();
        if !(1..=3).contains(&len) {
            return None;
        }
        let value = std::str::from_utf8(&b[from..from + len])
            .ok()?
            .parse()
            .ok()?;
        Some((value, from + len))
    };
    let mut marks = Vec::new();
    for i in 0..b.len() {
        let after_word = i > 0 && b[i - 1].is_ascii_alphanumeric();
        if b[i] == b's'
            && let Some((season, j)) = number(i + 1)
            && b.get(j) == Some(&b'e')
            && let Some((episode, _)) = number(j + 1)
        {
            marks.push((season, episode));
        } else if b[i].is_ascii_digit()
            && !after_word
            && let Some((season, j)) = number(i)
            && b.get(j) == Some(&b'x')
            && let Some((episode, k)) = number(j + 1)
            && k - j > 2
        {
            marks.push((season, episode));
        }
    }
    marks
}

fn client() -> Option<reqwest::Client> {
    reqwest::Client::builder()
        .timeout(FETCH_TIMEOUT)
        .user_agent(concat!("stremio-subsync/", env!("CARGO_PKG_VERSION")))
        .build()
        .ok()
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

/// GET with a size cap; errors never carry the URL.
async fn get_capped(request: reqwest::RequestBuilder) -> Result<Vec<u8>, SourceError> {
    let mut response = request
        .send()
        .await
        .and_then(reqwest::Response::error_for_status)
        .map_err(|err| classify(&err))?;
    if response
        .content_length()
        .is_some_and(|n| n > MAX_DOWNLOAD as u64)
    {
        return Err(SourceError::TooLarge);
    }
    let mut body = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(|err| classify(&err))? {
        body.extend_from_slice(&chunk);
        if body.len() > MAX_DOWNLOAD {
            return Err(SourceError::TooLarge);
        }
    }
    Ok(body)
}

pub struct OpenSubtitlesV3 {
    base: Url,
    client: reqwest::Client,
}

impl OpenSubtitlesV3 {
    pub fn new() -> Option<Self> {
        Some(Self {
            base: Url::parse(OPENSUBTITLES_V3).ok()?,
            client: client()?,
        })
    }
}

impl SubtitleSource for OpenSubtitlesV3 {
    fn kind(&self) -> SourceKind {
        SourceKind::OpenSubtitles
    }

    fn search<'a>(&'a self, query: &'a Query) -> SearchFuture<'a> {
        Box::pin(async move {
            let kind = match query.kind {
                Kind::Movie => "movie",
                Kind::Series => "series",
            };
            let url = self
                .base
                .join(&format!("subtitles/{kind}/{}.json", query.stremio_id()))
                .map_err(|_| SourceError::Other)?;
            parse_opensubtitles(&get_capped(self.client.get(url)).await?, query)
        })
    }

    fn download<'a>(&'a self, _query: &'a Query, candidate: &'a Candidate) -> DownloadFuture<'a> {
        Box::pin(async move {
            let url = Url::parse(&candidate.link).map_err(|_| SourceError::Other)?;
            get_capped(self.client.get(url)).await
        })
    }
}

/// SubDL with the user's key. No `Debug`: the key is part of every search URL.
pub struct SubDl {
    key: String,
    client: reqwest::Client,
}

impl SubDl {
    pub fn new(key: String) -> Option<Self> {
        Some(Self {
            key,
            client: client()?,
        })
    }
}

impl SubtitleSource for SubDl {
    fn kind(&self) -> SourceKind {
        SourceKind::SubDl
    }

    fn search<'a>(&'a self, query: &'a Query) -> SearchFuture<'a> {
        Box::pin(async move {
            let Some(language) = subdl_language(&query.lang) else {
                return Ok(Vec::new());
            };
            let kind = match query.kind {
                Kind::Movie => "movie",
                Kind::Series => "tv",
            };
            let mut params = vec![
                ("api_key", self.key.clone()),
                ("imdb_id", query.imdb.clone()),
                ("type", kind.to_owned()),
                ("languages", language.to_owned()),
                ("subs_per_page", "30".to_owned()),
            ];
            if let (Some(s), Some(e)) = (query.season, query.episode) {
                params.push(("season_number", s.to_string()));
                params.push(("episode_number", e.to_string()));
            }
            let url = Url::parse_with_params(SUBDL_API, &params).map_err(|_| SourceError::Other)?;
            parse_subdl(&get_capped(self.client.get(url)).await?, query)
        })
    }

    fn download<'a>(&'a self, _query: &'a Query, candidate: &'a Candidate) -> DownloadFuture<'a> {
        Box::pin(async move {
            let url = Url::parse(&candidate.link).map_err(|_| SourceError::Other)?;
            get_capped(self.client.get(url)).await
        })
    }
}

/// AltyaziDB with the user's key. No `Debug`: the key is in every request's
/// `X-API-Key` header.
pub struct AltyaziDb {
    key: String,
    client: reqwest::Client,
}

impl AltyaziDb {
    pub fn new(key: String) -> Option<Self> {
        Some(Self {
            key,
            client: client()?,
        })
    }

    fn get(&self, url: Url) -> reqwest::RequestBuilder {
        self.client.get(url).header("X-API-Key", &self.key)
    }
}

impl SubtitleSource for AltyaziDb {
    fn kind(&self) -> SourceKind {
        SourceKind::AltyaziDb
    }

    fn search<'a>(&'a self, query: &'a Query) -> SearchFuture<'a> {
        Box::pin(async move {
            let Some(language) = altyazidb_language(&query.lang) else {
                return Ok(Vec::new());
            };
            let mut params = vec![
                ("imdb_id", query.imdb.clone()),
                ("lang", language.to_owned()),
                ("limit", "30".to_owned()),
            ];
            if let (Some(s), Some(e)) = (query.season, query.episode) {
                params.push(("season", s.to_string()));
                params.push(("episode", e.to_string()));
            }
            let url = Url::parse_with_params(ALTYAZIDB_SEARCH, &params)
                .map_err(|_| SourceError::Other)?;
            parse_altyazidb(&get_capped(self.get(url)).await?, query)
        })
    }

    fn download<'a>(&'a self, query: &'a Query, candidate: &'a Candidate) -> DownloadFuture<'a> {
        Box::pin(async move {
            let mut url = Url::parse(&candidate.link).map_err(|_| SourceError::Other)?;
            // The server takes the episode out of a season pack.
            if let (Some(s), Some(e)) = (query.season, query.episode) {
                url.query_pairs_mut()
                    .append_pair("season", &s.to_string())
                    .append_pair("episode", &e.to_string());
            }
            let body = get_capped(self.get(url)).await?;
            // Errors come back as JSON; a subtitle never starts with `{`.
            if body.iter().find(|b| !b.is_ascii_whitespace()) == Some(&b'{') {
                return Err(SourceError::NoSubtitle);
            }
            Ok(body)
        })
    }
}

/// SubSource with the user's key. No `Debug`: the key is in every request's
/// `X-API-Key` header.
pub struct SubSource {
    key: String,
    client: reqwest::Client,
}

impl SubSource {
    pub fn new(key: String) -> Option<Self> {
        Some(Self {
            key,
            client: client()?,
        })
    }

    fn get(&self, url: Url) -> reqwest::RequestBuilder {
        self.client.get(url).header("X-API-Key", &self.key)
    }
}

impl SubtitleSource for SubSource {
    fn kind(&self) -> SourceKind {
        SourceKind::SubSource
    }

    fn search<'a>(&'a self, query: &'a Query) -> SearchFuture<'a> {
        Box::pin(async move {
            let Some(language) = subsource_language(&query.lang) else {
                return Ok(Vec::new());
            };
            // Two steps: the IMDb id gives SubSource's title (one per season),
            // the title gives its subtitles.
            let mut params = vec![
                ("searchType", "imdb".to_owned()),
                ("imdb", query.imdb.clone()),
            ];
            match (query.kind, query.season) {
                (Kind::Movie, _) => params.push(("type", "movie".to_owned())),
                (Kind::Series, season) => {
                    params.push(("type", "series".to_owned()));
                    if let Some(s) = season {
                        params.push(("season", s.to_string()));
                    }
                }
            }
            let url = Url::parse_with_params(&format!("{SUBSOURCE_API}/movies/search"), &params)
                .map_err(|_| SourceError::Other)?;
            let Some(title) = parse_subsource_title(&get_capped(self.get(url)).await?, query)?
            else {
                return Ok(Vec::new());
            };
            let params = [
                ("movieId", title.to_string()),
                ("language", language.to_owned()),
                ("limit", "100".to_owned()),
            ];
            let url = Url::parse_with_params(&format!("{SUBSOURCE_API}/subtitles"), &params)
                .map_err(|_| SourceError::Other)?;
            parse_subsource(&get_capped(self.get(url)).await?, query)
        })
    }

    fn download<'a>(&'a self, _query: &'a Query, candidate: &'a Candidate) -> DownloadFuture<'a> {
        Box::pin(async move {
            let url = Url::parse(&candidate.link).map_err(|_| SourceError::Other)?;
            get_capped(self.get(url)).await
        })
    }
}

/// Wyzie Subs with the user's key. No `Debug`: the key is part of every
/// search URL. Download links carry Wyzie's own token, not the key.
pub struct Wyzie {
    key: String,
    client: reqwest::Client,
}

impl Wyzie {
    pub fn new(key: String) -> Option<Self> {
        Some(Self {
            key,
            client: client()?,
        })
    }
}

impl SubtitleSource for Wyzie {
    fn kind(&self) -> SourceKind {
        SourceKind::Wyzie
    }

    fn search<'a>(&'a self, query: &'a Query) -> SearchFuture<'a> {
        Box::pin(async move {
            let Some(language) = wyzie_language(&query.lang) else {
                return Ok(Vec::new());
            };
            let mut params = vec![
                ("id", query.imdb.clone()),
                ("language", language),
                ("format", "srt".to_owned()),
                // Every source the key may use (charlie and lima on a free one).
                ("source", "all".to_owned()),
                ("limit", "50".to_owned()),
                ("key", self.key.clone()),
            ];
            if let (Some(s), Some(e)) = (query.season, query.episode) {
                params.push(("season", s.to_string()));
                params.push(("episode", e.to_string()));
            }
            let url =
                Url::parse_with_params(WYZIE_SEARCH, &params).map_err(|_| SourceError::Other)?;
            parse_wyzie(&get_capped(self.client.get(url)).await?, query)
        })
    }

    fn download<'a>(&'a self, _query: &'a Query, candidate: &'a Candidate) -> DownloadFuture<'a> {
        Box::pin(async move {
            let url = Url::parse(&candidate.link).map_err(|_| SourceError::Other)?;
            let body = get_capped(self.client.get(url)).await?;
            // Errors come back as JSON; a subtitle never starts with `{`.
            if body.iter().find(|b| !b.is_ascii_whitespace()) == Some(&b'{') {
                return Err(SourceError::NoSubtitle);
            }
            Ok(body)
        })
    }
}

/// The Subtitle Database: no key.
pub struct SubtitleDb {
    client: reqwest::Client,
}

impl SubtitleDb {
    pub fn new() -> Option<Self> {
        Some(Self { client: client()? })
    }
}

impl SubtitleSource for SubtitleDb {
    fn kind(&self) -> SourceKind {
        SourceKind::SubtitleDb
    }

    fn search<'a>(&'a self, query: &'a Query) -> SearchFuture<'a> {
        Box::pin(async move {
            let Some(language) = subtitledb_language(&query.lang) else {
                return Ok(Vec::new());
            };
            let mut path = format!("{SUBTITLEDB_API}/v1/by-imdb/{}", query.imdb);
            if let (Some(s), Some(e)) = (query.season, query.episode) {
                path.push_str(&format!("/season/{s}/episode/{e}"));
            }
            let params = [("lang", language), ("limit", "100".to_owned())];
            let url = Url::parse_with_params(&path, &params).map_err(|_| SourceError::Other)?;
            parse_subtitledb(&get_capped(self.client.get(url)).await?, query)
        })
    }

    fn download<'a>(&'a self, _query: &'a Query, candidate: &'a Candidate) -> DownloadFuture<'a> {
        Box::pin(async move {
            let url = Url::parse(&candidate.link).map_err(|_| SourceError::Other)?;
            let body = get_capped(self.client.get(url)).await?;
            if !looks_like_subtitle(&body) {
                return Err(SourceError::NoSubtitle);
            }
            Ok(body)
        })
    }
}

/// False for an empty body, a JSON error or an HTML page.
fn looks_like_subtitle(body: &[u8]) -> bool {
    let Some(start) = body.iter().position(|b| !b.is_ascii_whitespace()) else {
        return false;
    };
    let head: Vec<u8> = body[start..]
        .iter()
        .take(15)
        .map(u8::to_ascii_lowercase)
        .collect();
    !(head.starts_with(b"{") || head.starts_with(b"<!doctype html") || head.starts_with(b"<html"))
}

/// A keyed source that is not asked again for [`REFUSED_FOR`] once its key
/// is refused: every request would fail the same way and only cost time.
/// Saving a new key restarts the server, which forgets the refusal.
pub struct RefusalMemory {
    inner: Arc<dyn SubtitleSource>,
    refused: std::sync::Mutex<Option<(Instant, SourceError)>>,
}

impl RefusalMemory {
    pub fn wrap(inner: Arc<dyn SubtitleSource>) -> Arc<dyn SubtitleSource> {
        Arc::new(Self {
            inner,
            refused: std::sync::Mutex::new(None),
        })
    }

    fn remembered(&self) -> Option<SourceError> {
        let guard = self.refused.lock().unwrap_or_else(|e| e.into_inner());
        guard
            .filter(|(at, _)| at.elapsed() < REFUSED_FOR)
            .map(|(_, err)| err)
    }

    fn note<T>(&self, result: Result<T, SourceError>) -> Result<T, SourceError> {
        if let Err(err @ SourceError::Status(401 | 403)) = result {
            *self.refused.lock().unwrap_or_else(|e| e.into_inner()) = Some((Instant::now(), err));
        }
        result
    }
}

impl SubtitleSource for RefusalMemory {
    fn kind(&self) -> SourceKind {
        self.inner.kind()
    }

    fn search<'a>(&'a self, query: &'a Query) -> SearchFuture<'a> {
        Box::pin(async move {
            if let Some(err) = self.remembered() {
                return Err(err);
            }
            self.note(self.inner.search(query).await)
        })
    }

    fn download<'a>(&'a self, query: &'a Query, candidate: &'a Candidate) -> DownloadFuture<'a> {
        Box::pin(async move {
            if let Some(err) = self.remembered() {
                return Err(err);
            }
            self.note(self.inner.download(query, candidate).await)
        })
    }
}

/// Shape of one downloaded candidate. Safe to log.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SrtStats {
    pub source: SourceKind,
    pub score: i32,
    pub bytes: usize,
    pub cues: usize,
    pub first_ms: Option<u64>,
    pub last_end_ms: Option<u64>,
}

/// Result of one M3 probe. Safe to log: kinds, counts and scores only.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourcesProbe {
    pub elapsed_ms: u128,
    pub searches: Vec<(SourceKind, Result<usize, SourceError>)>,
    /// Best few ranking scores, highest first.
    pub top_scores: Vec<i32>,
    pub downloads: Vec<Result<SrtStats, SourceError>>,
    /// Downloads left out because an earlier one had the same lines (SS-055).
    pub duplicates: usize,
    /// Downloads with the same times as an earlier one but other text: tried,
    /// but they took no slot (SS-071).
    pub timing_copies: usize,
}

/// One M3 probe: the report and the cues of each download (`None` where it
/// failed), in the same order. Keeps the ranked candidates not downloaded
/// yet, with their links; no `Debug`, so they cannot reach the log.
pub struct Probe {
    pub report: SourcesProbe,
    pub cues: Vec<Option<Vec<Cue>>>,
    /// How many of the first round's downloads the rule before SS-071, when
    /// a timing copy took a slot, would have made too (they lead `cues`).
    /// The second round runs only when none of these fits, as it did then,
    /// so every candidate that rule tried is still tried.
    pub first_slots: usize,
    sources: Vec<Arc<dyn SubtitleSource>>,
    query: Query,
    filename: Option<String>,
    rest: Vec<(usize, Candidate)>,
}

impl Probe {
    /// Whether ranked candidates are left to download.
    pub fn has_more(&self) -> bool {
        !self.rest.is_empty()
    }

    /// Downloads the next few ranked candidates and appends them to the
    /// report and the cues (SS-040).
    pub async fn download_more(&mut self) {
        self.take(MORE_DOWNLOADS).await;
    }

    /// Downloads ranked candidates in order until `want` new ones are read
    /// or failed. One with the same lines (times and text) as an earlier
    /// download is left out and the next one taken in its place: sources
    /// share files, and a copy would only take a slot (SS-055). One with the
    /// same times but other text (the same translation uploaded elsewhere,
    /// re-encoded or touched up) is kept and tried, but the next one is
    /// taken too (SS-071). Fetches at most twice `want`, so a run of copies
    /// cannot spend the quota.
    ///
    /// Returns how many downloads the rule before SS-071 would have kept by
    /// the time it stopped: the same up to there, as it took the same
    /// candidates in the same order and counted a timing copy as a slot.
    async fn take(&mut self, want: usize) -> usize {
        let (mut taken, mut fetched) = (0, 0);
        // Slots as counted before SS-071, and where that rule stopped.
        let (mut slots, mut old_end) = (0, None);
        while taken < want && fetched < 2 * want && !self.rest.is_empty() {
            let (i, candidate) = self.rest.remove(0);
            fetched += 1;
            let read = match self.sources[i].download(&self.query, &candidate).await {
                Ok(bytes) => {
                    let size = bytes.len();
                    crate::unpack::subtitle(bytes, &self.query)
                        .map(|file| (size, formats::cues(&srt::decode(&file, &self.query.lang))))
                }
                Err(err) => Err(err),
            };
            match read {
                Err(err) => {
                    self.report.downloads.push(Err(err));
                    self.cues.push(None);
                    taken += 1;
                }
                Ok((_, cues))
                    if !cues.is_empty() && self.cues.iter().flatten().any(|seen| *seen == cues) =>
                {
                    self.report.duplicates += 1;
                    continue;
                }
                Ok((size, cues)) => {
                    if self
                        .cues
                        .iter()
                        .flatten()
                        .any(|seen| same_times(seen, &cues))
                    {
                        self.report.timing_copies += 1;
                    } else {
                        taken += 1;
                    }
                    self.report.downloads.push(Ok(SrtStats {
                        source: candidate.source,
                        score: rank::score(&candidate.release, self.filename.as_deref()),
                        bytes: size,
                        cues: cues.len(),
                        first_ms: cues.first().map(|c| c.start_ms),
                        last_end_ms: cues.iter().map(|c| c.end_ms).max(),
                    }));
                    self.cues.push(Some(cues));
                }
            }
            slots += 1;
            if slots == want {
                old_end.get_or_insert(self.cues.len());
            }
        }
        old_end.unwrap_or(self.cues.len())
    }
}

/// Whether two downloads carry one timing: as many lines, each starting and
/// ending within `SAME_TIMES_MS` of its pair (SS-071).
fn same_times(a: &[Cue], b: &[Cue]) -> bool {
    !a.is_empty()
        && a.len() == b.len()
        && a.iter().zip(b).all(|(x, y)| {
            x.start_ms.abs_diff(y.start_ms) <= SAME_TIMES_MS
                && x.end_ms.abs_diff(y.end_ms) <= SAME_TIMES_MS
        })
}

/// Searches every source, ranks the candidates against `filename` and
/// downloads the best few.
pub async fn probe(
    sources: &[Arc<dyn SubtitleSource>],
    query: &Query,
    filename: Option<&str>,
) -> Probe {
    let started = Instant::now();
    let mut searches = Vec::new();
    let mut found: Vec<(usize, Candidate)> = Vec::new();
    // All sources are asked at once: the slowest, not the sum, is the wait.
    let results =
        futures_util::future::join_all(sources.iter().map(|source| source.search(query))).await;
    for (i, (source, result)) in sources.iter().zip(results).enumerate() {
        searches.push((source.kind(), result.as_ref().map(Vec::len).map_err(|e| *e)));
        found.extend(result.unwrap_or_default().into_iter().map(|c| (i, c)));
    }
    found.sort_by_cached_key(|(_, c)| std::cmp::Reverse(rank::score(&c.release, filename)));
    let top_scores = found
        .iter()
        .take(5)
        .map(|(_, c)| rank::score(&c.release, filename))
        .collect();
    let mut probe = Probe {
        report: SourcesProbe {
            elapsed_ms: 0,
            searches,
            top_scores,
            downloads: Vec::new(),
            duplicates: 0,
            timing_copies: 0,
        },
        cues: Vec::new(),
        first_slots: 0,
        sources: sources.to_vec(),
        query: query.clone(),
        filename: filename.map(str::to_owned),
        rest: found,
    };
    probe.first_slots = probe.take(PROBE_DOWNLOADS).await;
    probe.report.elapsed_ms = started.elapsed().as_millis();
    probe
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn movie() -> Query {
        Query::new("movie", "tt0133093", "tur").unwrap()
    }

    fn episode() -> Query {
        Query::new("series", "tt0903747:1:2", "tur").unwrap()
    }

    fn candidate(source: SourceKind, release: &str, link: &str) -> Candidate {
        Candidate {
            source,
            release: release.to_owned(),
            fps_milli: None,
            hearing_impaired: None,
            link: link.to_owned(),
        }
    }

    #[test]
    fn query_reads_movie_and_episode_ids() {
        let q = episode();
        assert_eq!(
            (q.kind, q.season, q.episode),
            (Kind::Series, Some(1), Some(2))
        );
        assert_eq!(q.stremio_id(), "tt0903747:1:2");
        assert_eq!(movie().stremio_id(), "tt0133093");
        for (kind, id) in [
            ("movie", "tt0133093:1:2"),
            ("series", "tt0903747"),
            ("series", "tt0903747:1"),
            ("movie", "tt"),
            ("movie", "nm123"),
            ("movie", "tt12x"),
            ("channel", "tt1"),
            ("series", "tt1:a:2"),
        ] {
            assert_eq!(Query::new(kind, id, "tur"), None, "{kind} {id}");
        }
    }

    #[test]
    fn debug_hides_ids_names_and_links() {
        let shown = format!(
            "{:?} {:?}",
            episode(),
            candidate(
                SourceKind::SubDl,
                "Secret.Release-GRP",
                "https://x.test/secret"
            )
        );
        for hidden in ["tt0903747", "Secret", "x.test"] {
            assert!(!shown.contains(hidden), "{shown}");
        }
    }

    #[test]
    fn opensubtitles_keeps_language_episode_and_https_links() {
        let body = json!({ "subtitles": [
            { "url": "https://subs.test/1", "lang": "tur",
              "subtitleFileName": "Breaking.Bad.S01E02.720p.HDTV.x264-CTU.srt",
              "movieReleaseName": "Breaking Bad", "fpsMilli": 23976,
              "season": 1, "episode": 2 },
            { "url": "https://subs.test/2", "lang": "eng", "season": 1, "episode": 2 },
            { "url": "https://subs.test/3", "lang": "tur", "season": 1, "episode": 3 },
            { "url": "http://subs.test/4", "lang": "tur" },
            { "url": "https://subs.test/5", "lang": "tur",
              "subtitleFileName": "tr.srt", "movieReleaseName": "Breaking.Bad.S01E02.WEB",
              "fpsMilli": "0", "season": 0, "episode": 0 },
            { "lang": "tur" }
        ]})
        .to_string();
        let found = parse_opensubtitles(body.as_bytes(), &episode()).unwrap();
        assert_eq!(
            found,
            vec![
                Candidate {
                    source: SourceKind::OpenSubtitles,
                    release: "Breaking.Bad.S01E02.720p.HDTV.x264-CTU".to_owned(),
                    fps_milli: Some(23_976),
                    hearing_impaired: None,
                    link: "https://subs.test/1".to_owned(),
                },
                Candidate {
                    source: SourceKind::OpenSubtitles,
                    release: "Breaking.Bad.S01E02.WEB".to_owned(),
                    fps_milli: None,
                    hearing_impaired: None,
                    link: "https://subs.test/5".to_owned(),
                },
            ]
        );
        assert_eq!(
            parse_opensubtitles(b"<html>", &episode()),
            Err(SourceError::Decode)
        );
    }

    #[test]
    fn subdl_builds_download_links_and_keeps_episode() {
        let body = json!({ "status": true, "subtitles": [
            { "release_name": "Show.S01E02.1080p.WEB-DL", "url": "/subtitle/1-2.zip",
              "season": 1, "episode": 2, "hi": false },
            { "release_name": "Show.S01.Pack", "url": "/subtitle/3-4.zip",
              "season": 1, "episode": null },
            { "release_name": "Show.S01E05", "url": "/subtitle/5-6.zip",
              "season": 1, "episode": 5 },
            { "name": "only name", "url": "https://elsewhere.test/x.zip" }
        ]})
        .to_string();
        let found = parse_subdl(body.as_bytes(), &episode()).unwrap();
        let links: Vec<&str> = found.iter().map(|c| c.link.as_str()).collect();
        assert_eq!(
            links,
            vec![
                "https://dl.subdl.com/subtitle/1-2.zip",
                "https://dl.subdl.com/subtitle/3-4.zip"
            ]
        );
        assert_eq!(found[0].hearing_impaired, Some(false));
        let empty = json!({ "status": false, "error": "not found" }).to_string();
        assert_eq!(parse_subdl(empty.as_bytes(), &episode()), Ok(Vec::new()));
        assert_eq!(subdl_language("tur"), Some("TR"));
        assert_eq!(subdl_language("xxx"), None);
    }

    #[test]
    fn altyazidb_keeps_language_episode_and_packs_but_not_forced_or_machine() {
        let body = json!({ "success": true, "movie": { "search_method": "imdb_id" }, "data": [
            { "id": "101", "language": "tr", "season": 1, "episode": "2", "fps": "23.976",
              "versions": [{ "group": "FLUX", "file": "Show.S01E02.1080p.WEB.h264-FLUX" }],
              "releases": ["Show.S01E02.720p"], "hearing_impaired": 1, "forced": 0,
              "ai_ceviri": 0 },
            { "id": 102, "language": "tr", "season": 1, "episode": "PAKET", "fps": "N/A",
              "releases": ["Show.S01.1080p.BluRay"], "is_package": 1 },
            { "id": "103", "language": "tr", "season": 1, "episode": "5" },
            { "id": "104", "language": "en", "season": 1, "episode": "2" },
            { "id": "105", "language": "tr", "season": 1, "episode": "2", "forced": 1 },
            { "id": "106", "language": "tr", "season": 1, "episode": "2", "ai_ceviri": 1 },
            { "id": "../x", "language": "tr", "season": 1, "episode": "2" }
        ]})
        .to_string();
        let found = parse_altyazidb(body.as_bytes(), &episode()).unwrap();
        let links: Vec<&str> = found.iter().map(|c| c.link.as_str()).collect();
        assert_eq!(
            links,
            vec![
                "https://altyazidb.com/api/v1/subtitle?sub_id=101",
                "https://altyazidb.com/api/v1/subtitle?sub_id=102"
            ]
        );
        assert_eq!(found[0].release, "Show.S01E02.1080p.WEB.h264-FLUX");
        assert_eq!(found[0].fps_milli, Some(23_976));
        assert_eq!(found[0].hearing_impaired, Some(true));
        assert_eq!(found[1].release, "Show.S01.1080p.BluRay");
        assert_eq!(found[1].fps_milli, None);
        let empty = json!({ "success": true, "data": [] }).to_string();
        assert_eq!(
            parse_altyazidb(empty.as_bytes(), &episode()),
            Ok(Vec::new())
        );
        assert_eq!(
            parse_altyazidb(b"<html>", &episode()),
            Err(SourceError::Decode)
        );
        let other_lang = Query::new("series", "tt0903747:1:2", "pob").unwrap();
        assert_eq!(
            parse_altyazidb(body.as_bytes(), &other_lang),
            Ok(Vec::new())
        );
        assert_eq!(altyazidb_language("eng"), Some("en"));
    }

    #[test]
    fn subsource_title_is_the_asked_season() {
        let body = json!({ "success": true, "data": [
            { "movieId": 11, "imdbId": "tt0903747", "type": "tvseries", "season": 2 },
            { "movieId": 12, "imdbId": "tt0000001", "type": "tvseries", "season": 1 },
            { "movieId": 13, "imdbId": "tt0903747", "type": "tvseries", "season": 1 }
        ]})
        .to_string();
        assert_eq!(
            parse_subsource_title(body.as_bytes(), &episode()),
            Ok(Some(13))
        );
        let film = json!({ "success": true, "data": [
            { "movieId": 21, "imdbId": "tt0133093", "type": "movie", "season": null }
        ]})
        .to_string();
        assert_eq!(
            parse_subsource_title(film.as_bytes(), &movie()),
            Ok(Some(21))
        );
        let none = json!({ "success": true, "data": [] }).to_string();
        assert_eq!(parse_subsource_title(none.as_bytes(), &movie()), Ok(None));
        assert_eq!(
            parse_subsource_title(b"<html>", &movie()),
            Err(SourceError::Decode)
        );
    }

    #[test]
    fn subsource_keeps_language_episode_and_packs_but_not_foreign_parts() {
        let body = json!({ "success": true, "data": [
            { "subtitleId": 201, "language": "turkish", "framerate": "23.976",
              "releaseInfo": ["Show.S01E02.1080p.WEB.h264-FLUX", "Show.S01E02.720p"],
              "hearingImpaired": true, "foreignParts": false },
            { "subtitleId": 202, "language": "turkish", "releaseInfo": ["Show.S01.1080p.BluRay"] },
            { "subtitleId": 203, "language": "turkish", "releaseInfo": ["Show.S01E05.720p"] },
            { "subtitleId": 204, "language": "english", "releaseInfo": ["Show.S01E02.720p"] },
            { "subtitleId": 205, "language": "turkish", "releaseInfo": ["Show.S01E02"],
              "foreignParts": true },
            { "subtitleId": "../x", "language": "turkish", "releaseInfo": ["Show.S01E02"] }
        ], "pagination": { "page": 1, "limit": 100, "total": 6, "pages": 1 } })
        .to_string();
        let found = parse_subsource(body.as_bytes(), &episode()).unwrap();
        let links: Vec<&str> = found.iter().map(|c| c.link.as_str()).collect();
        assert_eq!(
            links,
            vec![
                "https://api.subsource.net/api/v1/subtitles/201/download",
                "https://api.subsource.net/api/v1/subtitles/202/download"
            ]
        );
        assert_eq!(found[0].source, SourceKind::SubSource);
        assert_eq!(found[0].release, "Show.S01E02.1080p.WEB.h264-FLUX");
        assert_eq!(found[0].fps_milli, Some(23_976));
        assert_eq!(found[0].hearing_impaired, Some(true));
        assert_eq!(found[1].release, "Show.S01.1080p.BluRay");
        assert_eq!(found[1].hearing_impaired, None);
        // A movie keeps every release, whatever it names.
        let film = Query::new("movie", "tt0903747", "tur").unwrap();
        assert_eq!(parse_subsource(body.as_bytes(), &film).unwrap().len(), 3);
        assert_eq!(
            parse_subsource(b"<html>", &episode()),
            Err(SourceError::Decode)
        );
        let other_lang = Query::new("series", "tt0903747:1:2", "pob").unwrap();
        assert_eq!(
            parse_subsource(body.as_bytes(), &other_lang),
            Ok(Vec::new())
        );
        assert_eq!(subsource_language("eng"), Some("english"));
    }

    #[test]
    fn wyzie_keeps_language_srt_and_episode_but_not_machine() {
        let url = |id: u32| format!("https://sub.wyzie.io/c/abc/id/{id}?format=srt&tok=t");
        let body = json!([
            { "id": "301", "url": url(301), "format": "srt", "language": "tr",
              "isHearingImpaired": true, "source": "charlie", "release": "Show.S01E02.720p",
              "releases": ["Show.S01E02.1080p.WEB.h264-FLUX"], "fileName": "show.s01e02.srt",
              "ai": false },
            { "id": "302", "url": url(302), "format": "srt", "language": "tr",
              "release": "Show.S01.1080p.BluRay", "releases": null },
            { "id": "303", "url": url(303), "format": "srt", "language": "tr",
              "release": "Show.S01E05.720p" },
            { "id": "304", "url": url(304), "format": "srt", "language": "en",
              "release": "Show.S01E02.720p" },
            { "id": "305", "url": url(305), "format": "ass", "language": "tr",
              "release": "Show.S01E02.720p" },
            { "id": "306", "url": url(306), "format": "srt", "language": "tr",
              "release": "Show.S01E02.720p", "ai": true },
            { "id": "307", "url": "http://sub.wyzie.io/c/x", "format": "srt", "language": "tr" }
        ])
        .to_string();
        let found = parse_wyzie(body.as_bytes(), &episode()).unwrap();
        let links: Vec<&str> = found.iter().map(|c| c.link.as_str()).collect();
        assert_eq!(links, vec![url(301), url(302)]);
        assert_eq!(found[0].source, SourceKind::Wyzie);
        assert_eq!(found[0].release, "Show.S01E02.1080p.WEB.h264-FLUX");
        assert_eq!(found[0].hearing_impaired, Some(true));
        assert_eq!(found[1].release, "Show.S01.1080p.BluRay");
        assert_eq!(found[1].hearing_impaired, None);
        assert_eq!(parse_wyzie(b"[]", &episode()), Ok(Vec::new()));
        assert_eq!(
            parse_wyzie(br#"{"error":"bad key"}"#, &episode()),
            Err(SourceError::Decode)
        );
        let other_lang = Query::new("series", "tt0903747:1:2", "pob").unwrap();
        assert_eq!(parse_wyzie(body.as_bytes(), &other_lang), Ok(Vec::new()));
        assert_eq!(wyzie_language("tur").as_deref(), Some("tr"));
    }

    #[test]
    fn subtitledb_keeps_language_readable_formats_and_episode_but_not_machine() {
        let body = json!({ "subtitles": { "items": [
            { "id": 11, "language": "tr", "format": "srt",
              "release_name": "Show.S01E02.1080p.WEB-DL", "hearing_impaired": true, "fps": 23.976 },
            { "id": "12", "language": "TR", "format": "ass", "release_name": "Show.S01E02.720p",
              "fps": "25" },
            { "id": 13, "language": "tr", "format": "sub", "release_name": "Show.S01E02" },
            { "id": 14, "language": "tr", "release_name": "Show.S01E03" },
            { "id": 15, "language": "en", "release_name": "Show.S01E02" },
            { "id": 16, "language": "tr", "ai_transcribed": true },
            { "id": "../x", "language": "tr" },
            { "language": "tr" }
        ]}})
        .to_string();
        let found = parse_subtitledb(body.as_bytes(), &episode()).unwrap();
        let links: Vec<&str> = found.iter().map(|c| c.link.as_str()).collect();
        assert_eq!(
            links,
            [
                "https://api.thesubtitledb.org/get/11",
                "https://api.thesubtitledb.org/get/12"
            ]
        );
        assert_eq!(found[0].source, SourceKind::SubtitleDb);
        assert_eq!(found[0].release, "Show.S01E02.1080p.WEB-DL");
        assert_eq!(found[0].fps_milli, Some(23_976));
        assert_eq!(found[0].hearing_impaired, Some(true));
        assert_eq!(found[1].fps_milli, Some(25_000));
        assert_eq!(
            parse_subtitledb(br#"{"subtitles":null}"#, &episode()),
            Ok(Vec::new())
        );
        assert_eq!(
            parse_subtitledb(b"<html>", &episode()),
            Err(SourceError::Decode)
        );
        assert_eq!(subtitledb_language("tur").as_deref(), Some("tr"));
        assert_eq!(subtitledb_language("pob").as_deref(), Some("pb"));
    }

    #[test]
    fn subtitledb_download_refuses_pages_and_errors() {
        assert!(looks_like_subtitle(
            b"1\n00:00:01,000 --> 00:00:02,000\nHi\n"
        ));
        assert!(looks_like_subtitle(b"PK\x03\x04"));
        assert!(looks_like_subtitle(b"WEBVTT\n"));
        for bad in [
            &b""[..],
            b"  \n",
            b"{\"error\":1}",
            b"<!DOCTYPE html><p>",
            b"\n<html>",
        ] {
            assert!(!looks_like_subtitle(bad), "{bad:?}");
        }
    }

    /// Counts searches; refuses the key while `status` says so.
    struct Keyed {
        status: std::sync::Mutex<u16>,
        asked: std::sync::atomic::AtomicUsize,
    }

    impl SubtitleSource for Keyed {
        fn kind(&self) -> SourceKind {
            SourceKind::SubDl
        }

        fn search<'a>(&'a self, _query: &'a Query) -> SearchFuture<'a> {
            Box::pin(async move {
                self.asked.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                match *self.status.lock().unwrap() {
                    200 => Ok(Vec::new()),
                    code => Err(SourceError::Status(code)),
                }
            })
        }

        fn download<'a>(&'a self, _q: &'a Query, _c: &'a Candidate) -> DownloadFuture<'a> {
            Box::pin(async move { Err(SourceError::Other) })
        }
    }

    #[tokio::test]
    async fn a_refused_key_is_not_asked_again_but_other_errors_are() {
        let keyed = Arc::new(Keyed {
            status: std::sync::Mutex::new(429),
            asked: Default::default(),
        });
        let source = RefusalMemory::wrap(keyed.clone());
        let asked = || keyed.asked.load(std::sync::atomic::Ordering::SeqCst);
        assert_eq!(source.kind(), SourceKind::SubDl);
        // A full quota passes on in time; it is asked each time.
        for _ in 0..2 {
            assert_eq!(source.search(&movie()).await, Err(SourceError::Status(429)));
        }
        assert_eq!(asked(), 2);
        *keyed.status.lock().unwrap() = 401;
        for _ in 0..3 {
            assert_eq!(source.search(&movie()).await, Err(SourceError::Status(401)));
        }
        assert_eq!(asked(), 3, "asked once, then remembered");
        // Still remembered when the site would answer again.
        *keyed.status.lock().unwrap() = 200;
        assert_eq!(source.search(&movie()).await, Err(SourceError::Status(401)));
        assert_eq!(
            source
                .download(&movie(), &candidate(SourceKind::SubDl, "x", "y"))
                .await,
            Err(SourceError::Status(401))
        );
        assert_eq!(asked(), 3);
    }

    #[test]
    fn episode_marks_read_common_forms() {
        assert_eq!(episode_marks("Show.S01E02.720p.srt"), vec![(1, 2)]);
        assert_eq!(episode_marks("show s1e10 x264"), vec![(1, 10)]);
        assert_eq!(episode_marks("Show 2x05 - Title.srt"), vec![(2, 5)]);
        assert_eq!(episode_marks("Movie.1920x1080.x264.srt"), vec![]);
        assert_eq!(episode_marks("Show.S01.Complete.srt"), vec![]);
    }

    struct FakeSource {
        kind: SourceKind,
        found: Result<Vec<Candidate>, SourceError>,
    }

    impl SubtitleSource for FakeSource {
        fn kind(&self) -> SourceKind {
            self.kind
        }

        fn search<'a>(&'a self, _query: &'a Query) -> SearchFuture<'a> {
            Box::pin(async move { self.found.clone() })
        }

        fn download<'a>(
            &'a self,
            _query: &'a Query,
            candidate: &'a Candidate,
        ) -> DownloadFuture<'a> {
            Box::pin(async move {
                match candidate.link.as_str() {
                    "bad" => Err(SourceError::Status(404)),
                    // A RAR with an ASS file in it.
                    "rar" => {
                        let mut rar = rars::Builder::new(rars::ArchiveVersion::Rar50);
                        let ass = "[Script Info]\n\n[Events]\nFormat: Layer, Start, End, Style, \
                            Name, MarginL, MarginR, MarginV, Effect, Text\n\
                            Dialogue: 0,0:00:05.00,0:00:06.00,Default,,0,0,0,,{\\i1}Packed{\\i0}\n";
                        rar.add_bytes(b"Movie.ass".to_vec(), ass.as_bytes().to_vec(), None, None)
                            .unwrap();
                        Ok(rar.to_bytes().unwrap())
                    }
                    "broken" => Ok(b"Rar!\x1a\x07\x01\x00broken".to_vec()),
                    // The same lines as "dup", stored another way.
                    "copy" => Ok(b"\xEF\xBB\xBF1\r\n00:00:01,000 --> 00:00:02,000\r\nA\r\n\r\n2\r\n00:00:03,000 --> 00:00:04,500\r\nB\r\n".to_vec()),
                    // Each "new…" its own line at its own time.
                    other if other.starts_with("new") => {
                        let s = 10 + other[3..].parse::<u32>().unwrap_or(0) % 40;
                        Ok(format!("1\n00:00:{s:02},000 --> 00:00:{s:02},500\n{other}\n")
                            .into_bytes())
                    }
                    "dup" => Ok(
                        b"1\n00:00:01,000 --> 00:00:02,000\nA\n\n2\n00:00:03,000 --> 00:00:04,500\nB\n"
                            .to_vec(),
                    ),
                    // Each other candidate its own text and times, or it
                    // would be a timing copy of the first (SS-071).
                    _ => Ok(format!(
                        "1\n00:00:01,{0:03} --> 00:00:02,000\n{1}\n\n2\n00:00:03,000 --> 00:00:04,500\nB\n",
                        candidate.release.bytes().map(usize::from).sum::<usize>() * 97 % 1000,
                        candidate.release
                    )
                    .into_bytes()),
                }
            })
        }
    }

    #[tokio::test]
    async fn download_more_takes_the_next_ranked_candidates_once() {
        let file = "Movie.2020.1080p.BluRay.x264-GRP.mkv";
        let found = (0..8)
            .map(|i| {
                let link = if i == 4 { "bad" } else { "ok" };
                candidate(SourceKind::OpenSubtitles, &format!("Movie.2020.r{i}"), link)
            })
            .collect();
        let sources: Vec<Arc<dyn SubtitleSource>> = vec![Arc::new(FakeSource {
            kind: SourceKind::OpenSubtitles,
            found: Ok(found),
        })];
        let mut probe = probe(&sources, &movie(), Some(file)).await;
        assert_eq!((probe.report.downloads.len(), probe.cues.len()), (3, 3));
        assert!(probe.has_more());
        probe.download_more().await;
        assert_eq!((probe.report.downloads.len(), probe.cues.len()), (6, 6));
        assert_eq!(probe.report.downloads[4], Err(SourceError::Status(404)));
        assert!(probe.cues[4].is_none() && probe.cues[5].is_some());
        assert!(probe.has_more(), "two are left");
        probe.download_more().await;
        assert_eq!(probe.report.downloads.len(), 8);
        assert!(!probe.has_more());
        probe.download_more().await;
        assert_eq!(probe.report.downloads.len(), 8, "nothing left to take");
    }

    #[tokio::test]
    async fn a_copy_of_an_earlier_download_gives_its_slot_to_the_next() {
        let file = "Movie.2020.1080p.BluRay.x264-GRP.mkv";
        let ranked = |links: &[&str]| -> Vec<Candidate> {
            links
                .iter()
                .enumerate()
                .map(|(i, link)| {
                    let release = format!("Movie.2020.r{}", "x".repeat(links.len() - i));
                    candidate(SourceKind::Wyzie, &release, link)
                })
                .collect()
        };
        let source = |found| -> Vec<Arc<dyn SubtitleSource>> {
            vec![Arc::new(FakeSource {
                kind: SourceKind::Wyzie,
                found: Ok(found),
            })]
        };
        let lines = |probe: &Probe| -> Vec<String> {
            probe
                .cues
                .iter()
                .map(|c| c.as_ref().map_or("-".into(), |c| c[0].text.clone()))
                .collect()
        };

        // Second and fourth are copies of the first; a failure still counts.
        let sources = source(ranked(&[
            "dup", "copy", "bad", "dup", "new1", "new2", "new3",
        ]));
        let mut first = probe(&sources, &movie(), Some(file)).await;
        assert_eq!(lines(&first), vec!["A", "-", "new1"]);
        assert_eq!(first.report.duplicates, 2);
        assert_eq!(first.report.downloads.len(), first.cues.len());
        first.download_more().await;
        assert_eq!(lines(&first), vec!["A", "-", "new1", "new2", "new3"]);
        assert!(!first.has_more());

        // A run of copies stops after twice the slots, keeping the rest.
        let sources = source(ranked(&["dup", "dup", "dup", "dup", "dup", "dup", "new1"]));
        let probe = probe(&sources, &movie(), Some(file)).await;
        assert_eq!(lines(&probe), vec!["A"]);
        assert_eq!(probe.report.duplicates, 5);
        assert!(probe.has_more(), "the last one is left for later");
    }

    /// Ranked files given as (times, text) ids, `None` for a failed
    /// download; logs what is fetched.
    struct TimedSource {
        files: Vec<Option<(u32, u32)>>,
        fetched: std::sync::Mutex<Vec<usize>>,
    }

    impl SubtitleSource for TimedSource {
        fn kind(&self) -> SourceKind {
            SourceKind::OpenSubtitles
        }

        fn search<'a>(&'a self, _query: &'a Query) -> SearchFuture<'a> {
            let found = (0..self.files.len())
                .map(|i| candidate(SourceKind::OpenSubtitles, "Movie.2020", &i.to_string()))
                .collect();
            Box::pin(async { Ok(found) })
        }

        fn download<'a>(
            &'a self,
            _query: &'a Query,
            candidate: &'a Candidate,
        ) -> DownloadFuture<'a> {
            let i: usize = candidate.link.parse().unwrap();
            self.fetched.lock().unwrap().push(i);
            let file = self.files[i].map(|(times, text)| {
                let s = 10 + times;
                format!("1\n00:00:{s:02},000 --> 00:00:{s:02},500\nx{text}\n\n2\n00:01:{s:02},000 --> 00:01:{s:02},500\ny\n")
                    .into_bytes()
            });
            Box::pin(async move { file.ok_or(SourceError::Status(404)) })
        }
    }

    /// The rule before SS-071 (a timing copy took a slot): where a round of
    /// `want` slots starting at `from` stops.
    fn old_round(
        files: &[Option<(u32, u32)>],
        from: usize,
        want: usize,
        seen: &mut Vec<(u32, u32)>,
    ) -> usize {
        let (mut taken, mut fetched, mut at) = (0, 0, from);
        while taken < want && fetched < 2 * want && at < files.len() {
            let file = files[at];
            at += 1;
            fetched += 1;
            match file {
                Some(lines) if seen.contains(&lines) => {}
                Some(lines) => {
                    seen.push(lines);
                    taken += 1;
                }
                None => taken += 1,
            }
        }
        at
    }

    #[tokio::test]
    async fn timing_copies_never_leave_out_a_candidate_the_old_rule_tried() {
        // Every ranking of 7 files, each a new timing, a timing copy of the
        // first (other text), an exact copy of the first, or a failure.
        const PLACES: u32 = 7;
        for code in 0..4u32.pow(PLACES) {
            let mut files = Vec::new();
            let mut first = None;
            for place in 0..PLACES {
                let fresh = (place + 1, place + 1);
                let file = match (code / 4u32.pow(place) % 4, first) {
                    (1, Some((times, _))) => Some((times, fresh.1)),
                    (2, Some(lines)) => Some(lines),
                    (3, _) => None,
                    _ => Some(fresh),
                };
                if first.is_none() {
                    first = file;
                }
                files.push(file);
            }
            let mut seen = Vec::new();
            let old_first = old_round(&files, 0, PROBE_DOWNLOADS, &mut seen);
            let old_second = old_round(&files, old_first, MORE_DOWNLOADS, &mut seen);
            // What the old first round kept: all but its exact copies.
            let mut kept = Vec::new();
            for file in &files[..old_first] {
                if file.is_none() || !kept.contains(file) {
                    kept.push(*file);
                }
            }

            let source = Arc::new(TimedSource {
                files: files.clone(),
                fetched: Default::default(),
            });
            let sources: Vec<Arc<dyn SubtitleSource>> = vec![source.clone()];
            let mut probe = probe(&sources, &movie(), None).await;
            let fetched = |source: &TimedSource| source.fetched.lock().unwrap().clone();
            let first_round = fetched(&source);
            assert_eq!(
                first_round,
                (0..first_round.len()).collect::<Vec<_>>(),
                "{files:?}"
            );
            assert!(first_round.len() >= old_first, "{files:?}");
            assert_eq!(probe.first_slots, kept.len(), "{files:?}");
            assert!(probe.report.downloads.len() >= probe.first_slots);
            probe.download_more().await;
            assert!(fetched(&source).len() >= old_second, "{files:?}");
        }
    }

    #[tokio::test]
    async fn probe_opens_archives_and_reads_other_formats() {
        let sources: Vec<Arc<dyn SubtitleSource>> = vec![Arc::new(FakeSource {
            kind: SourceKind::SubtitleDb,
            found: Ok(vec![
                candidate(SourceKind::SubtitleDb, "Movie.2020.1080p", "rar"),
                candidate(SourceKind::SubtitleDb, "Movie.2020.720p", "broken"),
            ]),
        })];
        let probe = probe(&sources, &movie(), Some("Movie.2020.1080p.mkv")).await;
        let first = probe.report.downloads[0].as_ref().unwrap();
        assert_eq!(
            (first.source, first.cues, first.first_ms),
            (SourceKind::SubtitleDb, 1, Some(5_000))
        );
        assert_eq!(probe.cues[0].as_ref().unwrap()[0].text, "<i>Packed</i>");
        assert_eq!(probe.report.downloads[1], Err(SourceError::Decode));
        assert!(probe.cues[1].is_none());
    }

    #[tokio::test]
    async fn probe_ranks_across_sources_and_downloads_best() {
        let file = "Movie.2020.1080p.BluRay.x264-GRP.mkv";
        let sources: Vec<Arc<dyn SubtitleSource>> = vec![
            Arc::new(FakeSource {
                kind: SourceKind::OpenSubtitles,
                found: Ok(vec![
                    candidate(SourceKind::OpenSubtitles, "Movie.2020.HDCAM-XYZ", "cam"),
                    candidate(
                        SourceKind::OpenSubtitles,
                        "Movie.2020.720p.WEB-DL-OTHER",
                        "bad",
                    ),
                ]),
            }),
            Arc::new(FakeSource {
                kind: SourceKind::SubDl,
                found: Ok(vec![candidate(
                    SourceKind::SubDl,
                    "Movie.2020.1080p.BluRay.x264-GRP",
                    "best",
                )]),
            }),
            Arc::new(FakeSource {
                kind: SourceKind::SubDl,
                found: Err(SourceError::Status(403)),
            }),
        ];
        let Probe {
            report: outcome,
            cues: times,
            ..
        } = probe(&sources, &movie(), Some(file)).await;
        assert_eq!(
            outcome.searches,
            vec![
                (SourceKind::OpenSubtitles, Ok(2)),
                (SourceKind::SubDl, Ok(1)),
                (SourceKind::SubDl, Err(SourceError::Status(403))),
            ]
        );
        assert_eq!(outcome.top_scores.len(), 3);
        assert!(outcome.top_scores.windows(2).all(|w| w[0] >= w[1]));
        let first = outcome.downloads[0].as_ref().unwrap();
        assert_eq!(
            (first.source, first.cues, first.last_end_ms),
            (SourceKind::SubDl, 2, Some(4_500))
        );
        // The fake moves each candidate's first line by up to a second.
        assert!(
            first
                .first_ms
                .is_some_and(|ms| (1_000..2_000).contains(&ms))
        );
        assert_eq!(outcome.downloads[1], Err(SourceError::Status(404)));
        assert!(outcome.top_scores[2] < 0, "CAM release ranks last");
        let counts: Vec<Option<usize>> = times.iter().map(|t| t.as_ref().map(Vec::len)).collect();
        assert_eq!(
            times[0].as_ref().unwrap()[0].text,
            "Movie.2020.1080p.BluRay.x264-GRP"
        );
        assert_eq!(counts, vec![Some(2), None, Some(2)]);
    }
}
