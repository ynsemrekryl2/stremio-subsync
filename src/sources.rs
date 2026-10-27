//! Candidate subtitles (M3): OpenSubtitles v3 (Stremio's keyless add-on) and
//! SubDL (free key) for one title and language, and one candidate's SRT bytes.
//!
//! IMDb ids, release names, download links and the SubDL key never reach the
//! log: `Debug` on these types prints only kinds, counts and flags.

use reqwest::Url;
use serde::{Deserialize, Deserializer};
use serde_json::Value;
use std::future::Future;
use std::io::Read;
use std::pin::Pin;
use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::rank;
use crate::srt::{self, Cue};

const FETCH_TIMEOUT: Duration = Duration::from_secs(15);
/// Largest list, subtitle file or archive read from a source.
const MAX_DOWNLOAD: usize = 4 * 1024 * 1024;
/// Largest SRT taken out of an archive.
const MAX_SRT: u64 = 2 * 1024 * 1024;
/// Candidates the probe downloads.
const PROBE_DOWNLOADS: usize = 3;

const OPENSUBTITLES_V3: &str = "https://opensubtitles-v3.strem.io/";
const SUBDL_API: &str = "https://api.subdl.com/api/v1/subtitles";
const SUBDL_DOWNLOAD: &str = "https://dl.subdl.com";

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
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum SourceKind {
    OpenSubtitles,
    SubDl,
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
    /// Bytes of one SRT file for `candidate`.
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

/// `(season, episode)` pairs written in a file name: `S01E02`, `s1e2`, `1x02`.
fn episode_marks(name: &str) -> Vec<(u32, u32)> {
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

/// The SRT inside a SubDL archive: for an episode, the file named for it
/// (or the only one); for a movie, the largest.
pub fn pick_srt(archive: &[u8], query: &Query) -> Result<Vec<u8>, SourceError> {
    let mut zip =
        zip::ZipArchive::new(std::io::Cursor::new(archive)).map_err(|_| SourceError::Decode)?;
    let mut files = Vec::new();
    for i in 0..zip.len() {
        let Ok(file) = zip.by_index(i) else { continue };
        // Old archives store names in a legacy code page; only ".srt" and
        // episode marks matter here, so a lossy read is enough.
        let name = String::from_utf8_lossy(file.name_raw()).into_owned();
        if file.is_file()
            && name.to_ascii_lowercase().ends_with(".srt")
            && !name.starts_with("__MACOSX")
        {
            files.push((i, file.size(), episode_marks(&name)));
        }
    }
    let chosen = match (query.season, query.episode) {
        (Some(season), Some(episode)) => {
            let named: Vec<_> = files
                .iter()
                .filter(|f| f.2.contains(&(season, episode)))
                .collect();
            match named.as_slice() {
                [] if files.len() == 1 => files.first(),
                named => named.iter().copied().max_by_key(|f| f.1),
            }
        }
        _ => files.iter().max_by_key(|f| f.1),
    };
    let &(index, size, _) = chosen.ok_or(SourceError::NoSubtitle)?;
    if size > MAX_SRT {
        return Err(SourceError::TooLarge);
    }
    let mut bytes = Vec::new();
    zip.by_index(index)
        .map_err(|_| SourceError::Decode)?
        .take(MAX_SRT + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| SourceError::Decode)?;
    if bytes.len() as u64 > MAX_SRT {
        return Err(SourceError::TooLarge);
    }
    Ok(bytes)
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
async fn get_capped(client: &reqwest::Client, url: Url) -> Result<Vec<u8>, SourceError> {
    let mut response = client
        .get(url)
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
            parse_opensubtitles(&get_capped(&self.client, url).await?, query)
        })
    }

    fn download<'a>(&'a self, _query: &'a Query, candidate: &'a Candidate) -> DownloadFuture<'a> {
        Box::pin(async move {
            let url = Url::parse(&candidate.link).map_err(|_| SourceError::Other)?;
            get_capped(&self.client, url).await
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
            parse_subdl(&get_capped(&self.client, url).await?, query)
        })
    }

    fn download<'a>(&'a self, query: &'a Query, candidate: &'a Candidate) -> DownloadFuture<'a> {
        Box::pin(async move {
            let url = Url::parse(&candidate.link).map_err(|_| SourceError::Other)?;
            pick_srt(&get_capped(&self.client, url).await?, query)
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
}

/// Searches every source, ranks the candidates against `filename` and
/// downloads the best few. Beside the report, the cues of each download
/// (`None` where it failed), in the same order.
pub async fn probe(
    sources: &[Arc<dyn SubtitleSource>],
    query: &Query,
    filename: Option<&str>,
) -> (SourcesProbe, Vec<Option<Vec<Cue>>>) {
    let started = Instant::now();
    let mut searches = Vec::new();
    let mut found: Vec<(usize, Candidate)> = Vec::new();
    for (i, source) in sources.iter().enumerate() {
        let result = source.search(query).await;
        searches.push((source.kind(), result.as_ref().map(Vec::len).map_err(|e| *e)));
        found.extend(result.unwrap_or_default().into_iter().map(|c| (i, c)));
    }
    found.sort_by_cached_key(|(_, c)| std::cmp::Reverse(rank::score(&c.release, filename)));
    let top_scores = found
        .iter()
        .take(5)
        .map(|(_, c)| rank::score(&c.release, filename))
        .collect();
    let mut downloads = Vec::new();
    let mut read = Vec::new();
    for (i, candidate) in found.iter().take(PROBE_DOWNLOADS) {
        let result = sources[*i].download(query, candidate).await.map(|bytes| {
            let cues = srt::parse(&srt::decode(&bytes, &query.lang));
            let stats = SrtStats {
                source: candidate.source,
                score: rank::score(&candidate.release, filename),
                bytes: bytes.len(),
                cues: cues.len(),
                first_ms: cues.first().map(|c| c.start_ms),
                last_end_ms: cues.iter().map(|c| c.end_ms).max(),
            };
            (stats, cues)
        });
        let result = match result {
            Ok((stats, cues)) => {
                read.push(Some(cues));
                Ok(stats)
            }
            Err(err) => {
                read.push(None);
                Err(err)
            }
        };
        downloads.push(result);
    }
    let report = SourcesProbe {
        elapsed_ms: started.elapsed().as_millis(),
        searches,
        top_scores,
        downloads,
    };
    (report, read)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::io::Write;

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
    fn episode_marks_read_common_forms() {
        assert_eq!(episode_marks("Show.S01E02.720p.srt"), vec![(1, 2)]);
        assert_eq!(episode_marks("show s1e10 x264"), vec![(1, 10)]);
        assert_eq!(episode_marks("Show 2x05 - Title.srt"), vec![(2, 5)]);
        assert_eq!(episode_marks("Movie.1920x1080.x264.srt"), vec![]);
        assert_eq!(episode_marks("Show.S01.Complete.srt"), vec![]);
    }

    fn zip_of(files: &[(&str, &[u8])]) -> Vec<u8> {
        let mut out = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
        for (name, body) in files {
            out.start_file(*name, zip::write::SimpleFileOptions::default())
                .unwrap();
            out.write_all(body).unwrap();
        }
        out.finish().unwrap().into_inner()
    }

    #[test]
    fn pick_srt_takes_the_episode_or_largest() {
        let pack = zip_of(&[
            ("Show.S01E01.srt", b"one"),
            ("Show.S01E02.srt", b"two"),
            ("Show.S01E02.HI.srt", b"two, longer"),
            ("__MACOSX/Show.S01E02.srt", b"junk junk junk junk"),
            ("readme.txt", b"hello hello hello hello"),
        ]);
        assert_eq!(pick_srt(&pack, &episode()).unwrap(), b"two, longer");

        let single = zip_of(&[("tr.srt", b"only")]);
        assert_eq!(pick_srt(&single, &episode()).unwrap(), b"only");

        let other = zip_of(&[("Show.S01E01.srt", b"one"), ("Show.S01E03.srt", b"three")]);
        assert_eq!(pick_srt(&other, &episode()), Err(SourceError::NoSubtitle));

        let movie_zip = zip_of(&[("cd.srt", b"short"), ("full.srt", b"much longer")]);
        assert_eq!(pick_srt(&movie_zip, &movie()).unwrap(), b"much longer");

        assert_eq!(pick_srt(b"not a zip", &movie()), Err(SourceError::Decode));
        let none = zip_of(&[("notes.txt", b"x")]);
        assert_eq!(pick_srt(&none, &movie()), Err(SourceError::NoSubtitle));
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
                    _ => Ok(b"1\n00:00:01,000 --> 00:00:02,000\nA\n\n2\n00:00:03,000 --> 00:00:04,500\nB\n"
                        .to_vec()),
                }
            })
        }
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
        let (outcome, times) = probe(&sources, &movie(), Some(file)).await;
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
            (first.source, first.cues, first.first_ms, first.last_end_ms),
            (SourceKind::SubDl, 2, Some(1_000), Some(4_500))
        );
        assert_eq!(outcome.downloads[1], Err(SourceError::Status(404)));
        assert!(outcome.top_scores[2] < 0, "CAM release ranks last");
        let counts: Vec<Option<usize>> = times.iter().map(|t| t.as_ref().map(Vec::len)).collect();
        assert_eq!(times[0].as_ref().unwrap()[0].text, "A");
        assert_eq!(counts, vec![Some(2), None, Some(2)]);
    }
}
