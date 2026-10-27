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
use std::sync::atomic::{AtomicUsize, Ordering};
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

/// Why a later range read failed. Carries no URL.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReadError {
    Timeout,
    Connect,
    Status(u16),
    /// The server answered 200 instead of 206; nothing more is read.
    RangeIgnored,
    /// `stage` is "send" or "body"; `detail` is the error's cause chain with
    /// the URL removed.
    Other {
        stage: &'static str,
        detail: String,
    },
}

pub type ReadFuture<'a> = Pin<Box<dyn Future<Output = Result<Vec<u8>, ReadError>> + Send + 'a>>;

/// Random access to the bytes of one file; faked in tests.
pub trait ByteSource: Send + Sync {
    /// Reads up to `len` bytes starting at `start`; fewer only at end of file.
    fn read(&self, start: u64, len: usize) -> ReadFuture<'_>;

    /// Reads that failed once and were tried again.
    fn retries(&self) -> usize {
        0
    }
}

/// A successful first read: its report, the bytes, and the file for more reads.
/// No `Debug`: the file holds the resolved URL.
pub struct Opened {
    pub report: RangeReport,
    pub head: Vec<u8>,
    pub file: Box<dyn ByteSource>,
}

pub type OpenFuture<'a> = Pin<Box<dyn Future<Output = Result<Opened, RangeError>> + Send + 'a>>;

/// Opens a stream URL by reading its first bytes; faked in tests.
pub trait RangeReader: Send + Sync {
    fn open<'a>(
        &'a self,
        url: &'a str,
        source_host: &'a str,
        expected_size: Option<u64>,
    ) -> OpenFuture<'a>;
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

/// Reads at most `limit` bytes of a body; dropping the response afterwards
/// closes the connection, so a server that ignores the range costs no more.
async fn read_capped(
    response: &mut reqwest::Response,
    limit: usize,
) -> Result<Vec<u8>, reqwest::Error> {
    let mut body = Vec::with_capacity(limit.min(HEAD_BYTES));
    while body.len() < limit {
        match response.chunk().await? {
            Some(chunk) => body.extend_from_slice(&chunk),
            None => break,
        }
    }
    body.truncate(limit);
    Ok(body)
}

impl RangeReader for HttpRangeReader {
    fn open<'a>(
        &'a self,
        url: &'a str,
        source_host: &'a str,
        expected_size: Option<u64>,
    ) -> OpenFuture<'a> {
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
                let head = match read_capped(&mut response, HEAD_BYTES).await {
                    Ok(head) => head,
                    Err(err) => return Err(classify(&err, hops)),
                };
                let elapsed_ms = started.elapsed().as_millis();
                let report = analyze(hops, status, &headers, &head, expected_size, elapsed_ms);
                let file = Box::new(HttpFile {
                    client: self.client.clone(),
                    url,
                    retries: AtomicUsize::new(0),
                });
                return Ok(Opened { report, head, file });
            }
        })
    }
}

/// The final URL after redirects. No `Debug`: it is a playable link.
struct HttpFile {
    client: reqwest::Client,
    url: Url,
    retries: AtomicUsize,
}

impl HttpFile {
    async fn read_once(&self, start: u64, len: usize) -> Result<Vec<u8>, ReadError> {
        let end = start.saturating_add(len as u64 - 1);
        let mut response = self
            .client
            .get(self.url.clone())
            .header(RANGE, format!("bytes={start}-{end}"))
            .send()
            .await
            .map_err(|err| read_error(err, "send"))?;
        match response.status() {
            StatusCode::PARTIAL_CONTENT => {}
            StatusCode::OK => return Err(ReadError::RangeIgnored),
            status => return Err(ReadError::Status(status.as_u16())),
        }
        read_capped(&mut response, len)
            .await
            .map_err(|err| read_error(err, "body"))
    }
}

impl ByteSource for HttpFile {
    fn read(&self, start: u64, len: usize) -> ReadFuture<'_> {
        Box::pin(async move {
            if len == 0 {
                return Ok(Vec::new());
            }
            match self.read_once(start, len).await {
                Err(ReadError::Timeout | ReadError::Connect | ReadError::Other { .. }) => {
                    self.retries.fetch_add(1, Ordering::Relaxed);
                    self.read_once(start, len).await
                }
                result => result,
            }
        })
    }

    fn retries(&self) -> usize {
        self.retries.load(Ordering::Relaxed)
    }
}

const DETAIL_MAX: usize = 160;

/// The cause chain of an error, without any URL, for the log.
fn error_detail(err: reqwest::Error) -> String {
    let err = err.without_url();
    let mut parts = vec![err.to_string()];
    let mut source = std::error::Error::source(&err);
    while let Some(cause) = source {
        parts.push(cause.to_string());
        source = cause.source();
    }
    scrub(&parts.join(": "))
}

/// Replaces anything URL-shaped and caps the length.
fn scrub(text: &str) -> String {
    let cleaned = text
        .split_whitespace()
        .map(|word| if word.contains("://") { "<url>" } else { word })
        .collect::<Vec<_>>()
        .join(" ");
    cleaned.chars().take(DETAIL_MAX).collect()
}

fn read_error(err: reqwest::Error, stage: &'static str) -> ReadError {
    if err.is_timeout() {
        ReadError::Timeout
    } else if err.is_connect() {
        ReadError::Connect
    } else {
        ReadError::Other {
            stage,
            detail: error_detail(err),
        }
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
    fn scrub_removes_urls_and_caps_length() {
        assert_eq!(
            scrub("error sending request for url (https://x.test/secret): connection closed"),
            "error sending request for url <url> connection closed"
        );
        assert_eq!(scrub(&"a ".repeat(200)).chars().count(), DETAIL_MAX);
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
