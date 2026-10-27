//! K2 probe: reads the AIOStreams stream list for the title being played and
//! checks whether the player's hints pick out exactly one stream.
//!
//! Stream URLs are never requested here: on TorBox, opening one can start a
//! download. Only counts and booleans leave this module through `Debug`.

use reqwest::Url;
use serde::{Deserialize, Deserializer};
use serde_json::Value;
use std::future::Future;
use std::pin::Pin;
use std::time::{Duration, Instant};

use crate::blocks;
use crate::extra::PlaybackHints;
use crate::mkv::{self, MkvOutcome};
use crate::range::{ByteSource, Opened, RangeOutcome, RangeReader};

const FETCH_TIMEOUT: Duration = Duration::from_secs(15);

/// One entry of a Stremio stream list. `Debug` is deliberately not derived:
/// the URL and filename must never reach the log.
#[derive(Deserialize)]
pub struct Stream {
    url: Option<String>,
    #[serde(rename = "infoHash")]
    info_hash: Option<String>,
    #[serde(rename = "behaviorHints", default)]
    behavior_hints: BehaviorHints,
}

#[derive(Deserialize, Default)]
struct BehaviorHints {
    filename: Option<String>,
    #[serde(rename = "videoSize", default, deserialize_with = "lenient_u64")]
    video_size: Option<u64>,
}

#[derive(Deserialize)]
struct StreamList {
    #[serde(default)]
    streams: Vec<Stream>,
}

/// Accepts a size given as a number or a numeric string; anything else is absent.
fn lenient_u64<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Option<u64>, D::Error> {
    Ok(match Value::deserialize(deserializer)? {
        Value::Number(n) => n.as_u64(),
        Value::String(s) => s.trim().parse().ok(),
        _ => None,
    })
}

pub fn parse_stream_list(body: &[u8]) -> Result<Vec<Stream>, FetchError> {
    serde_json::from_slice::<StreamList>(body)
        .map(|list| list.streams)
        .map_err(|_| FetchError::Decode)
}

/// Why a stream list could not be read. Carries no URL.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FetchError {
    Timeout,
    Connect,
    Status(u16),
    Decode,
    Other,
}

/// What kind of playback target the single matched stream points to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Target {
    /// HTTP(S) URL; `same_host` tells whether it is served by AIOStreams itself.
    Http {
        same_host: bool,
    },
    /// Only an info hash, no URL: the player would need its own torrent client.
    InfoHashOnly,
    Missing,
}

/// Counts that answer K2 without revealing what is being watched.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct MatchReport {
    pub streams: usize,
    pub with_filename: usize,
    pub with_size: usize,
    pub filename_exact: usize,
    /// Case-insensitive, last path segment only.
    pub filename_loose: usize,
    pub size: usize,
    /// Exact filename and size both equal.
    pub both: usize,
    /// Set only when exactly one stream matched on both fields.
    pub target: Option<Target>,
}

fn last_segment(name: &str) -> &str {
    name.rsplit(['/', '\\']).next().unwrap_or(name)
}

/// Counts matches and hands back the single stream matched on both fields,
/// if there is exactly one.
pub fn match_streams<'a>(
    hints: &PlaybackHints,
    streams: &'a [Stream],
    source_host: &str,
) -> (MatchReport, Option<&'a Stream>) {
    let mut report = MatchReport {
        streams: streams.len(),
        ..MatchReport::default()
    };
    let mut both = Vec::new();
    for stream in streams {
        let name = stream.behavior_hints.filename.as_deref();
        let size = stream.behavior_hints.video_size;
        report.with_filename += usize::from(name.is_some());
        report.with_size += usize::from(size.is_some());

        let exact = name.is_some() && name == hints.filename.as_deref();
        let loose = match (name, hints.filename.as_deref()) {
            (Some(a), Some(b)) => last_segment(a).eq_ignore_ascii_case(last_segment(b)),
            _ => false,
        };
        let size_eq = size.is_some() && size == hints.video_size;
        report.filename_exact += usize::from(exact);
        report.filename_loose += usize::from(loose);
        report.size += usize::from(size_eq);
        if exact && size_eq {
            both.push(stream);
        }
    }
    report.both = both.len();
    let single = match both.as_slice() {
        [stream] => Some(*stream),
        _ => None,
    };
    report.target = single.map(|stream| target_of(stream, source_host));
    (report, single)
}

fn target_of(stream: &Stream, source_host: &str) -> Target {
    match stream.url.as_deref().and_then(|raw| Url::parse(raw).ok()) {
        Some(url) if matches!(url.scheme(), "http" | "https") => Target::Http {
            same_host: url.host_str() == Some(source_host),
        },
        _ if stream.info_hash.is_some() => Target::InfoHashOnly,
        _ => Target::Missing,
    }
}

pub type FetchFuture<'a> =
    Pin<Box<dyn Future<Output = Result<Vec<Stream>, FetchError>> + Send + 'a>>;

/// Where stream lists come from; faked in tests.
pub trait StreamSource: Send + Sync {
    fn fetch<'a>(&'a self, kind: &'a str, id: &'a str) -> FetchFuture<'a>;
    /// Host of the source, used to tell proxied URLs from direct debrid links.
    fn host(&self) -> &str;
}

/// Result of one probe, safe to log.
#[derive(Debug, PartialEq, Eq)]
pub enum ProbeOutcome {
    /// Neither filename nor size was sent, so there is nothing to match.
    NoHints,
    Matched {
        elapsed_ms: u128,
        report: MatchReport,
        /// Present only when the range read is enabled and the single match
        /// has an HTTP URL.
        range: Option<RangeOutcome>,
        /// Tracks and Cues of the matched file; only after a partial read
        /// that starts with the EBML header.
        mkv: Option<Box<MkvOutcome>>,
    },
    Failed {
        elapsed_ms: u128,
        error: FetchError,
    },
}

/// With `block_probe`, the ruler track's blocks are then read in the
/// background and logged on their own line (SS-028).
pub async fn probe(
    source: &dyn StreamSource,
    range: Option<&dyn RangeReader>,
    block_probe: bool,
    kind: &str,
    id: &str,
    hints: &PlaybackHints,
) -> ProbeOutcome {
    if hints.filename.is_none() && hints.video_size.is_none() {
        return ProbeOutcome::NoHints;
    }
    let started = Instant::now();
    let result = source.fetch(kind, id).await;
    let elapsed_ms = started.elapsed().as_millis();
    let streams = match result {
        Ok(streams) => streams,
        Err(error) => return ProbeOutcome::Failed { elapsed_ms, error },
    };
    let (report, single) = match_streams(hints, &streams, source.host());
    let (range, mkv) = match (range, report.target, single.and_then(|s| s.url.as_deref())) {
        (Some(reader), Some(Target::Http { .. }), Some(url)) => {
            match reader.open(url, source.host(), hints.video_size).await {
                Ok(Opened { report, head, file }) => {
                    let file: std::sync::Arc<dyn ByteSource> = file.into();
                    let mkv = if report.partial && report.ebml_magic {
                        Some(Box::new(mkv::inspect(file.as_ref(), &head).await))
                    } else {
                        None
                    };
                    let targets = mkv
                        .as_deref()
                        .and_then(|outcome| outcome.as_ref().ok())
                        .and_then(|report| report.blocks.clone());
                    if block_probe && let Some(targets) = targets {
                        let kind = kind.to_owned();
                        tokio::spawn(async move {
                            let report = blocks::run(file, targets).await;
                            eprintln!("m6 blocks kind={kind} report={report:?}");
                        });
                    }
                    (Some(Ok(report)), mkv)
                }
                Err(err) => (Some(Err(err)), None),
            }
        }
        _ => (None, None),
    };
    ProbeOutcome::Matched {
        elapsed_ms,
        report,
        range,
        mkv,
    }
}

/// Why a manifest link was refused. Carries no part of the link.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ManifestUrlError {
    Empty,
    ControlChars,
    ManyLinks,
    NotAUrl,
    Scheme,
    NotManifest,
    Client,
}

impl std::fmt::Display for ManifestUrlError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Empty => "empty",
            Self::ControlChars => "contains control characters (paste it again, or type it)",
            Self::ManyLinks => "contains more than one manifest.json link",
            Self::NotAUrl => "not a URL (check for missing characters or extra text)",
            Self::Scheme => "must start with https://, http:// or stremio://",
            Self::NotManifest => "path must end in manifest.json",
            Self::Client => "cannot build the HTTP client",
        })
    }
}

/// Undoes what copying and pasting tends to add: terminal paste markers,
/// surrounding quotes or angle brackets, text around the link, a link broken
/// by spaces, and a `stremio://` or missing scheme.
fn clean_link(raw: &str) -> Result<String, ManifestUrlError> {
    let raw = raw.replace("\u{1b}[200~", "").replace("\u{1b}[201~", "");
    let strip = |s: &str| {
        s.trim_matches(|c: char| c.is_whitespace() || matches!(c, '"' | '\'' | '<' | '>'))
            .to_owned()
    };
    // A URL never holds raw whitespace, so any found is from copying.
    let candidates: Vec<&str> = raw
        .split_whitespace()
        .filter(|part| part.contains("manifest.json"))
        .collect();
    let link = match candidates.as_slice() {
        [one] if one.contains("://") => strip(one),
        [] | [_] => strip(&raw.split_whitespace().collect::<String>()),
        _ => return Err(ManifestUrlError::ManyLinks),
    };
    if link.is_empty() {
        return Err(ManifestUrlError::Empty);
    }
    if link.chars().any(char::is_control) {
        return Err(ManifestUrlError::ControlChars);
    }
    Ok(match link.strip_prefix("stremio://") {
        Some(rest) => format!("https://{rest}"),
        None if !link.contains("://") => format!("https://{link}"),
        None => link,
    })
}

/// AIOStreams instance addressed by its manifest link. The link embeds the
/// user's configuration, so this type has no `Debug` and errors drop the URL.
pub struct AioStreams {
    base: Url,
    client: reqwest::Client,
}

impl AioStreams {
    /// Accepts an `http(s)://…/manifest.json` link, or the `stremio://` form
    /// of it that install buttons hand out.
    pub fn from_manifest_url(raw: &str) -> Result<Self, ManifestUrlError> {
        let url = Url::parse(&clean_link(raw)?).map_err(|_| ManifestUrlError::NotAUrl)?;
        if !matches!(url.scheme(), "http" | "https") || url.host_str().is_none() {
            return Err(ManifestUrlError::Scheme);
        }
        let path = url
            .path()
            .strip_suffix("manifest.json")
            .ok_or(ManifestUrlError::NotManifest)?;
        let mut base = url.clone();
        base.set_path(path);
        base.set_query(None);
        base.set_fragment(None);
        let client = reqwest::Client::builder()
            .timeout(FETCH_TIMEOUT)
            .user_agent(concat!("stremio-subsync/", env!("CARGO_PKG_VERSION")))
            .build()
            .map_err(|_| ManifestUrlError::Client)?;
        Ok(Self { base, client })
    }

    fn stream_url(&self, kind: &str, id: &str) -> Option<Url> {
        self.base.join(&format!("stream/{kind}/{id}.json")).ok()
    }
}

fn classify(err: &reqwest::Error) -> FetchError {
    if err.is_timeout() {
        FetchError::Timeout
    } else if err.is_connect() {
        FetchError::Connect
    } else if let Some(status) = err.status() {
        FetchError::Status(status.as_u16())
    } else if err.is_decode() {
        FetchError::Decode
    } else {
        FetchError::Other
    }
}

impl StreamSource for AioStreams {
    fn fetch<'a>(&'a self, kind: &'a str, id: &'a str) -> FetchFuture<'a> {
        Box::pin(async move {
            let url = self.stream_url(kind, id).ok_or(FetchError::Other)?;
            let response = self
                .client
                .get(url)
                .send()
                .await
                .and_then(reqwest::Response::error_for_status)
                .map_err(|err| classify(&err))?;
            let body = response.bytes().await.map_err(|err| classify(&err))?;
            parse_stream_list(&body)
        })
    }

    fn host(&self) -> &str {
        self.base.host_str().unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const HOST: &str = "aio.test";

    fn streams(value: Value) -> Vec<Stream> {
        parse_stream_list(value.to_string().as_bytes()).unwrap()
    }

    fn hints(filename: &str, size: u64) -> PlaybackHints {
        PlaybackHints {
            filename: Some(filename.to_owned()),
            video_size: Some(size),
            video_hash: None,
        }
    }

    fn sample() -> Vec<Stream> {
        streams(json!({ "streams": [
            { "url": "https://aio.test/playback/abc",
              "behaviorHints": { "filename": "Show.S01E02.1080p.mkv", "videoSize": 1000 } },
            { "url": "https://debrid.test/d/xyz",
              "behaviorHints": { "filename": "Show.S01E02.2160p.mkv", "videoSize": "2000" } },
            { "url": "https://debrid.test/d/other",
              "behaviorHints": { "filename": "Pack/show.s01e02.1080p.MKV", "videoSize": 1001 } },
            { "infoHash": "abcdef", "behaviorHints": {} },
            { "name": "no hints at all" }
        ]}))
    }

    #[test]
    fn single_exact_match_reports_target() {
        let report = match_streams(&hints("Show.S01E02.1080p.mkv", 1000), &sample(), HOST).0;
        assert_eq!(
            report,
            MatchReport {
                streams: 5,
                with_filename: 3,
                with_size: 3,
                filename_exact: 1,
                filename_loose: 2,
                size: 1,
                both: 1,
                target: Some(Target::Http { same_host: true }),
            }
        );
    }

    #[test]
    fn size_given_as_string_is_accepted() {
        let report = match_streams(&hints("Show.S01E02.2160p.mkv", 2000), &sample(), HOST).0;
        assert_eq!(report.both, 1);
        assert_eq!(report.target, Some(Target::Http { same_host: false }));
    }

    #[test]
    fn ambiguous_match_has_no_target() {
        let list = streams(json!({ "streams": [
            { "url": "https://a.test/1", "behaviorHints": { "filename": "A.mkv", "videoSize": 5 } },
            { "url": "https://a.test/2", "behaviorHints": { "filename": "A.mkv", "videoSize": 5 } }
        ]}));
        let report = match_streams(&hints("A.mkv", 5), &list, HOST).0;
        assert_eq!(report.both, 2);
        assert_eq!(report.target, None);
    }

    #[test]
    fn info_hash_only_target() {
        let list = streams(json!({ "streams": [
            { "infoHash": "abc", "behaviorHints": { "filename": "A.mkv", "videoSize": 5 } }
        ]}));
        let report = match_streams(&hints("A.mkv", 5), &list, HOST).0;
        assert_eq!(report.target, Some(Target::InfoHashOnly));
    }

    #[test]
    fn malformed_body_is_decode_error() {
        assert_eq!(parse_stream_list(b"<html>").err(), Some(FetchError::Decode));
        assert!(parse_stream_list(b"{}").unwrap().is_empty());
    }

    #[test]
    fn manifest_url_becomes_stream_base() {
        let source =
            AioStreams::from_manifest_url("https://aio.test/stremio/u/secret/manifest.json")
                .unwrap();
        assert_eq!(source.host(), "aio.test");
        assert_eq!(
            source
                .stream_url("series", "tt0944947:1:2")
                .unwrap()
                .as_str(),
            "https://aio.test/stremio/u/secret/stream/series/tt0944947:1:2.json"
        );
    }

    #[test]
    fn stremio_scheme_is_read_as_https() {
        let source =
            AioStreams::from_manifest_url(" stremio://aio.test/stremio/u/secret/manifest.json\n")
                .unwrap();
        assert_eq!(
            source.stream_url("movie", "tt1").unwrap().as_str(),
            "https://aio.test/stremio/u/secret/stream/movie/tt1.json"
        );
    }

    #[test]
    fn paste_artifacts_are_removed() {
        let expected = "https://aio.test/stremio/u/secret/stream/movie/tt1.json";
        for raw in [
            "\u{1b}[200~https://aio.test/stremio/u/secret/manifest.json\u{1b}[201~",
            "\"https://aio.test/stremio/u/secret/manifest.json\"",
            "<https://aio.test/stremio/u/secret/manifest.json>",
            "aio.test/stremio/u/secret/manifest.json",
            "Manifest URL: https://aio.test/stremio/u/secret/manifest.json (copied)",
            "https://aio.test/stremio/u/se cret/manifest.json",
            "\thttps://aio.test/stremio/u/secret/manifest.json\t",
        ] {
            let source = AioStreams::from_manifest_url(raw).unwrap();
            assert_eq!(
                source.stream_url("movie", "tt1").unwrap().as_str(),
                expected,
                "{raw:?}"
            );
        }
    }

    #[test]
    fn refusal_reason_names_the_failed_check() {
        let reason = |raw| AioStreams::from_manifest_url(raw).err();
        assert_eq!(reason("  "), Some(ManifestUrlError::Empty));
        assert_eq!(
            reason("https://a.test/\u{7}manifest.json"),
            Some(ManifestUrlError::ControlChars)
        );
        assert_eq!(
            reason("https://a.test/manifest.json https://b.test/manifest.json"),
            Some(ManifestUrlError::ManyLinks)
        );
        assert_eq!(
            reason("https://[bad/manifest.json"),
            Some(ManifestUrlError::NotAUrl)
        );
        assert_eq!(
            reason("ftp://aio.test/manifest.json"),
            Some(ManifestUrlError::Scheme)
        );
        assert_eq!(
            reason("https://aio.test/configure"),
            Some(ManifestUrlError::NotManifest)
        );
        let printed = ManifestUrlError::NotManifest.to_string();
        assert!(!printed.contains("aio.test"), "{printed}");
    }

    #[test]
    fn non_manifest_urls_are_rejected() {
        for raw in [
            "https://aio.test/stremio/u/secret/",
            "ftp://aio.test/manifest.json",
            "not a url",
        ] {
            assert!(AioStreams::from_manifest_url(raw).is_err(), "{raw}");
        }
    }

    struct FakeSource(Result<Value, FetchError>);

    impl StreamSource for FakeSource {
        fn fetch<'a>(&'a self, _kind: &'a str, _id: &'a str) -> FetchFuture<'a> {
            let result = self.0.clone().map(streams);
            Box::pin(async move { result })
        }

        fn host(&self) -> &str {
            HOST
        }
    }

    struct FakeRange(std::sync::Mutex<Vec<(String, Option<u64>)>>);

    impl RangeReader for FakeRange {
        fn open<'a>(
            &'a self,
            url: &'a str,
            _source_host: &'a str,
            expected_size: Option<u64>,
        ) -> crate::range::OpenFuture<'a> {
            self.0.lock().unwrap().push((url.to_owned(), expected_size));
            Box::pin(async { Err(crate::range::RangeError::Other { hops: Vec::new() }) })
        }
    }

    #[tokio::test]
    async fn probe_reads_range_of_single_match_only() {
        let reader = FakeRange(std::sync::Mutex::new(Vec::new()));
        let source = FakeSource(Ok(json!({ "streams": [
            { "url": "https://d.test/one", "behaviorHints": { "filename": "A.mkv", "videoSize": 5 } },
            { "url": "https://d.test/two", "behaviorHints": { "filename": "B.mkv", "videoSize": 5 } }
        ]})));
        let outcome = probe(
            &source,
            Some(&reader),
            false,
            "movie",
            "tt1",
            &hints("A.mkv", 5),
        )
        .await;
        assert!(matches!(
            outcome,
            ProbeOutcome::Matched {
                range: Some(Err(_)),
                ..
            }
        ));
        assert_eq!(
            *reader.0.lock().unwrap(),
            vec![("https://d.test/one".to_owned(), Some(5))]
        );

        // Ambiguous: two streams match, so no URL is touched.
        let reader = FakeRange(std::sync::Mutex::new(Vec::new()));
        let source = FakeSource(Ok(json!({ "streams": [
            { "url": "https://d.test/one", "behaviorHints": { "filename": "A.mkv", "videoSize": 5 } },
            { "url": "https://d.test/two", "behaviorHints": { "filename": "A.mkv", "videoSize": 5 } }
        ]})));
        let outcome = probe(
            &source,
            Some(&reader),
            false,
            "movie",
            "tt1",
            &hints("A.mkv", 5),
        )
        .await;
        assert!(matches!(outcome, ProbeOutcome::Matched { range: None, .. }));
        assert!(reader.0.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn probe_skips_fetch_without_hints() {
        let source = FakeSource(Err(FetchError::Other));
        let outcome = probe(
            &source,
            None,
            false,
            "movie",
            "tt1",
            &PlaybackHints::default(),
        )
        .await;
        assert_eq!(outcome, ProbeOutcome::NoHints);
    }

    #[tokio::test]
    async fn probe_reports_fetch_failure() {
        let source = FakeSource(Err(FetchError::Status(502)));
        let outcome = probe(&source, None, false, "movie", "tt1", &hints("A.mkv", 5)).await;
        assert!(matches!(
            outcome,
            ProbeOutcome::Failed {
                error: FetchError::Status(502),
                ..
            }
        ));
    }

    #[tokio::test]
    async fn probe_debug_hides_filename_and_url() {
        let source = FakeSource(Ok(json!({ "streams": [
            { "url": "https://secret-host.test/Secret.Title.mkv",
              "behaviorHints": { "filename": "Secret.Title.mkv", "videoSize": 5 } }
        ]})));
        let outcome = probe(
            &source,
            None,
            false,
            "movie",
            "tt1",
            &hints("Secret.Title.mkv", 5),
        )
        .await;
        let printed = format!("{outcome:?}");
        assert!(!printed.contains("Secret"), "{printed}");
        assert!(!printed.contains("secret-host"), "{printed}");
        assert!(printed.contains("both: 1"), "{printed}");
    }
}
