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

use crate::extra::PlaybackHints;
use crate::range::RangeReader;
use crate::sources::{self, SubtitleSource};
use crate::streams::{self, StreamSource};

/// How many ranked candidates the sources probe downloads per request.
pub const SOURCES_LIMIT: usize = 3;

/// Subtitle provider and the languages asked of it.
type SubtitleSetup = (Arc<dyn SubtitleSource>, Arc<[String]>);

/// Shared state for the HTTP handlers.
pub struct AppState {
    token: String,
    streams: Option<Arc<dyn StreamSource>>,
    range: Option<Arc<dyn RangeReader>>,
    subtitles: Option<SubtitleSetup>,
}

impl AppState {
    /// `token` is the secret path segment; it must be non-empty.
    pub fn new(token: String) -> Option<Self> {
        (!token.is_empty()).then_some(Self {
            token,
            streams: None,
            range: None,
            subtitles: None,
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

    /// Enables the M3 probe: candidates in `languages` are listed, ranked
    /// and the best few downloaded for each request.
    pub fn with_subtitle_source(
        mut self,
        source: Arc<dyn SubtitleSource>,
        languages: Vec<String>,
    ) -> Self {
        self.subtitles = Some((source, languages.into()));
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
    if let Some((source, languages)) = state.subtitles.clone() {
        let (kind, id, hints) = (kind.to_owned(), id.to_owned(), hints.clone());
        tokio::spawn(async move {
            let outcome = sources::collect(
                source.as_ref(),
                &kind,
                &id,
                &hints,
                &languages,
                SOURCES_LIMIT,
            )
            .await;
            eprintln!("m3 sources kind={kind} outcome={outcome:?}");
        });
    }
    if let Some(source) = state.streams.clone() {
        // Runs beside the response so Stremio is not kept waiting.
        let range = state.range.clone();
        let (kind, id) = (kind.to_owned(), id.to_owned());
        tokio::spawn(async move {
            let outcome =
                streams::probe(source.as_ref(), range.as_deref(), &kind, &id, &hints).await;
            eprintln!("k2 probe kind={kind} outcome={outcome:?}");
        });
    }
    // Lookup, extraction and alignment are not implemented yet.
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

    struct SignalSubtitles(std::sync::Mutex<Option<tokio::sync::oneshot::Sender<String>>>);

    impl SubtitleSource for SignalSubtitles {
        fn list<'a>(
            &'a self,
            kind: &'a str,
            id: &'a str,
            hints: &'a PlaybackHints,
        ) -> sources::ListFuture<'a> {
            if let Some(tx) = self.0.lock().unwrap().take() {
                let _ = tx.send(format!("{kind}/{id}/{}", hints.filename.is_some()));
            }
            Box::pin(async { Ok(Vec::new()) })
        }

        fn download<'a>(&'a self, _candidate: &'a sources::Candidate) -> sources::BodyFuture<'a> {
            Box::pin(async { Err(sources::SourceError::Other) })
        }
    }

    #[tokio::test]
    async fn subtitles_request_triggers_sources_probe() {
        let (tx, rx) = tokio::sync::oneshot::channel();
        let source = Arc::new(SignalSubtitles(std::sync::Mutex::new(Some(tx))));
        let app = router(
            AppState::new(TOKEN.to_owned())
                .unwrap()
                .with_subtitle_source(source, vec!["tur".to_owned()]),
        );
        let uri = format!("/{TOKEN}/subtitles/movie/tt0111161/filename=A.mkv.json");
        let response = app
            .oneshot(Request::get(uri).body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let called = tokio::time::timeout(std::time::Duration::from_secs(1), rx)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(called, "movie/tt0111161/true");
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
