//! Stremio add-on HTTP surface: manifest and subtitle routes behind a
//! secret path token.

use axum::{
    Json, Router,
    extract::{MatchedPath, Path, Query as UrlQuery, Request, State},
    http::{HeaderMap, StatusCode, header},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::get,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::sync::{Arc, Mutex};
use tower_http::cors::CorsLayer;

use crate::align;
use crate::dump::{self, Dump};
use crate::events::{self, event};
use crate::extra::PlaybackHints;
use crate::label;
use crate::menu::{self, MenuCache, Notice};
use crate::range::RangeReader;
use crate::ruler::Event;
use crate::sources::{self, Query, SubtitleSource};
use crate::srt;
use crate::streams::{self, StreamSource};

/// Shared state for the HTTP handlers.
pub struct AppState {
    token: String,
    streams: Option<Arc<dyn StreamSource>>,
    range: Option<Arc<dyn RangeReader>>,
    /// Read the ruler track's blocks after each K2 probe (SS-028).
    block_probe: bool,
    subtitle_sources: Vec<Arc<dyn SubtitleSource>>,
    /// ISO 639-2 language subtitles are looked for in.
    lang: String,
    /// Where probe timings are saved for calibration, if anywhere.
    dump_dir: Option<std::path::PathBuf>,
    /// Menus already answered: accepted subtitles, or in labeling mode
    /// every candidate.
    menus: Mutex<MenuCache>,
    /// List every candidate, marked, instead of the accepted ones.
    label_mode: bool,
    /// Where answered menus are kept across restarts (not in labeling mode).
    cache_dir: Option<std::path::PathBuf>,
}

impl AppState {
    /// `token` is the secret path segment; it must be non-empty.
    pub fn new(token: String) -> Option<Self> {
        (!token.is_empty()).then_some(Self {
            token,
            streams: None,
            range: None,
            block_probe: false,
            subtitle_sources: Vec::new(),
            lang: String::new(),
            dump_dir: None,
            menus: Mutex::new(MenuCache::default()),
            label_mode: false,
            cache_dir: None,
        })
    }

    /// Enables the K2 probe against this stream list source.
    pub fn with_stream_source(mut self, source: Arc<dyn StreamSource>) -> Self {
        self.streams = Some(source);
        self
    }

    /// Lets the K2 probe read the first bytes of the single matched stream.
    pub fn with_range_reader(mut self, reader: Arc<dyn RangeReader>) -> Self {
        self.range = Some(reader);
        self
    }

    /// After each K2 probe, reads the ruler track's blocks in the background.
    pub fn with_block_probe(mut self) -> Self {
        self.block_probe = true;
        self
    }

    /// Enables the M3 probe: search these sources in `lang`, rank, download.
    pub fn with_subtitle_sources(
        mut self,
        sources: Vec<Arc<dyn SubtitleSource>>,
        lang: String,
    ) -> Self {
        self.subtitle_sources = sources;
        self.lang = lang;
        self
    }

    /// Saves ruler and candidate timings of every aligned request under `dir`.
    pub fn with_dump_dir(mut self, dir: std::path::PathBuf) -> Self {
        self.dump_dir = Some(dir);
        self
    }

    /// Lists every downloaded candidate in Stremio's menu, marked ✓ or ✗.
    pub fn with_label_mode(mut self) -> Self {
        self.label_mode = true;
        self
    }

    /// Keeps answered menus under `dir` so a restart needs no new probes.
    pub fn with_cache_dir(mut self, dir: std::path::PathBuf) -> Self {
        self.cache_dir = Some(dir);
        self
    }

    fn authorized(&self, candidate: &str) -> bool {
        constant_time_eq(self.token.as_bytes(), candidate.as_bytes())
    }
}

fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

#[derive(Serialize)]
struct SubtitlesResponse {
    subtitles: Vec<Value>,
}

pub fn router(state: AppState) -> Router {
    let state = Arc::new(state);
    Router::new()
        .route("/{token}/manifest.json", get(manifest))
        .route("/{token}/subtitles/{kind}/{id}", get(subtitles))
        .route(
            "/{token}/subtitles/{kind}/{id}/{extra}",
            get(subtitles_with_extra),
        )
        .route("/{token}/sub/{entry}/{file}", get(menu_srt))
        .route("/{token}/dashboard", get(dashboard))
        .route(EVENTS_ROUTE, get(events_json))
        .layer(middleware::from_fn_with_state(state.clone(), log_access))
        .layer(CorsLayer::permissive())
        .with_state(state)
}

const EVENTS_ROUTE: &str = "/{token}/events.json";
const DASHBOARD_HTML: &str = include_str!("dashboard.html");

/// Live view of recent log lines (SS-029), behind the same token.
async fn dashboard(State(state): State<Arc<AppState>>, Path(token): Path<String>) -> Response {
    if !state.authorized(&token) {
        return StatusCode::NOT_FOUND.into_response();
    }
    (
        [
            (header::CONTENT_TYPE, "text/html; charset=utf-8"),
            (header::CACHE_CONTROL, "no-store"),
        ],
        DASHBOARD_HTML,
    )
        .into_response()
}

#[derive(Deserialize)]
struct After {
    after: Option<u64>,
}

async fn events_json(
    State(state): State<Arc<AppState>>,
    Path(token): Path<String>,
    UrlQuery(query): UrlQuery<After>,
) -> Response {
    if !state.authorized(&token) {
        return StatusCode::NOT_FOUND.into_response();
    }
    (
        [(header::CACHE_CONTROL, "no-store")],
        Json(events::since(query.after.unwrap_or(0))),
    )
        .into_response()
}

/// One line per request with the route template, never the path itself:
/// the path carries the token, the IMDb id and the filename.
fn access_line(method: &str, route: Option<&str>, token_ok: bool, status: StatusCode) -> String {
    let route = route.unwrap_or("none");
    let token = if token_ok { "ok" } else { "bad" };
    format!(
        "http {method} route={route} token={token} status={}",
        status.as_u16()
    )
}

async fn log_access(
    State(state): State<Arc<AppState>>,
    matched: Option<MatchedPath>,
    request: Request,
    next: Next,
) -> Response {
    let method = request.method().clone();
    let first_segment = request.uri().path().trim_start_matches('/');
    let first_segment = first_segment.split('/').next().unwrap_or_default();
    let token_ok = state.authorized(first_segment);
    let response = next.run(request).await;
    let route = matched.as_ref().map(MatchedPath::as_str);
    // The dashboard polls every few seconds; logging that would bury the rest.
    if route == Some(EVENTS_ROUTE) && response.status().is_success() {
        return response;
    }
    event!(
        "http",
        "{}",
        access_line(method.as_str(), route, token_ok, response.status())
    );
    response
}

pub fn manifest_json() -> Value {
    json!({
        "id": "local.stremio-subsync",
        "version": env!("CARGO_PKG_VERSION"),
        "name": "SubSync",
        "description": "Altyazıları oynatılan dosyanın gömülü İngilizce izine göre hizalar.",
        "resources": ["subtitles"],
        "types": ["movie", "series"],
        "idPrefixes": ["tt"],
        "catalogs": [],
        "behaviorHints": { "configurable": false, "configurationRequired": false }
    })
}

async fn manifest(State(state): State<Arc<AppState>>, Path(token): Path<String>) -> Response {
    if !state.authorized(&token) {
        return StatusCode::NOT_FOUND.into_response();
    }
    Json(manifest_json()).into_response()
}

async fn subtitles(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path((token, kind, id)): Path<(String, String, String)>,
) -> Response {
    respond(
        state,
        &headers,
        &token,
        &kind,
        &id,
        PlaybackHints::default(),
    )
    .await
}

async fn subtitles_with_extra(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path((token, kind, id, extra)): Path<(String, String, String, String)>,
) -> Response {
    let hints = PlaybackHints::parse(&extra);
    respond(state, &headers, &token, &kind, &id, hints).await
}

async fn respond(
    state: Arc<AppState>,
    headers: &HeaderMap,
    token: &str,
    kind: &str,
    id: &str,
    hints: PlaybackHints,
) -> Response {
    if !state.authorized(token) {
        return StatusCode::NOT_FOUND.into_response();
    }
    let id = id.strip_suffix(".json").unwrap_or(id);
    if !matches!(kind, "movie" | "series") || !id.starts_with("tt") {
        return StatusCode::NOT_FOUND.into_response();
    }
    // Only field presence is logged; ids and filenames reveal what is watched.
    event!("request", "subtitles request kind={kind} hints={hints:?}");
    // Kept in memory only, to find this request's menu again; on disk only
    // its hash is used.
    let request = format!(
        "{kind}/{id}/{}/{}",
        hints.filename.as_deref().unwrap_or_default(),
        hints.video_size.unwrap_or_default()
    );
    let now = menu::now_s();
    if let Some((list, offered)) = menu_list(&state, headers, token, &request, now) {
        event!("served", "subtitles served from=memory offered={offered}");
        return list;
    }
    if !state.label_mode
        && let Some(dir) = state.cache_dir.clone()
    {
        let key = request.clone();
        if let Ok(Some((created, keep_s, entries))) =
            tokio::task::spawn_blocking(move || menu::load(&dir, &key, now)).await
        {
            if let Ok(mut menus) = state.menus.lock() {
                menus.insert(request.clone(), entries, created, keep_s);
            }
            if let Some((list, offered)) = menu_list(&state, headers, token, &request, now) {
                event!("served", "subtitles served from=disk offered={offered}");
                return list;
            }
        }
    }
    if state.streams.is_some() || !state.subtitle_sources.is_empty() {
        let work = tokio::spawn(run_probes(
            state.clone(),
            kind.to_owned(),
            id.to_owned(),
            hints,
            request.clone(),
        ));
        // Stremio waits for the menu; give the probes time to fill it. If
        // they take longer they still finish, and the next open is served
        // from the cache.
        if tokio::time::timeout(MENU_WAIT, work).await.is_err() {
            // Say so instead of an empty menu; the probes still finish and
            // replace this notice.
            let notice = Notice::NotReady;
            if let Ok(mut menus) = state.menus.lock()
                && menus.find(&request, now).is_none()
            {
                menus.insert(request.clone(), vec![notice.entry()], now, notice.keep_s());
            }
        }
        if let Some((list, offered)) = menu_list(&state, headers, token, &request, now) {
            event!("served", "subtitles served from=probes offered={offered}");
            return list;
        }
    }
    event!("served", "subtitles served from=none offered=0");
    Json(SubtitlesResponse {
        subtitles: Vec::new(),
    })
    .into_response()
}

/// How long a request waits for the probes before answering without them.
const MENU_WAIT: std::time::Duration = std::time::Duration::from_secs(15);

/// The menu of `request` as a Stremio subtitle list, and its length.
fn menu_list(
    state: &AppState,
    headers: &HeaderMap,
    token: &str,
    request: &str,
    now: u64,
) -> Option<(Response, usize)> {
    let menus = state.menus.lock().ok()?;
    let (entry, entries) = menus.find(request, now)?;
    let host = headers.get(header::HOST)?.to_str().ok()?;
    let scheme = headers
        .get("x-forwarded-proto")
        .and_then(|v| v.to_str().ok())
        .filter(|p| *p == "https")
        .unwrap_or("http");
    let subtitles: Vec<Value> = entries
        .iter()
        .enumerate()
        .map(|(i, offered)| {
            let n = i + 1;
            json!({
                "id": format!("subsync-{entry}-{n}"),
                "url": format!("{scheme}://{host}/{token}/sub/{entry}/{n}.srt"),
                "lang": offered.lang,
            })
        })
        .collect();
    let offered = subtitles.len();
    Some((
        Json(SubtitlesResponse { subtitles }).into_response(),
        offered,
    ))
}

async fn menu_srt(
    State(state): State<Arc<AppState>>,
    Path((token, entry, file)): Path<(String, u64, String)>,
) -> Response {
    if !state.authorized(&token) {
        return StatusCode::NOT_FOUND.into_response();
    }
    let Some(n) = file.strip_suffix(".srt").and_then(|n| n.parse().ok()) else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let srt = state
        .menus
        .lock()
        .ok()
        .and_then(|menus| menus.srt(entry, n).map(str::to_owned));
    match srt {
        Some(srt) => (
            [(header::CONTENT_TYPE, "application/x-subrip; charset=utf-8")],
            srt,
        )
            .into_response(),
        None => StatusCode::NOT_FOUND.into_response(),
    }
}

/// Probes for `request` and stores the answer: the subtitles to offer, or a
/// notice saying why there are none.
async fn run_probes(
    state: Arc<AppState>,
    kind: String,
    id: String,
    hints: PlaybackHints,
    request: String,
) {
    let now = menu::now_s();
    let (entries, keep_s, notice) = match build_menu(&state, kind, id, hints).await {
        Ok(entries) if !entries.is_empty() => (entries, menu::KEEP_FOUND_S, None),
        Ok(_) => {
            let notice = Notice::NoneAccepted;
            (vec![notice.entry()], notice.keep_s(), Some(notice))
        }
        Err(notice) => (vec![notice.entry()], notice.keep_s(), Some(notice)),
    };
    event!(
        "menu",
        "menu ready entries={} notice={notice:?} label_mode={}",
        entries.len(),
        state.label_mode
    );
    // Passing answers and labeling menus stay in memory only.
    if !state.label_mode
        && keep_s >= menu::KEEP_NONE_S
        && let Some(dir) = state.cache_dir.clone()
    {
        let (key, saved) = (request.clone(), entries.clone());
        let written =
            tokio::task::spawn_blocking(move || menu::save(&dir, &key, &saved, now, keep_s)).await;
        if let Ok(Err(err)) = written {
            event!("cache", "cache write failed error={:?}", err.kind());
        }
    }
    if let Ok(mut menus) = state.menus.lock() {
        menus.insert(request, entries, now, keep_s);
    }
}

/// The ruler tracks of a K2 probe, or why there are none.
fn rulers_of(k2: Option<streams::ProbeOutcome>) -> Result<Vec<(u64, Vec<Event>)>, Notice> {
    let (report, range, mkv) = match k2 {
        Some(streams::ProbeOutcome::Matched {
            report, range, mkv, ..
        }) => (report, range, mkv),
        Some(streams::ProbeOutcome::NoHints) => return Err(Notice::NoStreamMatch),
        Some(streams::ProbeOutcome::Failed { .. }) | None => return Err(Notice::Failed),
    };
    if report.matched_by.is_none() {
        return Err(Notice::NoStreamMatch);
    }
    match (range, mkv) {
        (_, Some(mkv)) => match *mkv {
            Ok(report) if !report.rulers.0.is_empty() => Ok(report.rulers.0),
            Ok(report) if report.english_text_indexed == Some(false) => Err(Notice::NotIndexed),
            Ok(report) if report.subtitles.iter().any(|s| s.english) => Err(Notice::ImageOnly),
            Ok(_) => Err(Notice::NoEnglishTrack),
            Err(_) => Err(Notice::Failed),
        },
        (Some(Ok(range)), None) if !range.ebml_magic => Err(Notice::NotMkv),
        _ => Err(Notice::Failed),
    }
}

/// Saves English PGS cue times under `<dump dir>/pgs` (SS-025), whether or
/// not the file has a text ruler. Only Cues already read are used, so this
/// adds no media requests.
async fn save_pgs(state: &AppState, kind: &str, k2: Option<&streams::ProbeOutcome>) {
    let Some(dir) = state.dump_dir.as_ref().map(|dir| dir.join("pgs")) else {
        return;
    };
    let Some(streams::ProbeOutcome::Matched { mkv: Some(mkv), .. }) = k2 else {
        return;
    };
    let Ok(report) = &**mkv else {
        return;
    };
    if report.pgs.is_empty() {
        return;
    }
    let mut saved = Dump::new(kind, &report.rulers.0, &[], &[]);
    saved.pgs = report.pgs.clone();
    match tokio::task::spawn_blocking(move || dump::write(&dir, &saved)).await {
        Ok(Ok(name)) => event!("dump", "pgs dump written file={name}"),
        Ok(Err(err)) => event!("dump", "pgs dump failed error={:?}", err.kind()),
        Err(_) => event!("dump", "pgs dump failed error=task"),
    }
}

/// K2 and M3 probes side by side, then alignment, the timing dump and the
/// menu: accepted subtitles, or in labeling mode every candidate, marked.
/// A missing ruler answers at once, without waiting for M3.
async fn build_menu(
    state: &Arc<AppState>,
    kind: String,
    id: String,
    hints: PlaybackHints,
) -> Result<Vec<menu::MenuEntry>, Notice> {
    let query = Query::new(&kind, &id, &state.lang);
    let m3 = match query {
        Some(query) if !state.subtitle_sources.is_empty() => {
            let sources = state.subtitle_sources.clone();
            let filename = hints.filename.clone();
            Some(tokio::spawn(async move {
                let (outcome, cues) = sources::probe(&sources, &query, filename.as_deref()).await;
                event!("m3", "m3 probe query={query:?} outcome={outcome:?}");
                (outcome, cues)
            }))
        }
        _ => None,
    };
    let k2 = match state.streams.clone() {
        Some(source) => {
            let outcome = streams::probe(
                source.as_ref(),
                state.range.as_deref(),
                state.block_probe,
                &kind,
                &id,
                &hints,
            )
            .await;
            event!("k2", "k2 probe kind={kind} outcome={outcome:?}");
            Some(outcome)
        }
        None => None,
    };
    save_pgs(state, &kind, k2.as_ref()).await;
    let rulers = match rulers_of(k2) {
        Ok(rulers) => rulers,
        Err(notice) => {
            if let Some(m3) = m3 {
                m3.abort();
            }
            return Err(notice);
        }
    };
    let (outcome, cues) = match m3 {
        Some(m3) => m3.await.map_err(|_| Notice::Failed)?,
        None => return Err(Notice::Failed),
    };
    if outcome.searches.iter().all(|(_, found)| found.is_err()) {
        return Err(Notice::Failed);
    }
    if outcome
        .searches
        .iter()
        .all(|(_, found)| matches!(found, Ok(0) | Err(_)))
    {
        return Err(Notice::NoCandidates);
    }
    let scores: Vec<Option<i32>> = outcome
        .downloads
        .iter()
        .map(|d| d.as_ref().ok().map(|stats| stats.score))
        .collect();
    let times: Vec<Option<Vec<Event>>> =
        cues.iter().map(|c| c.as_deref().map(srt::events)).collect();
    if times.iter().all(Option::is_none) {
        return Err(Notice::Failed);
    }
    // Alignment, the dump and the menu are CPU and disk work; keep them off
    // the async workers.
    let dump_dir = state.dump_dir.clone();
    let label_mode = state.label_mode;
    let lang = state.lang.clone();
    let (trials, entries) = tokio::task::spawn_blocking(move || {
        let dump_name = match dump_dir
            .map(|dir| dump::write(&dir, &Dump::new(&kind, &rulers, &times, &scores)))
        {
            Some(Ok(name)) => {
                event!("dump", "dump written file={name}");
                Some(name)
            }
            Some(Err(err)) => {
                event!("dump", "dump failed error={:?}", err.kind());
                None
            }
            None => None,
        };
        let trials = align::trials(&rulers, &times);
        let (track, events) = &rulers[0];
        let entries = if label_mode {
            label::build(&trials, (*track, events), &cues, dump_name.as_deref())
        } else {
            menu::accepted(&trials, &cues, &lang)
        };
        (trials, entries)
    })
    .await
    .map_err(|_| Notice::Failed)?;
    for trial in &trials {
        event!("m4", "m4 probe {trial:?}");
    }
    Ok(entries)
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::Request;
    use http_body_util::BodyExt;
    use tower::ServiceExt;

    const TOKEN: &str = "test-token-123";

    fn app() -> Router {
        router(AppState::new(TOKEN.to_owned()).unwrap())
    }

    async fn get(uri: &str) -> (StatusCode, Option<Value>) {
        let response = app()
            .oneshot(Request::get(uri).body(Body::empty()).unwrap())
            .await
            .unwrap();
        let status = response.status();
        let bytes = response.into_body().collect().await.unwrap().to_bytes();
        (status, serde_json::from_slice(&bytes).ok())
    }

    #[tokio::test]
    async fn manifest_declares_subtitles_only() {
        let (status, body) = get(&format!("/{TOKEN}/manifest.json")).await;
        assert_eq!(status, StatusCode::OK);
        let body = body.unwrap();
        assert_eq!(body["resources"], json!(["subtitles"]));
        assert_eq!(body["types"], json!(["movie", "series"]));
    }

    #[tokio::test]
    async fn subtitles_route_accepts_extra_segment() {
        let uri =
            format!("/{TOKEN}/subtitles/series/tt0944947:1:2/filename=A.mkv&videoSize=10.json");
        let (status, body) = get(&uri).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body.unwrap(), json!({ "subtitles": [] }));
    }

    #[tokio::test]
    async fn subtitles_route_without_extra() {
        let (status, _) = get(&format!("/{TOKEN}/subtitles/movie/tt0111161.json")).await;
        assert_eq!(status, StatusCode::OK);
    }

    #[tokio::test]
    async fn wrong_token_is_not_found() {
        for uri in [
            "/wrong-token-123/manifest.json",
            "/wrong-token-123/subtitles/movie/tt0111161.json",
            "/test-token-12/manifest.json",
        ] {
            let (status, _) = get(uri).await;
            assert_eq!(status, StatusCode::NOT_FOUND, "{uri}");
        }
    }

    #[tokio::test]
    async fn dashboard_and_events_need_the_token() {
        let response = app()
            .oneshot(
                Request::get(format!("/{TOKEN}/dashboard"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            response.headers()[header::CONTENT_TYPE],
            "text/html; charset=utf-8"
        );
        let html = response.into_body().collect().await.unwrap().to_bytes();
        let html = String::from_utf8_lossy(&html);
        // Relative fetch: the page never holds the token itself.
        assert!(html.contains(r#"fetch("events.json?after="#), "{html}");
        assert!(!html.contains(TOKEN));

        event!("request", "dashboard test line");
        let (status, body) = get(&format!("/{TOKEN}/events.json?after=0")).await;
        assert_eq!(status, StatusCode::OK);
        let body = body.unwrap();
        assert!(body["next"].as_u64().unwrap() >= 1);
        assert!(body["started_ms"].as_u64().unwrap() > 0);
        let next = body["next"].as_u64().unwrap();
        let (_, newer) = get(&format!("/{TOKEN}/events.json?after={next}")).await;
        assert!(
            newer.unwrap()["events"]
                .as_array()
                .unwrap()
                .iter()
                .all(|e| e["seq"].as_u64().unwrap() > next)
        );

        for uri in ["/wrong-token-123/dashboard", "/wrong-token-123/events.json"] {
            let (status, _) = get(uri).await;
            assert_eq!(status, StatusCode::NOT_FOUND, "{uri}");
        }
    }

    #[tokio::test]
    async fn unsupported_type_or_id_is_not_found() {
        for uri in [
            format!("/{TOKEN}/subtitles/tv/tt0111161.json"),
            format!("/{TOKEN}/subtitles/movie/kitsu:1.json"),
        ] {
            let (status, _) = get(&uri).await;
            assert_eq!(status, StatusCode::NOT_FOUND, "{uri}");
        }
    }

    struct SignalSource(std::sync::Mutex<Option<tokio::sync::oneshot::Sender<String>>>);

    impl StreamSource for SignalSource {
        fn fetch<'a>(&'a self, kind: &'a str, id: &'a str) -> streams::FetchFuture<'a> {
            if let Some(tx) = self.0.lock().unwrap().take() {
                let _ = tx.send(format!("{kind}/{id}"));
            }
            Box::pin(async { Ok(Vec::new()) })
        }

        fn host(&self) -> &str {
            "aio.test"
        }
    }

    #[tokio::test]
    async fn subtitles_request_triggers_stream_probe() {
        let (tx, rx) = tokio::sync::oneshot::channel();
        let source = Arc::new(SignalSource(std::sync::Mutex::new(Some(tx))));
        let app = router(
            AppState::new(TOKEN.to_owned())
                .unwrap()
                .with_stream_source(source),
        );
        let uri = format!("/{TOKEN}/subtitles/series/tt0944947:1:2/filename=A.mkv.json");
        let response = app
            .oneshot(Request::get(uri).body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let called = tokio::time::timeout(std::time::Duration::from_secs(1), rx)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(called, "series/tt0944947:1:2");
    }

    struct SignalSubtitles(std::sync::Mutex<Option<tokio::sync::oneshot::Sender<Query>>>);

    impl SubtitleSource for SignalSubtitles {
        fn kind(&self) -> sources::SourceKind {
            sources::SourceKind::OpenSubtitles
        }

        fn search<'a>(&'a self, query: &'a Query) -> sources::SearchFuture<'a> {
            if let Some(tx) = self.0.lock().unwrap().take() {
                let _ = tx.send(query.clone());
            }
            Box::pin(async { Ok(Vec::new()) })
        }

        fn download<'a>(
            &'a self,
            _query: &'a Query,
            _candidate: &'a sources::Candidate,
        ) -> sources::DownloadFuture<'a> {
            Box::pin(async { Err(sources::SourceError::Other) })
        }
    }

    #[tokio::test]
    async fn missing_ruler_answers_at_once_with_a_notice() {
        // The stream list has no match for the file, so there is no ruler.
        let (streams_tx, _streams_rx) = tokio::sync::oneshot::channel();
        let (search_tx, _search_rx) = tokio::sync::oneshot::channel();
        let subtitles: Arc<dyn SubtitleSource> =
            Arc::new(SignalSubtitles(std::sync::Mutex::new(Some(search_tx))));
        let app = router(
            AppState::new(TOKEN.to_owned())
                .unwrap()
                .with_stream_source(Arc::new(SignalSource(std::sync::Mutex::new(Some(
                    streams_tx,
                )))))
                .with_subtitle_sources(vec![subtitles], "tur".to_owned()),
        );
        let uri = format!("/{TOKEN}/subtitles/series/tt0944947:1:2/filename=A.mkv.json");
        let started = std::time::Instant::now();
        let (status, body) = call(&app, uri.clone()).await;
        assert_eq!(status, StatusCode::OK);
        assert!(started.elapsed() < std::time::Duration::from_secs(2));
        let list: Value = serde_json::from_str(&body).unwrap();
        assert_eq!(list["subtitles"][0]["lang"], "⚠ Dosya yok");
        let path = list["subtitles"][0]["url"]
            .as_str()
            .unwrap()
            .trim_start_matches("http://127.0.0.1:7000")
            .to_owned();
        let (status, srt) = call(&app, path).await;
        assert_eq!(status, StatusCode::OK);
        assert!(srt.contains("AIOStreams listesinde bulunamadı"), "{srt}");
        // The reason lasts a day: the next open is answered from memory.
        assert_eq!(call(&app, uri).await.1, body);
    }

    /// All metadata is in the head; the only later read can be the block probe.
    /// With `pgs`, the file is image-only: one English PGS track, indexed.
    fn probe_head(pgs: bool) -> Vec<u8> {
        fn el(id: u32, data: &[u8]) -> Vec<u8> {
            let bytes = id.to_be_bytes();
            let skip = bytes.iter().take_while(|&&b| b == 0).count();
            let mut out = bytes[skip..].to_vec();
            out.push(1);
            out.extend_from_slice(&(data.len() as u64).to_be_bytes()[1..]);
            out.extend_from_slice(data);
            out
        }
        let codec: &[u8] = if pgs { b"S_HDMV/PGS" } else { b"S_TEXT/UTF8" };
        let track = el(
            0xAE,
            &[
                el(0xD7, &[2]),
                el(0x83, &[17]),
                el(0x86, codec),
                el(0x22B59C, b"eng"),
            ]
            .concat(),
        );
        let cue_at = |time: u8| {
            el(
                0xBB,
                &[
                    el(0xB3, &[time]),
                    el(
                        0xB7,
                        &[
                            el(0xF7, &[2]),
                            el(0xB2, &[100]),
                            el(0xF1, &[0]),
                            el(0xF0, &[0]),
                        ]
                        .concat(),
                    ),
                ]
                .concat(),
            )
        };
        let cue = if pgs {
            [cue_at(10), cue_at(20), cue_at(20)].concat()
        } else {
            cue_at(0)
        };
        [
            el(0x1A45DFA3, &el(0x4282, b"matroska")),
            el(
                0x18538067,
                &[el(0x1654AE6B, &track), el(0x1C53BB6B, &cue)].concat(),
            ),
        ]
        .concat()
    }

    struct ProbeStream;

    impl StreamSource for ProbeStream {
        fn fetch<'a>(&'a self, _kind: &'a str, _id: &'a str) -> streams::FetchFuture<'a> {
            Box::pin(async {
                streams::parse_stream_list(
                    br#"{"streams":[{"url":"https://media.test/file","behaviorHints":{"filename":"A.mkv","videoSize":5}}]}"#,
                )
            })
        }

        fn host(&self) -> &str {
            "aio.test"
        }
    }

    struct PendingBlocks(Arc<tokio::sync::Notify>);

    impl crate::range::ByteSource for PendingBlocks {
        fn read(&self, _start: u64, _len: usize) -> crate::range::ReadFuture<'_> {
            self.0.notify_one();
            Box::pin(std::future::pending())
        }
    }

    /// Serves `probe_head`; `.1` makes it the image-only file.
    struct ProbeRange(Arc<tokio::sync::Notify>, bool);

    impl RangeReader for ProbeRange {
        fn open<'a>(
            &'a self,
            _url: &'a str,
            _host: &'a str,
            _size: Option<u64>,
        ) -> crate::range::OpenFuture<'a> {
            let head = probe_head(self.1);
            let mut headers = HeaderMap::new();
            headers.insert(
                header::CONTENT_RANGE,
                format!("bytes 0-{}/{}", head.len() - 1, head.len())
                    .parse()
                    .unwrap(),
            );
            let report = crate::range::analyze(
                Vec::new(),
                StatusCode::PARTIAL_CONTENT,
                &headers,
                &head,
                None,
                0,
            );
            let file = Box::new(PendingBlocks(self.0.clone()));
            Box::pin(async { Ok(crate::range::Opened { report, head, file }) })
        }
    }

    #[tokio::test]
    async fn block_probe_does_not_delay_or_replace_cached_failure_notice() {
        let read = Arc::new(tokio::sync::Notify::new());
        let app = router(
            AppState::new(TOKEN.to_owned())
                .unwrap()
                .with_stream_source(Arc::new(ProbeStream))
                .with_range_reader(Arc::new(ProbeRange(read.clone(), false)))
                .with_subtitle_sources(
                    vec![Arc::new(SignalSubtitles(Mutex::new(None)))],
                    "tur".to_owned(),
                )
                .with_block_probe(),
        );
        let uri = format!("/{TOKEN}/subtitles/movie/tt1/filename=A.mkv&videoSize=5.json");
        // Block reads never finish; delivery must still answer and cache its reason.
        let (status, body) =
            tokio::time::timeout(std::time::Duration::from_secs(1), call(&app, uri.clone()))
                .await
                .unwrap();
        assert_eq!(status, StatusCode::OK);
        let list: Value = serde_json::from_str(&body).unwrap();
        assert_eq!(list["subtitles"][0]["lang"], "⚠ Aday yok");
        tokio::time::timeout(std::time::Duration::from_secs(1), read.notified())
            .await
            .expect("the request must also trigger the background block probe");
        assert_eq!(call(&app, uri).await.1, body);
    }

    #[tokio::test]
    async fn image_only_file_saves_pgs_times_and_says_why() {
        let dir = std::env::temp_dir().join(format!(
            "subsync-pgs-dump-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let app = router(
            AppState::new(TOKEN.to_owned())
                .unwrap()
                .with_stream_source(Arc::new(ProbeStream))
                .with_range_reader(Arc::new(ProbeRange(
                    Arc::new(tokio::sync::Notify::new()),
                    true,
                )))
                .with_subtitle_sources(
                    vec![Arc::new(SignalSubtitles(Mutex::new(None)))],
                    "tur".to_owned(),
                )
                .with_dump_dir(dir.clone()),
        );
        let uri = format!("/{TOKEN}/subtitles/movie/tt1/filename=A.mkv&videoSize=5.json");
        let (status, body) = call(&app, uri).await;
        assert_eq!(status, StatusCode::OK);
        let list: Value = serde_json::from_str(&body).unwrap();
        assert_eq!(list["subtitles"][0]["lang"], "⚠ İng. resim");
        let saved: Vec<_> = std::fs::read_dir(dir.join("pgs"))
            .unwrap()
            .map(|e| e.unwrap().path())
            .collect();
        assert_eq!(saved.len(), 1);
        let json = std::fs::read_to_string(&saved[0]).unwrap();
        assert_eq!(
            json,
            r#"{"kind":"movie","rulers":[],"candidates":[],"pgs":[{"track":2,"forced":false,"times":[10,20,20]}]}"#
        );
        // No alignment dump: there was nothing to align.
        assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 1);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    async fn call(app: &Router, uri: String) -> (StatusCode, String) {
        let request = Request::get(uri)
            .header(header::HOST, "127.0.0.1:7000")
            .body(Body::empty())
            .unwrap();
        let response = app.clone().oneshot(request).await.unwrap();
        let status = response.status();
        let body = response.into_body().collect().await.unwrap().to_bytes();
        (status, String::from_utf8(body.to_vec()).unwrap())
    }

    const SRT: &str = "1\n00:00:01,000 --> 00:00:02,000\nx\n\n";

    fn offered() -> Vec<menu::MenuEntry> {
        vec![menu::MenuEntry {
            lang: "tur".to_owned(),
            srt: SRT.to_owned(),
        }]
    }

    #[tokio::test]
    async fn cached_menu_is_listed_and_served() {
        let state = AppState::new(TOKEN.to_owned()).unwrap();
        state.menus.lock().unwrap().insert(
            "movie/tt1/A.mkv/0".to_owned(),
            offered(),
            menu::now_s(),
            menu::KEEP_FOUND_S,
        );
        let app = router(state);

        let (status, body) = call(
            &app,
            format!("/{TOKEN}/subtitles/movie/tt1/filename=A.mkv.json"),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        let list: Value = serde_json::from_str(&body).unwrap();
        assert_eq!(
            list["subtitles"][0],
            json!({
                "id": "subsync-0-1",
                "url": format!("http://127.0.0.1:7000/{TOKEN}/sub/0/1.srt"),
                "lang": "tur",
            })
        );

        let (status, body) = call(&app, format!("/{TOKEN}/sub/0/1.srt")).await;
        assert_eq!((status, body.as_str()), (StatusCode::OK, SRT));
        for uri in [
            "/wrong-token/sub/0/1.srt".to_owned(),
            format!("/{TOKEN}/sub/0/2.srt"),
            format!("/{TOKEN}/sub/9/1.srt"),
            format!("/{TOKEN}/sub/0/one.srt"),
        ] {
            assert_eq!(
                call(&app, uri.clone()).await.0,
                StatusCode::NOT_FOUND,
                "{uri}"
            );
        }
    }

    #[tokio::test]
    async fn disk_cache_answers_without_probes() {
        let dir = std::env::temp_dir().join(format!(
            "subsync-addon-cache-{}-{}",
            std::process::id(),
            menu::now_s()
        ));
        menu::save(
            &dir,
            "series/tt2:1:3/B.mkv/7",
            &offered(),
            menu::now_s(),
            menu::KEEP_FOUND_S,
        )
        .unwrap();
        // No sources at all: only the cache can answer.
        let app = router(
            AppState::new(TOKEN.to_owned())
                .unwrap()
                .with_cache_dir(dir.clone()),
        );
        let (status, body) = call(
            &app,
            format!("/{TOKEN}/subtitles/series/tt2:1:3/filename=B.mkv&videoSize=7.json"),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        let list: Value = serde_json::from_str(&body).unwrap();
        let url = list["subtitles"][0]["url"].as_str().unwrap().to_owned();
        let path = url.trim_start_matches("http://127.0.0.1:7000");
        assert_eq!(
            call(&app, path.to_owned()).await,
            (StatusCode::OK, SRT.to_owned())
        );

        // Another file of the same episode is not this cache entry.
        let (_, body) = call(
            &app,
            format!("/{TOKEN}/subtitles/series/tt2:1:3/filename=C.mkv&videoSize=7.json"),
        )
        .await;
        assert_eq!(body, r#"{"subtitles":[]}"#);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn access_line_shows_template_not_values() {
        let line = access_line(
            "GET",
            Some("/{token}/subtitles/{kind}/{id}/{extra}"),
            false,
            StatusCode::NOT_FOUND,
        );
        assert_eq!(
            line,
            "http GET route=/{token}/subtitles/{kind}/{id}/{extra} token=bad status=404"
        );
        assert_eq!(
            access_line("GET", None, true, StatusCode::NOT_FOUND),
            "http GET route=none token=ok status=404"
        );
    }

    #[test]
    fn empty_token_is_rejected() {
        assert!(AppState::new(String::new()).is_none());
    }
}
