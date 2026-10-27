//! Stremio add-on HTTP surface: manifest and subtitle routes behind a
//! secret path token.

use axum::{
    Json, Router,
    extract::{Path, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::get,
};
use serde::Serialize;
use serde_json::{Value, json};
use std::sync::Arc;
use tower_http::cors::CorsLayer;

use crate::extra::PlaybackHints;
use crate::streams::{self, StreamSource};

/// Shared state for the HTTP handlers.
pub struct AppState {
    token: String,
    streams: Option<Arc<dyn StreamSource>>,
}

impl AppState {
    /// `token` is the secret path segment; it must be non-empty.
    pub fn new(token: String) -> Option<Self> {
        (!token.is_empty()).then_some(Self {
            token,
            streams: None,
        })
    }

    /// Enables the K2 probe against this stream list source.
    pub fn with_stream_source(mut self, source: Arc<dyn StreamSource>) -> Self {
        self.streams = Some(source);
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
    Router::new()
        .route("/{token}/manifest.json", get(manifest))
        .route("/{token}/subtitles/{kind}/{id}", get(subtitles))
        .route(
            "/{token}/subtitles/{kind}/{id}/{extra}",
            get(subtitles_with_extra),
        )
        .layer(CorsLayer::permissive())
        .with_state(Arc::new(state))
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
    if let Some(source) = state.streams.clone() {
        // Runs beside the response so Stremio is not kept waiting.
        let (kind, id) = (kind.to_owned(), id.to_owned());
        tokio::spawn(async move {
            let outcome = streams::probe(source.as_ref(), &kind, &id, &hints).await;
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

    #[test]
    fn empty_token_is_rejected() {
        assert!(AppState::new(String::new()).is_none());
    }
}
