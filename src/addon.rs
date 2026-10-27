//! Stremio add-on HTTP surface: manifest and subtitle routes behind a
//! secret path token.

use axum::{
    Json, Router,
    extract::{MatchedPath, Path, Request, State},
    http::StatusCode,
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::get,
};
use serde::Serialize;
use serde_json::{Value, json};
use std::sync::Arc;
use tower_http::cors::CorsLayer;

use crate::align;
use crate::extra::PlaybackHints;
use crate::range::RangeReader;
use crate::sources::{self, Query, SubtitleSource};
use crate::streams::{self, StreamSource};

/// Shared state for the HTTP handlers.
pub struct AppState {
    token: String,
    streams: Option<Arc<dyn StreamSource>>,
    range: Option<Arc<dyn RangeReader>>,
    subtitle_sources: Vec<Arc<dyn SubtitleSource>>,
    /// ISO 639-2 language subtitles are looked for in.
    lang: String,
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
    Path((token, kind, id)): Path<(String, String, String)>,
) -> Response {
    respond(&state, &token, &kind, &id, PlaybackHints::default())
}

async fn subtitles_with_extra(
    State(state): State<Arc<AppState>>,
    Path((token, kind, id, extra)): Path<(String, String, String, String)>,
) -> Response {
    respond(&state, &token, &kind, &id, PlaybackHints::parse(&extra))
}

fn respond(state: &AppState, token: &str, kind: &str, id: &str, hints: PlaybackHints) -> Response {
    if !state.authorized(token) {
        return StatusCode::NOT_FOUND.into_response();
    }
    let id = id.strip_suffix(".json").unwrap_or(id);
    if !matches!(kind, "movie" | "series") || !id.starts_with("tt") {
        return StatusCode::NOT_FOUND.into_response();
    }
    // Only field presence is logged; ids and filenames reveal what is watched.
    eprintln!("subtitles request kind={kind} hints={hints:?}");
    let stream_source = state.streams.clone();
    let subtitle_sources = Some(state.subtitle_sources.clone()).filter(|s| !s.is_empty());
    let query = Query::new(kind, id, &state.lang);
    if stream_source.is_some() || (subtitle_sources.is_some() && query.is_some()) {
        // Runs beside the response so Stremio is not kept waiting.
        let range = state.range.clone();
        let (kind, id) = (kind.to_owned(), id.to_owned());
        tokio::spawn(async move {
            let filename = hints.filename.clone();
            let k2 = async {
                let source = stream_source?;
                let outcome =
                    streams::probe(source.as_ref(), range.as_deref(), &kind, &id, &hints).await;
                eprintln!("k2 probe kind={kind} outcome={outcome:?}");
                Some(outcome)
            };
            let m3 = async {
                let (sources, query) = (subtitle_sources?, query?);
                let (outcome, times) = sources::probe(&sources, &query, filename.as_deref()).await;
                eprintln!("m3 probe query={query:?} outcome={outcome:?}");
                Some(times)
            };
            let (k2, times) = tokio::join!(k2, m3);
            let rulers = match k2 {
                Some(streams::ProbeOutcome::Matched { mkv: Some(mkv), .. }) => {
                    (*mkv).ok().map(|report| report.rulers.0)
                }
                _ => None,
            };
            // Alignment is CPU work; keep it off the async workers.
            if let (Some(rulers), Some(times)) = (rulers, times)
                && !rulers.is_empty()
                && times.iter().any(Option::is_some)
                && let Ok(trials) =
                    tokio::task::spawn_blocking(move || align::trials(&rulers, &times)).await
            {
                for trial in trials {
                    eprintln!("m4 probe {trial:?}");
                }
            }
        });
    }
    // Serving aligned subtitles comes in M5; for now only the probes run.
    Json(SubtitlesResponse {
        subtitles: Vec::new(),
    })
    .into_response()
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
