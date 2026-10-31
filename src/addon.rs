//! Stremio add-on HTTP surface: manifest and subtitle routes behind a
//! secret path token.

use axum::{
    Json, Router,
    body::Bytes,
    extract::{MatchedPath, Path, Query as UrlQuery, Request, State},
    http::{HeaderMap, StatusCode, header},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::get,
};
use futures_util::FutureExt;
use futures_util::future::{BoxFuture, Shared};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use tower_http::cors::CorsLayer;

use crate::align;
use crate::dump::{self, Dump};
use crate::events::{self, event};
use crate::extra::PlaybackHints;
use crate::label;
use crate::menu::{self, MenuCache, Notice};
use crate::mp4::Container;
use crate::pgs;
use crate::range::RangeReader;
use crate::ruler::Event;
use crate::settings::{self, TranslateSettings};
use crate::sources::{self, Query, SubtitleSource};
use crate::srt;
use crate::streams::{self, StreamSource};
use crate::translations::{Refused, Translations};

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
    /// Looked for when nothing in `lang` fits (SS-036); the source text for
    /// translation.
    fallback_lang: Option<String>,
    /// Where probe timings are saved for calibration, if anywhere.
    dump_dir: Option<std::path::PathBuf>,
    /// Menus already answered: accepted subtitles, or in labeling mode
    /// every candidate.
    menus: Mutex<MenuCache>,
    /// List every candidate, marked, instead of the accepted ones.
    label_mode: bool,
    /// Where answered menus are kept across restarts (not in labeling mode).
    cache_dir: Option<std::path::PathBuf>,
    /// Translates the fitting English subtitle when nothing in `lang` fits
    /// (SS-026).
    translations: Option<Arc<Translations>>,
    /// Probes running now, by request key. Stremio asks twice for one
    /// playback (without, then with the video hash); the second request
    /// waits for the first one's probes instead of starting its own (SS-041).
    probing: Mutex<HashMap<String, Shared<BoxFuture<'static, ()>>>>,
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
            fallback_lang: None,
            dump_dir: None,
            menus: Mutex::new(MenuCache::default()),
            label_mode: false,
            cache_dir: None,
            translations: None,
            probing: Mutex::new(HashMap::new()),
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

    /// When no subtitle in `lang` fits, searches the same sources in
    /// `fallback` and offers the best one that does (SS-036).
    pub fn with_fallback_lang(mut self, fallback: String) -> Self {
        self.fallback_lang = Some(fallback);
        self
    }

    /// Offers a translation of the fitting English subtitle when nothing in
    /// the wanted language fits (SS-026).
    pub fn with_translations(mut self, translations: Translations) -> Self {
        self.translations = Some(Arc::new(translations));
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
        .route(TRANSLATIONS_ROUTE, get(translations_json))
        .route(SETTINGS_ROUTE, get(settings_json).post(change_settings))
        .layer(middleware::from_fn_with_state(state.clone(), log_access))
        .layer(CorsLayer::permissive())
        .with_state(state)
}

const EVENTS_ROUTE: &str = "/{token}/events.json";
const TRANSLATIONS_ROUTE: &str = "/{token}/translations.json";
const SETTINGS_ROUTE: &str = "/{token}/settings.json";
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

/// Translation jobs for the dashboard (SS-047): progress and the played
/// file's name, never the text. `null` when translation is off.
async fn translations_json(
    State(state): State<Arc<AppState>>,
    Path(token): Path<String>,
) -> Response {
    if !state.authorized(&token) {
        return StatusCode::NOT_FOUND.into_response();
    }
    let status = state.translations.as_ref().map(|t| t.status());
    ([(header::CACHE_CONTROL, "no-store")], Json(status)).into_response()
}

/// The translation choice the dashboard shows and changes (SS-048).
#[derive(Serialize)]
struct SettingsView {
    current: TranslateSettings,
    defaults: TranslateSettings,
    /// Efforts besides [`settings::MODEL_DEFAULT`].
    efforts: &'static [&'static str],
}

fn settings_view(translations: &Translations) -> SettingsView {
    SettingsView {
        current: translations.settings(),
        defaults: TranslateSettings::defaults(),
        efforts: &crate::openrouter::REASONING_EFFORTS,
    }
}

/// `null` when translation is off.
async fn settings_json(State(state): State<Arc<AppState>>, Path(token): Path<String>) -> Response {
    if !state.authorized(&token) {
        return StatusCode::NOT_FOUND.into_response();
    }
    let view = state.translations.as_deref().map(settings_view);
    ([(header::CACHE_CONTROL, "no-store")], Json(view)).into_response()
}

/// Takes a new choice for new translation jobs. The body is read only
/// after the token check, so a wrong token gets 404 whatever it sends.
async fn change_settings(
    State(state): State<Arc<AppState>>,
    Path(token): Path<String>,
    body: Bytes,
) -> Response {
    if !state.authorized(&token) {
        return StatusCode::NOT_FOUND.into_response();
    }
    let Some(translations) = &state.translations else {
        return (StatusCode::CONFLICT, Json(json!({ "refused": "off" }))).into_response();
    };
    let Ok(choice) = serde_json::from_slice::<TranslateSettings>(&body) else {
        return (StatusCode::BAD_REQUEST, Json(json!({ "refused": "body" }))).into_response();
    };
    let (status, refused) = match translations.change(choice).await {
        Ok(()) => return Json(settings_view(translations)).into_response(),
        Err(Refused::Invalid(settings::Invalid::Model)) => (StatusCode::BAD_REQUEST, "model"),
        Err(Refused::Invalid(settings::Invalid::Reasoning)) => {
            (StatusCode::BAD_REQUEST, "reasoning")
        }
        Err(Refused::Unbuilt) => (StatusCode::BAD_REQUEST, "unbuilt"),
        Err(Refused::Save(kind)) => {
            event!(
                "translate",
                "translation settings save failed error={kind:?}"
            );
            (StatusCode::INTERNAL_SERVER_ERROR, "save")
        }
    };
    (status, Json(json!({ "refused": refused }))).into_response()
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
    if matches!(route, Some(EVENTS_ROUTE | TRANSLATIONS_ROUTE)) && response.status().is_success() {
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
    // The filename goes to the dashboard alone, behind the token.
    events::record_labeled(
        "request",
        format!("subtitles request kind={kind} hints={hints:?}"),
        hints.filename.clone(),
    );
    // Kept in memory only, to find this request's menu again; on disk only
    // its hash is used.
    let request = format!(
        "{kind}/{id}/{}/{}",
        hints.filename.as_deref().unwrap_or_default(),
        hints.video_size.unwrap_or_default()
    );
    // Names a translation job on the dashboard (SS-047).
    let label = hints.filename.clone();
    let label = label.as_deref();
    let now = menu::now_s();
    if let Some((list, offered)) = menu_list(&state, headers, token, &request, label, now) {
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
            if let Some((list, offered)) = menu_list(&state, headers, token, &request, label, now) {
                event!("served", "subtitles served from=disk offered={offered}");
                return list;
            }
        }
    }
    if state.streams.is_some() || !state.subtitle_sources.is_empty() {
        let (work, joined) = probes_for(&state, kind, id, hints, &request);
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
        if let Some((list, offered)) = menu_list(&state, headers, token, &request, label, now) {
            let from = if joined { "joined" } else { "probes" };
            event!("served", "subtitles served from={from} offered={offered}");
            return list;
        }
    }
    event!("served", "subtitles served from=none offered=0");
    Json(SubtitlesResponse {
        subtitles: Vec::new(),
    })
    .into_response()
}

/// The probes of `request`: the ones already running for it, or new ones.
/// The flag says whether they were already running.
fn probes_for(
    state: &Arc<AppState>,
    kind: &str,
    id: &str,
    hints: PlaybackHints,
    request: &str,
) -> (Shared<BoxFuture<'static, ()>>, bool) {
    let Ok(mut probing) = state.probing.lock() else {
        // Not shared, but still answered.
        let work = run_probes(
            state.clone(),
            kind.to_owned(),
            id.to_owned(),
            hints,
            request.to_owned(),
        );
        return (tokio::spawn(work).map(|_| ()).boxed().shared(), false);
    };
    if let Some(work) = probing.get(request) {
        return (work.clone(), true);
    }
    let (owner, key) = (state.clone(), request.to_owned());
    let work = run_probes(
        state.clone(),
        kind.to_owned(),
        id.to_owned(),
        hints,
        request.to_owned(),
    );
    // The task forgets itself when done, waited for or not; its menu is in
    // memory by then, so a later request finds that instead.
    let task = tokio::spawn(async move {
        work.await;
        if let Ok(mut probing) = owner.probing.lock() {
            probing.remove(&key);
        }
    });
    let work = task.map(|_| ()).boxed().shared();
    probing.insert(request.to_owned(), work.clone());
    (work, false)
}

/// How long a request waits for the probes before answering without them.
const MENU_WAIT: std::time::Duration = std::time::Duration::from_secs(15);

/// The menu of `request` as a Stremio subtitle list, and its length.
/// `label`, the played file's name, names a translation job on the
/// dashboard.
fn menu_list(
    state: &AppState,
    headers: &HeaderMap,
    token: &str,
    request: &str,
    label: Option<&str>,
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
    // A listed translation gets its job going before anyone picks it.
    let english: Vec<String> = entries
        .iter()
        .filter(|e| e.translated)
        .map(|e| e.srt.clone())
        .collect();
    drop(menus);
    if let Some(translations) = &state.translations {
        for english in &english {
            translations.ensure(english, label);
        }
    }
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
    let picked = state
        .menus
        .lock()
        .ok()
        .and_then(|menus| menus.get(entry, n).cloned());
    // A translation is served as far as it has got (SS-026); with no
    // translator any more, as the English it starts from.
    let srt = picked.map(|picked| match &state.translations {
        Some(translations) if picked.translated => {
            translations.ensure(&picked.srt, None);
            translations.current(&picked.srt).unwrap_or(picked.srt)
        }
        _ => picked.srt,
    });
    // Whether Stremio fetches the file again on each pick (SS-042).
    event!(
        "fetch",
        "subtitle fetched entry={entry} n={n} bytes={}",
        srt.as_ref().map_or(0, String::len)
    );
    match srt {
        // `no-store`, so a later pick can get a longer file.
        Some(srt) => (
            [
                (header::CONTENT_TYPE, "application/x-subrip; charset=utf-8"),
                (header::CACHE_CONTROL, "no-store"),
            ],
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
        Ok(Built::Found(entries)) => (entries, menu::KEEP_FOUND_S, None),
        // A subtitle in the wanted language may appear later, so the answer
        // lasts only as long as the reason. The reason is listed first,
        // unless a translation is offered: that is no error (SS-050).
        Ok(Built::Missing(notice, fallback)) => {
            let mut entries = Vec::new();
            // The best English that fits is what gets translated (SS-026).
            match (&state.translations, fallback.first()) {
                (Some(translations), Some(english)) => {
                    entries.push(translations.entry(&english.srt));
                }
                _ => entries.push(notice.entry()),
            }
            entries.extend(fallback);
            (entries, notice.keep_s(), Some(notice))
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
    let (report, range, mkv, mp4) = match k2 {
        Some(streams::ProbeOutcome::Matched {
            report,
            range,
            mkv,
            mp4,
            ..
        }) => (report, range, mkv, mp4),
        Some(streams::ProbeOutcome::NoHints) => return Err(Notice::NoStreamMatch),
        Some(streams::ProbeOutcome::Failed { .. }) | None => return Err(Notice::Failed),
    };
    if report.matched_by.is_none() {
        return Err(Notice::NoStreamMatch);
    }
    match (range, mkv, mp4) {
        (_, Some(mkv), _) => match *mkv {
            Ok(report) if !report.rulers.0.is_empty() => Ok(report.rulers.0),
            Ok(report) if report.english_text_indexed == Some(false) => Err(Notice::NotIndexed),
            Ok(report) => {
                // No English text track: the image track's times, if indexed.
                let rulers = pgs::rulers(&report.pgs);
                if !rulers.is_empty() {
                    let counts: Vec<(u64, usize)> =
                        rulers.iter().map(|(t, l)| (*t, l.len())).collect();
                    event!("m5", "pgs ruler tracks={counts:?}");
                    Ok(rulers)
                } else if report.subtitles.iter().any(|s| s.english) {
                    Err(Notice::ImageOnly)
                } else {
                    Err(Notice::NoEnglishTrack)
                }
            }
            Err(_) => Err(Notice::Failed),
        },
        (_, None, Some(mp4)) => match *mp4 {
            Ok(report) if !report.rulers.0.is_empty() => Ok(report.rulers.0),
            Ok(report) if report.fragmented => Err(Notice::Unsupported),
            Ok(_) => Err(Notice::NoEnglishTrack),
            Err(_) => Err(Notice::Failed),
        },
        (Some(Ok(range)), None, None) => match range.container {
            Container::Avi | Container::MpegTs => Err(Notice::Unsupported),
            Container::Unknown => Err(Notice::NotMkv),
            // Recognised, but the range was not honoured.
            Container::Matroska | Container::Mp4 => Err(Notice::Failed),
        },
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

/// What the probes found once there is a ruler.
enum Built {
    /// Subtitles in the wanted language (in labeling mode, every candidate).
    Found(Vec<menu::MenuEntry>),
    /// Why there are none, and the fallback-language subtitle that fits, if
    /// any (SS-036).
    Missing(Notice, Vec<menu::MenuEntry>),
}

/// K2 and M3 probes side by side, then alignment, the timing dump and the
/// menu: accepted subtitles, or in labeling mode every candidate, marked.
/// A missing ruler answers at once, without waiting for M3. When nothing in
/// the wanted language fits, the fallback language is tried against the
/// same ruler.
async fn build_menu(
    state: &Arc<AppState>,
    kind: String,
    id: String,
    hints: PlaybackHints,
) -> Result<Built, Notice> {
    let query = Query::new(&kind, &id, &state.lang);
    let m3 = match query {
        Some(query) if !state.subtitle_sources.is_empty() => {
            let sources = state.subtitle_sources.clone();
            let filename = hints.filename.clone();
            Some(tokio::spawn(async move {
                let probe = sources::probe(&sources, &query, filename.as_deref()).await;
                event!("m3", "m3 probe query={query:?} outcome={:?}", probe.report);
                probe
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
    let mut probe = match m3 {
        Some(m3) => m3.await.map_err(|_| Notice::Failed)?,
        None => return Err(Notice::Failed),
    };
    let rulers = Arc::new(rulers);
    let wanted = align_candidates(
        &mut probe,
        rulers.clone(),
        kind.clone(),
        state.lang.clone(),
        state.dump_dir.clone(),
        state.label_mode,
    )
    .await;
    let notice = match wanted {
        Ok(entries) if !entries.is_empty() => return Ok(Built::Found(entries)),
        Ok(_) => Notice::NoneAccepted,
        Err(Notice::NoCandidates) => Notice::NoCandidates,
        Err(notice) => return Err(notice),
    };
    let fallback = fallback(state, &kind, &id, hints.filename.as_deref(), rulers).await;
    Ok(Built::Missing(notice, fallback))
}

/// The best subtitle in the fallback language that fits the ruler, or none
/// (SS-036). Searched, ranked, aligned and accepted like the wanted
/// language; not dumped, and off in labeling mode.
async fn fallback(
    state: &AppState,
    kind: &str,
    id: &str,
    filename: Option<&str>,
    rulers: Arc<Vec<(u64, Vec<Event>)>>,
) -> Vec<menu::MenuEntry> {
    let Some(lang) = state.fallback_lang.clone().filter(|_| !state.label_mode) else {
        return Vec::new();
    };
    let Some(query) = Query::new(kind, id, &lang) else {
        return Vec::new();
    };
    let mut probe = sources::probe(&state.subtitle_sources, &query, filename).await;
    event!(
        "m3",
        "m3 fallback probe query={query:?} outcome={:?}",
        probe.report
    );
    let mut entries = align_candidates(&mut probe, rulers, kind.to_owned(), lang, None, false)
        .await
        .unwrap_or_default();
    entries.truncate(1);
    event!("m5", "fallback accepted={}", entries.len());
    entries
}

/// Downloaded candidates laid over the ruler: the accepted ones as menu
/// entries in `lang` (in labeling mode, every one, marked), or why there is
/// nothing to align. When none of the first downloads fits, the next few
/// ranked candidates are downloaded and tried too (SS-040); not in labeling
/// mode, which shows every candidate anyway. Timings of every download are
/// dumped under `dump_dir`, if given.
async fn align_candidates(
    probe: &mut sources::Probe,
    rulers: Arc<Vec<(u64, Vec<Event>)>>,
    kind: String,
    lang: String,
    dump_dir: Option<std::path::PathBuf>,
    label_mode: bool,
) -> Result<Vec<menu::MenuEntry>, Notice> {
    let searches = &probe.report.searches;
    if searches.iter().all(|(_, found)| found.is_err()) {
        return Err(Notice::Failed);
    }
    if searches
        .iter()
        .all(|(_, found)| matches!(found, Ok(0) | Err(_)))
    {
        return Err(Notice::NoCandidates);
    }
    if probe.cues.iter().all(Option::is_none) {
        return Err(Notice::Failed);
    }
    if label_mode {
        let dump_name = write_dump(dump_dir, kind, rulers.clone(), probe).await;
        return align_round(probe, rulers, lang, Some(dump_name), 0).await;
    }
    let mut entries = align_round(probe, rulers.clone(), lang.clone(), None, 0).await?;
    if entries.is_empty() && probe.has_more() {
        let (tried, copies) = (probe.cues.len(), probe.report.duplicates);
        probe.download_more().await;
        event!(
            "m3",
            "m3 more downloads={:?} duplicates={}",
            &probe.report.downloads[tried..],
            probe.report.duplicates - copies
        );
        entries = align_round(probe, rulers.clone(), lang, None, tried).await?;
    }
    write_dump(dump_dir, kind, rulers, probe).await;
    Ok(entries)
}

/// Every download of `probe` laid over the ruler: the accepted ones as menu
/// entries, or in labeling mode (`label` holds the dump's name, if one was
/// written) every one, marked. Trials of downloads before `logged` were
/// logged by an earlier round and are not logged again.
async fn align_round(
    probe: &sources::Probe,
    rulers: Arc<Vec<(u64, Vec<Event>)>>,
    lang: String,
    label: Option<Option<String>>,
    logged: usize,
) -> Result<Vec<menu::MenuEntry>, Notice> {
    let cues = probe.cues.clone();
    // Alignment and the menu are CPU work; keep them off the async workers.
    let (trials, entries) = tokio::task::spawn_blocking(move || {
        let times: Vec<Option<Vec<Event>>> =
            cues.iter().map(|c| c.as_deref().map(srt::events)).collect();
        let trials = align::trials(&rulers, &times);
        let (track, events) = &rulers[0];
        let entries = match label {
            Some(dump_name) => label::build(&trials, (*track, events), &cues, dump_name.as_deref()),
            None => menu::accepted(&trials, &rulers, &cues, &lang),
        };
        (trials, entries)
    })
    .await
    .map_err(|_| Notice::Failed)?;
    for trial in trials.iter().filter(|t| t.candidate >= logged) {
        event!("m4", "m4 probe {trial:?}");
    }
    Ok(entries)
}

/// Writes the timings of every download of `probe` under `dump_dir`, if
/// given; the dump's file name.
async fn write_dump(
    dump_dir: Option<std::path::PathBuf>,
    kind: String,
    rulers: Arc<Vec<(u64, Vec<Event>)>>,
    probe: &sources::Probe,
) -> Option<String> {
    let dir = dump_dir?;
    let scores: Vec<Option<i32>> = probe
        .report
        .downloads
        .iter()
        .map(|d| d.as_ref().ok().map(|stats| stats.score))
        .collect();
    let times: Vec<Option<Vec<Event>>> = probe
        .cues
        .iter()
        .map(|c| c.as_deref().map(srt::events))
        .collect();
    let written = tokio::task::spawn_blocking(move || {
        dump::write(&dir, &Dump::new(&kind, &rulers, &times, &scores))
    })
    .await;
    match written {
        Ok(Ok(name)) => {
            event!("dump", "dump written file={name}");
            Some(name)
        }
        Ok(Err(err)) => {
            event!("dump", "dump failed error={:?}", err.kind());
            None
        }
        Err(_) => {
            event!("dump", "dump failed error=task");
            None
        }
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

    fn opened_as(
        container: Container,
        mp4: Option<crate::mp4::Mp4Outcome>,
    ) -> streams::ProbeOutcome {
        streams::ProbeOutcome::Matched {
            elapsed_ms: 0,
            report: streams::MatchReport {
                matched_by: Some(streams::MatchedBy::Both),
                ..Default::default()
            },
            range: Some(Ok(crate::range::RangeReport {
                hops: Vec::new(),
                status: 206,
                partial: true,
                accept_ranges: true,
                size_matches: Some(true),
                content: crate::range::ContentKind::OctetStream,
                bytes_read: 0,
                ebml_magic: container == Container::Matroska,
                container,
                elapsed_ms: 0,
            })),
            mkv: None,
            mp4: mp4.map(Box::new),
        }
    }

    fn mp4_report(fragmented: bool, rulers: Vec<(u64, Vec<Event>)>) -> crate::mp4::Mp4Outcome {
        Ok(crate::mp4::Mp4Report {
            requests: 0,
            retries: 0,
            bytes_read: 0,
            elapsed_ms: 0,
            moov_first: true,
            fragmented,
            tracks: Vec::new(),
            rulers: crate::mkv::Rulers(rulers),
        })
    }

    #[test]
    fn non_mkv_files_get_a_ruler_or_their_own_reason() {
        let events = vec![Event {
            start_ms: 0,
            end_ms: 1000,
        }];
        let ruled = opened_as(
            Container::Mp4,
            Some(mp4_report(false, vec![(2, events.clone())])),
        );
        assert_eq!(rulers_of(Some(ruled)), Ok(vec![(2, events)]));
        let bare = opened_as(Container::Mp4, Some(mp4_report(false, Vec::new())));
        assert_eq!(rulers_of(Some(bare)), Err(Notice::NoEnglishTrack));
        let fragmented = opened_as(Container::Mp4, Some(mp4_report(true, Vec::new())));
        assert_eq!(rulers_of(Some(fragmented)), Err(Notice::Unsupported));
        let broken = opened_as(Container::Mp4, Some(Err(crate::mp4::Mp4Error::NoMoov)));
        assert_eq!(rulers_of(Some(broken)), Err(Notice::Failed));
        for (container, notice) in [
            (Container::Avi, Notice::Unsupported),
            (Container::MpegTs, Notice::Unsupported),
            (Container::Unknown, Notice::NotMkv),
        ] {
            assert_eq!(
                rulers_of(Some(opened_as(container, None))),
                Err(notice),
                "{container:?}"
            );
        }
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
    async fn filename_reaches_the_dashboard_but_not_the_log_line() {
        let (_, before) = get(&format!("/{TOKEN}/events.json?after=0")).await;
        let after = before.unwrap()["next"].as_u64().unwrap();
        let name = "Label.Test.S01E02.mkv";
        get(&format!(
            "/{TOKEN}/subtitles/series/tt0944947:1:2/filename={name}.json"
        ))
        .await;
        let (_, page) = get(&format!("/{TOKEN}/events.json?after={after}")).await;
        let page = page.unwrap();
        let request = page["events"]
            .as_array()
            .unwrap()
            .iter()
            .find(|e| e["label"] == name)
            .expect("request event with the filename");
        assert_eq!(request["area"], "request");
        let line = request["line"].as_str().unwrap();
        assert!(
            !line.contains(name) && !line.contains("tt0944947"),
            "{line}"
        );
        assert!(line.contains("has_filename: true"), "{line}");
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
        assert_eq!(list["subtitles"][0]["lang"], menu::NOTICE_LANG);
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
        let cues: &[(u64, u64)] = if pgs {
            &[(10, 100), (20, 100), (20, 100)]
        } else {
            &[(0, 100)]
        };
        mkv_head(pgs, cues)
    }

    /// An MKV head whose one English track has a cue point per `(time,
    /// duration)`, in milliseconds.
    fn mkv_head(pgs: bool, cues: &[(u64, u64)]) -> Vec<u8> {
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
        let cue_at = |&(time, duration): &(u64, u64)| {
            el(
                0xBB,
                &[
                    el(0xB3, &time.to_be_bytes()),
                    el(
                        0xB7,
                        &[
                            el(0xF7, &[2]),
                            el(0xB2, &duration.to_be_bytes()),
                            el(0xF1, &[0]),
                            el(0xF0, &[0]),
                        ]
                        .concat(),
                    ),
                ]
                .concat(),
            )
        };
        let cue: Vec<u8> = cues.iter().flat_map(cue_at).collect();
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

    /// Serves the head `.1`; later reads never finish.
    struct ProbeRange(Arc<tokio::sync::Notify>, Vec<u8>);

    impl RangeReader for ProbeRange {
        fn open<'a>(
            &'a self,
            _url: &'a str,
            _host: &'a str,
            _size: Option<u64>,
        ) -> crate::range::OpenFuture<'a> {
            let head = self.1.clone();
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
                .with_range_reader(Arc::new(ProbeRange(read.clone(), probe_head(false))))
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
        assert_eq!(names(&app, &list).await, ["NoCandidates"]);
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
                    probe_head(true),
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
        assert_eq!(names(&app, &list).await, ["ImageOnly"]);
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

    /// One candidate per language it has a subtitle for; notes each search.
    struct LangSubtitles {
        srt: std::collections::HashMap<&'static str, String>,
        searched: Mutex<Vec<String>>,
    }

    impl SubtitleSource for LangSubtitles {
        fn kind(&self) -> sources::SourceKind {
            sources::SourceKind::OpenSubtitles
        }

        fn search<'a>(&'a self, query: &'a Query) -> sources::SearchFuture<'a> {
            self.searched.lock().unwrap().push(query.lang.clone());
            let found = self
                .srt
                .contains_key(query.lang.as_str())
                .then(|| sources::Candidate::fake(self.kind(), "A"))
                .into_iter()
                .collect();
            Box::pin(async { Ok(found) })
        }

        fn download<'a>(
            &'a self,
            query: &'a Query,
            _candidate: &'a sources::Candidate,
        ) -> sources::DownloadFuture<'a> {
            let bytes = self
                .srt
                .get(query.lang.as_str())
                .map(|srt| srt.clone().into_bytes())
                .ok_or(sources::SourceError::Other);
            Box::pin(async { bytes })
        }
    }

    /// Candidates `r0`, `r1`, … in the wanted language, ranked in that
    /// order, one file each; records which ones were downloaded.
    struct RankedSubtitles {
        files: Vec<String>,
        downloaded: Mutex<Vec<String>>,
    }

    impl SubtitleSource for RankedSubtitles {
        fn kind(&self) -> sources::SourceKind {
            sources::SourceKind::OpenSubtitles
        }

        fn search<'a>(&'a self, _query: &'a Query) -> sources::SearchFuture<'a> {
            let found = (0..self.files.len())
                .map(|i| sources::Candidate::fake(self.kind(), &format!("r{i}")))
                .collect();
            Box::pin(async { Ok(found) })
        }

        fn download<'a>(
            &'a self,
            _query: &'a Query,
            candidate: &'a sources::Candidate,
        ) -> sources::DownloadFuture<'a> {
            self.downloaded
                .lock()
                .unwrap()
                .push(candidate.release.clone());
            let i: usize = candidate.release[1..].parse().unwrap();
            // Each file its own text, or later ones would be left out as
            // copies of the first (SS-055); timings stay as given.
            let bytes = self.files[i]
                .replacen("\n\n", &format!(" r{i}\n\n"), 1)
                .into_bytes();
            Box::pin(async { Ok(bytes) })
        }
    }

    #[tokio::test]
    async fn more_candidates_are_tried_only_when_none_of_the_first_fits() {
        let (head, fits, other) = film();
        let o = || other.clone();
        // (ranked files, labeling mode) → menu, downloads.
        let cases = [
            (
                vec![o(), o(), o(), fits.clone(), o()],
                false,
                vec!["tur"],
                5,
            ),
            (
                vec![fits.clone(), o(), o(), o(), o()],
                false,
                vec!["tur"],
                3,
            ),
            (vec![o(), o(), o(), o()], false, vec!["NoneAccepted"], 4),
            (vec![o(), o(), o()], false, vec!["NoneAccepted"], 3),
            (
                vec![o(), o(), o(), o(), o(), o(), o(), fits.clone()],
                false,
                vec!["NoneAccepted"],
                6,
            ),
            // Labeling mode lists every candidate anyway.
            (vec![o(), o(), o(), fits.clone()], true, vec![], 3),
        ];
        for (i, (files, label_mode, menu, downloads)) in cases.into_iter().enumerate() {
            let dir = std::env::temp_dir().join(format!(
                "subsync-more-{}-{i}-{}",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_nanos()
            ));
            let source = Arc::new(RankedSubtitles {
                files,
                downloaded: Mutex::new(Vec::new()),
            });
            let mut state = AppState::new(TOKEN.to_owned())
                .unwrap()
                .with_stream_source(Arc::new(ProbeStream))
                .with_range_reader(Arc::new(ProbeRange(
                    Arc::new(tokio::sync::Notify::new()),
                    head.clone(),
                )))
                .with_subtitle_sources(vec![source.clone()], "tur".to_owned())
                .with_dump_dir(dir.clone());
            if label_mode {
                state = state.with_label_mode();
            }
            let app = router(state);
            let uri = format!("/{TOKEN}/subtitles/movie/tt1/filename=A.mkv&videoSize=5.json");
            let (status, body) = call(&app, uri).await;
            assert_eq!(status, StatusCode::OK, "case {i}");
            let list: Value = serde_json::from_str(&body).unwrap();
            let langs = names(&app, &list).await;
            if !label_mode {
                assert_eq!(langs, menu, "case {i}");
            }
            let downloaded = source.downloaded.lock().unwrap().clone();
            let ranked: Vec<String> = (0..downloads).map(|n| format!("r{n}")).collect();
            assert_eq!(downloaded, ranked, "case {i}");

            // One dump, with every download.
            let names: Vec<_> = std::fs::read_dir(&dir)
                .unwrap()
                .map(|e| e.unwrap().path())
                .collect();
            assert_eq!(names.len(), 1, "case {i}");
            let saved: Dump = serde_json::from_slice(&std::fs::read(&names[0]).unwrap()).unwrap();
            assert_eq!(saved.candidates.len(), downloads, "case {i}");
            std::fs::remove_dir_all(&dir).unwrap();
        }
    }

    /// A 30-minute film's MKV head with an English text ruler, a subtitle
    /// for it timed 2.5 s early, and one for another film.
    fn film() -> (Vec<u8>, String, String) {
        use crate::synth::{self, Style};
        let mut rng = synth::Rng::new(36);
        let speech = synth::speech(&mut rng, 30);
        let ruler = synth::lines(&mut rng, &speech, &Style::RULER);
        let early = align::Fix {
            num: 1,
            den: 1,
            shift_ms: 2_500,
        };
        let fits = synth::distort(&synth::lines(&mut rng, &speech, &Style::OTHER), early);
        let elsewhere = synth::speech(&mut rng, 30);
        let other = synth::lines(&mut rng, &elsewhere, &Style::OTHER);
        let srt_of = |events: &[Event]| {
            let cues: Vec<srt::Cue> = events
                .iter()
                .enumerate()
                .map(|(i, e)| srt::Cue {
                    start_ms: e.start_ms,
                    end_ms: e.end_ms,
                    text: format!("line {i}"),
                })
                .collect();
            srt::render(
                &cues,
                align::Fix {
                    num: 1,
                    den: 1,
                    shift_ms: 0,
                },
            )
        };
        let cues: Vec<(u64, u64)> = ruler
            .iter()
            .map(|e| (e.start_ms, e.end_ms - e.start_ms))
            .collect();
        (mkv_head(false, &cues), srt_of(&fits), srt_of(&other))
    }

    /// `ProbeStream`'s list, once `go` lets each fetch through; counts them.
    struct GatedStream {
        go: tokio::sync::Semaphore,
        fetched: Mutex<usize>,
    }

    impl StreamSource for GatedStream {
        fn fetch<'a>(&'a self, kind: &'a str, id: &'a str) -> streams::FetchFuture<'a> {
            *self.fetched.lock().unwrap() += 1;
            Box::pin(async move {
                self.go.acquire().await.unwrap().forget();
                ProbeStream.fetch(kind, id).await
            })
        }

        fn host(&self) -> &str {
            "aio.test"
        }
    }

    #[tokio::test]
    async fn second_request_for_a_playback_waits_for_the_first_ones_probes() {
        let (head, fits, _) = film();
        let dir = std::env::temp_dir().join(format!(
            "subsync-joined-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let stream = Arc::new(GatedStream {
            go: tokio::sync::Semaphore::new(0),
            fetched: Mutex::new(0),
        });
        let mut srt = std::collections::HashMap::new();
        srt.insert("tur", fits);
        let source = Arc::new(LangSubtitles {
            srt,
            searched: Mutex::new(Vec::new()),
        });
        let app = router(
            AppState::new(TOKEN.to_owned())
                .unwrap()
                .with_stream_source(stream.clone())
                .with_range_reader(Arc::new(ProbeRange(
                    Arc::new(tokio::sync::Notify::new()),
                    head,
                )))
                .with_subtitle_sources(vec![source.clone()], "tur".to_owned())
                .with_dump_dir(dir.clone()),
        );
        // As Stremio asks: first without the video hash, then with it.
        let ask = |uri: String| {
            let app = app.clone();
            tokio::spawn(async move { call(&app, uri).await })
        };
        let first = ask(format!(
            "/{TOKEN}/subtitles/movie/tt1/filename=A.mkv&videoSize=5.json"
        ));
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        let second = ask(format!(
            "/{TOKEN}/subtitles/movie/tt1/videoHash=abc&filename=A.mkv&videoSize=5.json"
        ));
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        assert_eq!(*stream.fetched.lock().unwrap(), 1, "one probe running");
        stream.go.add_permits(10);
        let (first, second) = (first.await.unwrap(), second.await.unwrap());
        assert_eq!(first.0, StatusCode::OK);
        assert_eq!(first, second, "same menu");
        let list: Value = serde_json::from_str(&first.1).unwrap();
        assert_eq!(list["subtitles"][0]["lang"], "tur");
        assert_eq!(*stream.fetched.lock().unwrap(), 1, "one stream lookup");
        assert_eq!(*source.searched.lock().unwrap(), vec!["tur"], "one search");
        let dumps = std::fs::read_dir(&dir).unwrap().count();
        assert_eq!(dumps, 1, "one dump");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[tokio::test]
    async fn english_fallback_is_offered_only_when_nothing_else_fits() {
        let (head, fits, other) = film();
        // (Turkish, English, fallback on) → menu, languages searched.
        let cases = [
            (
                None,
                Some(&fits),
                true,
                vec!["NoCandidates", "eng"],
                vec!["tur", "eng"],
            ),
            (
                Some(&other),
                Some(&fits),
                true,
                vec!["NoneAccepted", "eng"],
                vec!["tur", "eng"],
            ),
            (Some(&fits), Some(&fits), true, vec!["tur"], vec!["tur"]),
            (
                Some(&other),
                Some(&other),
                true,
                vec!["NoneAccepted"],
                vec!["tur", "eng"],
            ),
            (None, Some(&fits), false, vec!["NoCandidates"], vec!["tur"]),
        ];
        for (i, (tur, eng, on, menu, searched)) in cases.into_iter().enumerate() {
            let mut srt = std::collections::HashMap::new();
            if let Some(tur) = tur {
                srt.insert("tur", tur.clone());
            }
            if let Some(eng) = eng {
                srt.insert("eng", eng.clone());
            }
            let source = Arc::new(LangSubtitles {
                srt,
                searched: Mutex::new(Vec::new()),
            });
            let mut state = AppState::new(TOKEN.to_owned())
                .unwrap()
                .with_stream_source(Arc::new(ProbeStream))
                .with_range_reader(Arc::new(ProbeRange(
                    Arc::new(tokio::sync::Notify::new()),
                    head.clone(),
                )))
                .with_subtitle_sources(vec![source.clone()], "tur".to_owned());
            if on {
                state = state.with_fallback_lang("eng".to_owned());
            }
            let app = router(state);
            let uri = format!("/{TOKEN}/subtitles/movie/tt1/filename=A.mkv&videoSize=5.json");
            let (status, body) = call(&app, uri).await;
            assert_eq!(status, StatusCode::OK, "case {i}");
            let list: Value = serde_json::from_str(&body).unwrap();
            let langs = names(&app, &list).await;
            assert_eq!(langs, menu, "case {i}");
            assert_eq!(*source.searched.lock().unwrap(), searched, "case {i}");

            // The English entry is the fitting subtitle, moved onto the ruler.
            if let Some(n) = langs.iter().position(|l| l == "eng") {
                let url = list["subtitles"][n]["url"].as_str().unwrap();
                let path = url.trim_start_matches("http://127.0.0.1:7000").to_owned();
                let (status, served) = call(&app, path).await;
                assert_eq!(status, StatusCode::OK, "case {i}");
                let (served, sent) = (srt::parse(&served), srt::parse(&fits));
                assert_eq!(served.len(), sent.len(), "case {i}");
                let moved = served[0].start_ms as i64 - sent[0].start_ms as i64;
                assert!((moved - 2_500).abs() <= 200, "case {i}: moved {moved} ms");
            }
        }
    }

    /// Translates each asked line as `tr <text>`; blocks wait for `go`
    /// permits, so a test can look at a translation half way.
    struct GatedTranslator {
        go: tokio::sync::Semaphore,
        analyzed: Mutex<usize>,
    }

    impl crate::translate::Translator for GatedTranslator {
        fn analyze<'a>(
            &'a self,
            _request: &'a crate::translate::AnalysisRequest,
        ) -> crate::translate::AnalyzeFuture<'a> {
            *self.analyzed.lock().unwrap() += 1;
            Box::pin(async {
                Ok(crate::translate::Analysis {
                    summary: "A film".to_owned(),
                    characters: Vec::new(),
                    glossary: Vec::new(),
                })
            })
        }

        fn translate<'a>(
            &'a self,
            request: &'a crate::translate::BlockRequest,
        ) -> crate::translate::TranslateFuture<'a> {
            Box::pin(async move {
                self.go.acquire().await.unwrap().forget();
                Ok(request
                    .context
                    .iter()
                    .filter(|l| request.output_ids.contains(&l.id))
                    .map(|l| crate::translate::Line {
                        id: l.id,
                        text: format!("tr {}", l.text),
                    })
                    .collect())
            })
        }
    }

    fn translated_app(
        head: &[u8],
        eng: &str,
        translator: Arc<GatedTranslator>,
        cache: &std::path::Path,
    ) -> Router {
        let mut srt = std::collections::HashMap::new();
        srt.insert("eng", eng.to_owned());
        let source = Arc::new(LangSubtitles {
            srt,
            searched: Mutex::new(Vec::new()),
        });
        router(
            AppState::new(TOKEN.to_owned())
                .unwrap()
                .with_stream_source(Arc::new(ProbeStream))
                .with_range_reader(Arc::new(ProbeRange(
                    Arc::new(tokio::sync::Notify::new()),
                    head.to_vec(),
                )))
                .with_subtitle_sources(vec![source], "tur".to_owned())
                .with_fallback_lang("eng".to_owned())
                .with_cache_dir(cache.to_owned())
                .with_translations(
                    crate::translations::Translations::new(
                        Box::new(move |_| {
                            Some(translator.clone() as Arc<dyn crate::translate::Translator>)
                        }),
                        TranslateSettings {
                            model: "test/model".to_owned(),
                            reasoning: settings::MODEL_DEFAULT.to_owned(),
                        },
                        "tur".to_owned(),
                        Some(cache.to_owned()),
                    )
                    .unwrap()
                    .with_settings_file(cache.join("settings.json")),
                ),
        )
    }

    /// The menu's languages and the path of each entry's file.
    /// The menu's names, a notice by its kind: every notice has one name,
    /// so its file tells which it is.
    async fn names(app: &Router, list: &Value) -> Vec<String> {
        let mut out = Vec::new();
        for entry in list["subtitles"].as_array().unwrap() {
            let lang = entry["lang"].as_str().unwrap();
            if lang != menu::NOTICE_LANG {
                out.push(lang.to_owned());
                continue;
            }
            let url = entry["url"].as_str().unwrap();
            let path = url.trim_start_matches("http://127.0.0.1:7000").to_owned();
            let srt = call(app, path).await.1;
            let notice = Notice::ALL
                .into_iter()
                .find(|n| n.entry().srt == srt)
                .expect("a notice's file");
            out.push(format!("{notice:?}"));
        }
        out
    }

    async fn menu_of(app: &Router) -> Vec<(String, String)> {
        let uri = format!("/{TOKEN}/subtitles/movie/tt1/filename=A.mkv&videoSize=5.json");
        let (status, body) = call(app, uri).await;
        assert_eq!(status, StatusCode::OK);
        let list: Value = serde_json::from_str(&body).unwrap();
        list["subtitles"]
            .as_array()
            .unwrap()
            .iter()
            .map(|s| {
                let url = s["url"].as_str().unwrap();
                (
                    s["lang"].as_str().unwrap().to_owned(),
                    url.trim_start_matches("http://127.0.0.1:7000").to_owned(),
                )
            })
            .collect()
    }

    #[tokio::test]
    async fn english_fallback_is_translated_and_served_as_far_as_it_got() {
        let (head, fits, _) = film();
        let cache = std::env::temp_dir().join(format!(
            "subsync-translation-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let translator = Arc::new(GatedTranslator {
            go: tokio::sync::Semaphore::new(0),
            analyzed: Mutex::new(0),
        });
        let app = translated_app(&head, &fits, translator.clone(), &cache);
        let menu = menu_of(&app).await;
        let langs: Vec<&str> = menu.iter().map(|(l, _)| l.as_str()).collect();
        // A translation on offer is no error: no notice beside it.
        assert_eq!(langs, vec!["Türkçe çeviri", "eng"]);
        let (english, translation) = (&menu[1].1, &menu[0].1);
        let english = srt::parse(&call(&app, english.clone()).await.1);

        // Nothing translated yet: the English, with a note on the first line.
        let served = srt::parse(&call(&app, translation.clone()).await.1);
        assert_eq!(served.len(), english.len());
        assert!(served[0].text.starts_with("[SubSync: çeviri %0,"));
        assert_eq!(served[1].text, english[1].text);

        // Every block let through: the whole translation, on the same times.
        translator.go.add_permits(1_000);
        let done = tokio::time::timeout(std::time::Duration::from_secs(5), async {
            loop {
                let body = call(&app, translation.clone()).await.1;
                if !body.contains("SubSync") {
                    break srt::parse(&body);
                }
                tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap();
        assert_eq!(done.len(), english.len());
        for (got, en) in done.iter().zip(&english) {
            assert_eq!((got.start_ms, got.end_ms), (en.start_ms, en.end_ms));
            assert_eq!(got.text, format!("tr {}", en.text));
        }
        assert_eq!(*translator.analyzed.lock().unwrap(), 1, "one job");

        // After a restart the translation comes from disk, untranslated again.
        let fresh = Arc::new(GatedTranslator {
            go: tokio::sync::Semaphore::new(0),
            analyzed: Mutex::new(0),
        });
        let app = translated_app(&head, &fits, fresh.clone(), &cache);
        let menu = menu_of(&app).await;
        assert_eq!(menu[0].0, "Türkçe çeviri");
        let again = tokio::time::timeout(std::time::Duration::from_secs(5), async {
            loop {
                let body = call(&app, menu[0].1.clone()).await.1;
                if !body.contains("SubSync") {
                    break srt::parse(&body);
                }
                tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap();
        assert_eq!(again, done);
        assert_eq!(*fresh.analyzed.lock().unwrap(), 0, "not translated again");
        let _ = std::fs::remove_dir_all(&cache);
    }

    #[tokio::test]
    async fn translation_progress_reaches_the_dashboard_without_text() {
        let (head, fits, _) = film();
        let cache = std::env::temp_dir().join(format!(
            "subsync-translation-status-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let translator = Arc::new(GatedTranslator {
            go: tokio::sync::Semaphore::new(0),
            analyzed: Mutex::new(0),
        });
        let app = translated_app(&head, &fits, translator.clone(), &cache);
        let status = |app: Router| async move {
            let (status, body) = call(&app, format!("/{TOKEN}/translations.json")).await;
            assert_eq!(status, StatusCode::OK);
            body
        };
        let empty: Value = serde_json::from_str(&status(app.clone()).await).unwrap();
        assert_eq!(empty["jobs"], json!([]));
        assert_eq!(empty["lang"], "tur");

        menu_of(&app).await;
        let body = status(app.clone()).await;
        let english = srt::parse(&fits);
        assert!(!body.contains(&english[0].text), "no subtitle text");
        let job = &serde_json::from_str::<Value>(&body).unwrap()["jobs"][0];
        assert_eq!(job["label"], "A.mkv");
        assert!(["queued", "running"].contains(&job["phase"].as_str().unwrap()));
        assert_eq!(job["lines"], english.len());
        assert_eq!(job["lines_done"], 0);

        translator.go.add_permits(1_000);
        let done = tokio::time::timeout(std::time::Duration::from_secs(5), async {
            loop {
                let body: Value = serde_json::from_str(&status(app.clone()).await).unwrap();
                if body["jobs"][0]["phase"] == "done" {
                    break body["jobs"][0].clone();
                }
                tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap();
        assert_eq!(done["lines_done"], english.len());
        assert!(done["started_ms"].as_u64().unwrap() <= done["finished_ms"].as_u64().unwrap());
        assert_eq!(done["from_disk"], false);

        let (status, _) = call(&app, "/wrong-token-123/translations.json".to_owned()).await;
        assert_eq!(status, StatusCode::NOT_FOUND);
        let (_, off) = get(&format!("/{TOKEN}/translations.json")).await;
        assert_eq!(off, Some(Value::Null), "translation off");
        let _ = std::fs::remove_dir_all(&cache);
    }

    async fn post(app: &Router, uri: String, body: &str) -> (StatusCode, Value) {
        let request = Request::post(uri)
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(body.to_owned()))
            .unwrap();
        let response = app.clone().oneshot(request).await.unwrap();
        let status = response.status();
        let bytes = response.into_body().collect().await.unwrap().to_bytes();
        (
            status,
            serde_json::from_slice(&bytes).unwrap_or(Value::Null),
        )
    }

    #[tokio::test]
    async fn dashboard_changes_the_translation_settings() {
        let (head, fits, _) = film();
        let cache = std::env::temp_dir().join(format!(
            "subsync-settings-route-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let translator = Arc::new(GatedTranslator {
            go: tokio::sync::Semaphore::new(0),
            analyzed: Mutex::new(0),
        });
        let app = translated_app(&head, &fits, translator, &cache);
        let uri = format!("/{TOKEN}/settings.json");

        let (status, body) = call(&app, uri.clone()).await;
        assert_eq!(status, StatusCode::OK);
        let view: Value = serde_json::from_str(&body).unwrap();
        assert_eq!(
            view["current"],
            json!({ "model": "test/model", "reasoning": "default" })
        );
        assert_eq!(view["defaults"]["model"], crate::openrouter::DEFAULT_MODEL);
        assert!(view["efforts"].as_array().unwrap().contains(&json!("low")));

        let refused = [
            (r#"{"model":"nope","reasoning":"low"}"#, "model"),
            (r#"{"model":"a/b","reasoning":"huge"}"#, "reasoning"),
            (r#"{"model":"a/b"}"#, "body"),
            ("not json", "body"),
        ];
        for (body, why) in refused {
            let (status, answer) = post(&app, uri.clone(), body).await;
            assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
            assert_eq!(answer["refused"], why, "{body}");
        }

        let (status, answer) = post(
            &app,
            uri.clone(),
            r#"{"model":"vendor/other","reasoning":"high"}"#,
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(answer["current"]["model"], "vendor/other");
        let (_, body) = call(&app, format!("/{TOKEN}/translations.json")).await;
        let status_now: Value = serde_json::from_str(&body).unwrap();
        assert_eq!(status_now["model"], "vendor/other reasoning=high");
        let saved = std::fs::read_to_string(cache.join("settings.json")).unwrap();
        assert!(saved.contains("vendor/other"), "kept for a restart");

        // A wrong token is 404 whatever it sends; without translation, off.
        let wrong = "/wrong-token-123/settings.json".to_owned();
        assert_eq!(call(&app, wrong.clone()).await.0, StatusCode::NOT_FOUND);
        assert_eq!(post(&app, wrong, "not json").await.0, StatusCode::NOT_FOUND);
        let (_, off) = get(&uri).await;
        assert_eq!(off, Some(Value::Null));
        let (status, answer) =
            post(&self::app(), uri, r#"{"model":"a/b","reasoning":"low"}"#).await;
        assert_eq!(status, StatusCode::CONFLICT);
        assert_eq!(answer["refused"], "off");
        let _ = std::fs::remove_dir_all(&cache);
    }

    #[tokio::test]
    async fn no_translator_means_no_translation_entry() {
        let (head, fits, _) = film();
        let mut srt = std::collections::HashMap::new();
        srt.insert("eng", fits);
        let source = Arc::new(LangSubtitles {
            srt,
            searched: Mutex::new(Vec::new()),
        });
        let app = router(
            AppState::new(TOKEN.to_owned())
                .unwrap()
                .with_stream_source(Arc::new(ProbeStream))
                .with_range_reader(Arc::new(ProbeRange(
                    Arc::new(tokio::sync::Notify::new()),
                    head,
                )))
                .with_subtitle_sources(vec![source], "tur".to_owned())
                .with_fallback_lang("eng".to_owned()),
        );
        let langs: Vec<String> = menu_of(&app).await.into_iter().map(|(l, _)| l).collect();
        assert_eq!(langs, vec![menu::NOTICE_LANG, "eng"]);
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
            translated: false,
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

        let before = crate::events::since(0).next;
        let (status, body) = call(&app, format!("/{TOKEN}/sub/0/1.srt")).await;
        assert_eq!((status, body.as_str()), (StatusCode::OK, SRT));
        let fetched = format!("subtitle fetched entry=0 n=1 bytes={}", SRT.len());
        assert!(
            crate::events::since(before)
                .events
                .iter()
                .any(|e| e.area == "fetch" && e.line == fetched)
        );
        let response = app
            .clone()
            .oneshot(
                Request::get(format!("/{TOKEN}/sub/0/1.srt"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.headers()[header::CACHE_CONTROL], "no-store");
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
