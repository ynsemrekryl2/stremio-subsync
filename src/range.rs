//! K2/K3 range read: asks the matched stream URL for its first bytes only,
//! following redirects by hand so each hop's host kind can be recorded.
//!
//! Off unless `SUBSYNC_RANGE_PROBE=1`. URLs never leave this module; reports
//! carry host kinds, status codes, counts and booleans.

use reqwest::header::{
    ACCEPT_RANGES, CONTENT_LENGTH, CONTENT_RANGE, CONTENT_TYPE, HeaderMap, LOCATION, RANGE,
};
use reqwest::{StatusCode, Url, redirect};
use std::future::Future;
use std::pin::Pin;
use std::time::{Duration, Instant};

/// How much of the file one probe may read.
pub const HEAD_BYTES: usize = 64 * 1024;
const MAX_REDIRECTS: usize = 5;
const READ_TIMEOUT: Duration = Duration::from_secs(20);
const EBML_MAGIC: [u8; 4] = [0x1A, 0x45, 0xDF, 0xA3];

/// Who serves a URL, without saying which URL.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HostKind {
    AioStreams,
    RealDebrid,
    TorBox,
    Other,
}

fn ends_with_domain(host: &str, domain: &str) -> bool {
    host == domain
        || host
            .strip_suffix(domain)
            .is_some_and(|rest| rest.ends_with('.'))
}

pub fn host_kind(host: &str, source_host: &str) -> HostKind {
    let host = host.to_ascii_lowercase();
    if host == source_host.to_ascii_lowercase() {
        HostKind::AioStreams
    } else if ["real-debrid.com", "rdeb.io"]
        .iter()
        .any(|d| ends_with_domain(&host, d))
    {
        HostKind::RealDebrid
    } else if ["torbox.app", "tb-cdn.st", "tb-cdn.io"]
        .iter()
        .any(|d| ends_with_domain(&host, d))
    {
        HostKind::TorBox
    } else {
        HostKind::Other
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContentKind {
    Matroska,
    OctetStream,
    OtherVideo,
    Other,
    Missing,
}

fn content_kind(headers: &HeaderMap) -> ContentKind {
    let Some(value) = headers.get(CONTENT_TYPE).and_then(|v| v.to_str().ok()) else {
        return ContentKind::Missing;
    };
    let mime = value.split(';').next().unwrap_or_default().trim();
    match mime.to_ascii_lowercase().as_str() {
        "video/x-matroska" | "video/matroska" | "video/webm" => ContentKind::Matroska,
        "application/octet-stream" | "binary/octet-stream" => ContentKind::OctetStream,
        m if m.starts_with("video/") => ContentKind::OtherVideo,
        _ => ContentKind::Other,
    }
}

/// Full file size the server claims: the total of `Content-Range` on a
/// partial answer, `Content-Length` on a full one.
fn total_size(status: StatusCode, headers: &HeaderMap) -> Option<u64> {
    let header = |name| headers.get(name).and_then(|v| v.to_str().ok());
    if status == StatusCode::PARTIAL_CONTENT {
        header(CONTENT_RANGE)?
            .rsplit('/')
            .next()?
            .trim()
            .parse()
            .ok()
    } else if status.is_success() {
        header(CONTENT_LENGTH)?.trim().parse().ok()
    } else {
        None
    }
}

/// What the final answer told us. Safe to log.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RangeReport {
    /// Host kind of every URL requested; the first is the stream URL itself.
    pub hops: Vec<HostKind>,
    pub status: u16,
    /// 206 with a `Content-Range`: the server honoured the range.
    pub partial: bool,
    /// `Accept-Ranges: bytes` was sent.
    pub accept_ranges: bool,
    /// Claimed file size equals the player's `videoSize`; `None` if unknown.
    pub size_matches: Option<bool>,
    pub content: ContentKind,
    pub bytes_read: usize,
    /// First bytes are the EBML header, so the file is Matroska.
    pub ebml_magic: bool,
    pub elapsed_ms: u128,
}

pub fn analyze(
    hops: Vec<HostKind>,
    status: StatusCode,
    headers: &HeaderMap,
    body: &[u8],
    expected_size: Option<u64>,
    elapsed_ms: u128,
) -> RangeReport {
    let accept_ranges = headers
        .get(ACCEPT_RANGES)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| v.trim().eq_ignore_ascii_case("bytes"));
    let size_matches = match (total_size(status, headers), expected_size) {
        (Some(total), Some(expected)) => Some(total == expected),
        _ => None,
    };
    RangeReport {
        hops,
        status: status.as_u16(),
        partial: status == StatusCode::PARTIAL_CONTENT && headers.contains_key(CONTENT_RANGE),
        accept_ranges,
        size_matches,
        content: content_kind(headers),
        bytes_read: body.len(),
        ebml_magic: body.starts_with(&EBML_MAGIC),
        elapsed_ms,
    }
}

/// Why the range read stopped. Carries no URL.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RangeError {
    Timeout { hops: Vec<HostKind> },
    Connect { hops: Vec<HostKind> },
    TooManyRedirects { hops: Vec<HostKind> },
    BadRedirect { hops: Vec<HostKind> },
    Other { hops: Vec<HostKind> },
}

pub type RangeOutcome = Result<RangeReport, RangeError>;
pub type RangeFuture<'a> = Pin<Box<dyn Future<Output = RangeOutcome> + Send + 'a>>;

/// Reads the first bytes of a stream URL; faked in tests.
pub trait RangeReader: Send + Sync {
    fn read_head<'a>(
        &'a self,
        url: &'a str,
        source_host: &'a str,
        expected_size: Option<u64>,
    ) -> RangeFuture<'a>;
}

pub struct HttpRangeReader {
    client: reqwest::Client,
}

impl HttpRangeReader {
    pub fn new() -> Option<Self> {
        let client = reqwest::Client::builder()
            .redirect(redirect::Policy::none())
            .timeout(READ_TIMEOUT)
            .user_agent(concat!("stremio-subsync/", env!("CARGO_PKG_VERSION")))
            .build()
            .ok()?;
        Some(Self { client })
    }
}

fn classify(err: &reqwest::Error, hops: Vec<HostKind>) -> RangeError {
    if err.is_timeout() {
        RangeError::Timeout { hops }
    } else if err.is_connect() {
        RangeError::Connect { hops }
    } else {
        RangeError::Other { hops }
    }
}

impl RangeReader for HttpRangeReader {
    fn read_head<'a>(
        &'a self,
        url: &'a str,
        source_host: &'a str,
        expected_size: Option<u64>,
    ) -> RangeFuture<'a> {
        Box::pin(async move {
            let started = Instant::now();
            let mut hops = Vec::new();
            let mut url =
                Url::parse(url).map_err(|_| RangeError::BadRedirect { hops: Vec::new() })?;
            loop {
                hops.push(host_kind(url.host_str().unwrap_or_default(), source_host));
                let mut response = match self
                    .client
                    .get(url.clone())
                    .header(RANGE, format!("bytes=0-{}", HEAD_BYTES - 1))
                    .send()
                    .await
                {
                    Ok(response) => response,
                    Err(err) => return Err(classify(&err, hops)),
                };
                if response.status().is_redirection() {
                    if hops.len() > MAX_REDIRECTS {
                        return Err(RangeError::TooManyRedirects { hops });
                    }
                    let next = response
                        .headers()
                        .get(LOCATION)
                        .and_then(|v| v.to_str().ok())
                        .and_then(|loc| url.join(loc).ok())
                        .filter(|next| matches!(next.scheme(), "http" | "https"));
                    match next {
                        Some(next) => url = next,
                        None => return Err(RangeError::BadRedirect { hops }),
                    }
                    continue;
                }
                let status = response.status();
                let headers = response.headers().clone();
                // Stop at the budget even if the server ignored the range;
                // dropping the response closes the connection.
                let mut body = Vec::with_capacity(HEAD_BYTES);
                while body.len() < HEAD_BYTES {
                    match response.chunk().await {
                        Ok(Some(chunk)) => body.extend_from_slice(&chunk),
                        Ok(None) => break,
                        Err(err) => return Err(classify(&err, hops)),
                    }
                }
                body.truncate(HEAD_BYTES);
                let elapsed_ms = started.elapsed().as_millis();
                return Ok(analyze(
                    hops,
                    status,
                    &headers,
                    &body,
                    expected_size,
                    elapsed_ms,
                ));
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use reqwest::header::HeaderValue;

    fn headers(pairs: &[(&'static str, &'static str)]) -> HeaderMap {
        let mut map = HeaderMap::new();
        for (name, value) in pairs {
            map.insert(*name, HeaderValue::from_static(value));
        }
        map
    }

    #[test]
    fn host_kinds() {
        let source = "aio.test";
        assert_eq!(host_kind("AIO.test", source), HostKind::AioStreams);
        assert_eq!(host_kind("real-debrid.com", source), HostKind::RealDebrid);
        assert_eq!(
            host_kind("47.download.real-debrid.com", source),
            HostKind::RealDebrid
        );
        assert_eq!(host_kind("store-1.tb-cdn.st", source), HostKind::TorBox);
        assert_eq!(host_kind("notreal-debrid.com", source), HostKind::Other);
        assert_eq!(host_kind("example.test", source), HostKind::Other);
    }

    #[test]
    fn partial_answer_from_debrid() {
        let mut body = EBML_MAGIC.to_vec();
        body.resize(HEAD_BYTES, 0);
        let report = analyze(
            vec![HostKind::AioStreams, HostKind::RealDebrid],
            StatusCode::PARTIAL_CONTENT,
            &headers(&[
                ("content-range", "bytes 0-65535/4294967296"),
                ("accept-ranges", "bytes"),
                ("content-type", "video/x-matroska"),
            ]),
            &body,
            Some(4_294_967_296),
            120,
        );
        assert_eq!(
            report,
            RangeReport {
                hops: vec![HostKind::AioStreams, HostKind::RealDebrid],
                status: 206,
                partial: true,
                accept_ranges: true,
                size_matches: Some(true),
                content: ContentKind::Matroska,
                bytes_read: HEAD_BYTES,
                ebml_magic: true,
                elapsed_ms: 120,
            }
        );
    }

    #[test]
    fn range_ignored_uses_content_length() {
        let report = analyze(
            vec![HostKind::Other],
            StatusCode::OK,
            &headers(&[
                ("content-length", "1000"),
                ("content-type", "application/octet-stream; charset=binary"),
            ]),
            b"not mkv",
            Some(999),
            5,
        );
        assert!(!report.partial);
        assert!(!report.accept_ranges);
        assert_eq!(report.size_matches, Some(false));
        assert_eq!(report.content, ContentKind::OctetStream);
        assert!(!report.ebml_magic);
    }

    #[test]
    fn unknown_total_or_size_is_none() {
        let report = analyze(
            vec![HostKind::Other],
            StatusCode::PARTIAL_CONTENT,
            &headers(&[("content-range", "bytes 0-9/*")]),
            b"",
            Some(10),
            1,
        );
        assert_eq!(report.size_matches, None);
        assert_eq!(report.content, ContentKind::Missing);
        let report = analyze(
            vec![],
            StatusCode::FORBIDDEN,
            &HeaderMap::new(),
            b"",
            None,
            1,
        );
        assert_eq!(report.size_matches, None);
        assert_eq!(report.status, 403);
    }
}
