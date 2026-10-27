//! Stremio add-on HTTP surface: manifest and subtitle routes behind a
//! secret path token.

use axum::{
    Json, Router,
    extract::{MatchedPath, Path, Request, State},
    http::{HeaderMap, StatusCode, header},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::get,
};
use serde::Serialize;
use serde_json::{Value, json};
use std::sync::{Arc, Mutex};
use tower_http::cors::CorsLayer;

use crate::align;
use crate::dump::{self, Dump};
use crate::extra::PlaybackHints;
use crate::label;
use crate::menu::{self, MenuCache};
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
        .layer(middleware::from_fn_with_state(state.clone(), log_access))
        .layer(CorsLayer::permissive())
        .with_state(state)
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
    eprintln!(
        "{}",
        access_line(
            method.as_str(),
            matched.as_ref().map(MatchedPath::as_str),
            token_ok,
            response.status()
        )
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
    eprintln!("subtitles request kind={kind} hints={hints:?}");
    // Kept in memory only, to find this request's menu again; on disk only
    // its hash is used.
    let request = format!(
        "{kind}/{id}/{}/{}",
        hints.filename.as_deref().unwrap_or_default(),
        hints.video_size.unwrap_or_default()
    );
    let now = menu::now_s();
    if let Some((list, offered)) = menu_list(&state, headers, token, &request, now) {
        eprintln!("subtitles served from=memory offered={offered}");
        return list;
    }
    if !state.label_mode
        && let Some(dir) = state.cache_dir.clone()
    {
        let key = request.clone();
        if let Ok(Some((created, entries))) =
            tokio::task::spawn_blocking(move || menu::load(&dir, &key, now)).await
        {
            if let Ok(mut menus) = state.menus.lock() {
                menus.insert(request.clone(), entries, created);
            }
            if let Some((list, offered)) = menu_list(&state, headers, token, &request, now) {
                eprintln!("subtitles served from=disk offered={offered}");
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
        let _ = tokio::time::timeout(MENU_WAIT, work).await;
        if let Some((list, offered)) = menu_list(&state, headers, token, &request, now) {
            eprintln!("subtitles served from=probes offered={offered}");
            return list;
        }
    }
    eprintln!("subtitles served from=none offered=0");
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

/// K2 and M3 probes side by side, then alignment, the timing dump and, in
/// labeling mode, the menu entries for `request`.
async fn run_probes(
    state: Arc<AppState>,
    kind: String,
    id: String,
    hints: PlaybackHints,
    request: String,
) {
    let query = Query::new(&kind, &id, &state.lang);
    let filename = hints.filename.clone();
    let k2 = async {
        let source = state.streams.clone()?;
        let outcome =
            streams::probe(source.as_ref(), state.range.as_deref(), &kind, &id, &hints).await;
        eprintln!("k2 probe kind={kind} outcome={outcome:?}");
        Some(outcome)
    };
    let m3 = async {
        let query = query.as_ref()?;
        if state.subtitle_sources.is_empty() {
            return None;
        }
        let (outcome, cues) =
            sources::probe(&state.subtitle_sources, query, filename.as_deref()).await;
        eprintln!("m3 probe query={query:?} outcome={outcome:?}");
        let scores: Vec<Option<i32>> = outcome
            .downloads
            .iter()
            .map(|d| d.as_ref().ok().map(|stats| stats.score))
            .collect();
        Some((cues, scores))
    };
    let (k2, m3) = tokio::join!(k2, m3);
    let rulers = match k2 {
        Some(streams::ProbeOutcome::Matched { mkv: Some(mkv), .. }) => {
            (*mkv).ok().map(|report| report.rulers.0)
        }
        _ => None,
    };
    let (Some(rulers), Some((cues, scores))) = (rulers, m3) else {
        return;
    };
    let times: Vec<Option<Vec<Event>>> =
        cues.iter().map(|c| c.as_deref().map(srt::events)).collect();
    if rulers.is_empty() || times.iter().all(Option::is_none) {
        return;
    }
    // Alignment, the dump, the menu and the disk cache are CPU and disk
    // work; keep them off the async workers.
    let dump_dir = state.dump_dir.clone();
    let cache_dir = state.cache_dir.clone();
    let label_mode = state.label_mode;
    let lang = state.lang.clone();
    let cache_key = request.clone();
    let now = menu::now_s();
    let Ok((trials, entries)) = tokio::task::spawn_blocking(move || {
        let dump_name = match dump_dir
            .map(|dir| dump::write(&dir, &Dump::new(&kind, &rulers, &times, &scores)))
        {
            Some(Ok(name)) => {
                eprintln!("dump written file={name}");
                Some(name)
            }
            Some(Err(err)) => {
                eprintln!("dump failed error={:?}", err.kind());
                None
            }
            None => None,
        };
        let trials = align::trials(&rulers, &times);
        let (track, events) = &rulers[0];
        let entries = if label_mode {
            label::build(&trials, (*track, events), &cues, dump_name.as_deref())
        } else {
            let entries = menu::accepted(&trials, *track, &cues, &lang);
            if let Some(dir) = cache_dir
                && let Err(err) = menu::save(&dir, &cache_key, &entries, now)
            {
                eprintln!("cache write failed error={:?}", err.kind());
            }
            entries
        };
        (trials, entries)
    })
    .await
    else {
        return;
    };
    for trial in &trials {
        eprintln!("m4 probe {trial:?}");
    }
    eprintln!(
        "menu ready entries={} label_mode={label_mode}",
        entries.len()
    );
    if let Ok(mut menus) = state.menus.lock() {
        menus.insert(request, entries, now);
    }
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
    async fn subtitles_request_triggers_sources_probe() {
        let (tx, rx) = tokio::sync::oneshot::channel();
        let source: Arc<dyn SubtitleSource> =
            Arc::new(SignalSubtitles(std::sync::Mutex::new(Some(tx))));
        let app = router(
            AppState::new(TOKEN.to_owned())
                .unwrap()
                .with_subtitle_sources(vec![source], "tur".to_owned()),
        );
        let uri = format!("/{TOKEN}/subtitles/series/tt0944947:1:2/filename=A.mkv.json");
        let response = app
            .oneshot(Request::get(uri).body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let query = tokio::time::timeout(std::time::Duration::from_secs(1), rx)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(query, Query::new("series", "tt0944947:1:2", "tur").unwrap());
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
        menu::save(&dir, "series/tt2:1:3/B.mkv/7", &offered(), menu::now_s()).unwrap();
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
