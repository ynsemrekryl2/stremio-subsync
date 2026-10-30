mod addon;
mod align;
mod blocks;
mod dump;
mod events;
mod extra;
mod label;
mod menu;
mod mkv;
mod mp4;
mod pgs;
mod range;
mod rank;
mod ruler;
mod sources;
mod srt;
mod streams;
#[cfg(test)]
mod synth;

use std::net::SocketAddr;
use std::sync::Arc;

use events::event;
use sources::SubtitleSource;

/// Minimum length for the secret path token. The install link is the only
/// thing protecting the server, so short tokens are refused.
const MIN_TOKEN_LEN: usize = 32;
/// Listen address when `SUBSYNC_BIND` is unset. Port 7000 is avoided because
/// macOS AirPlay Receiver holds it on all interfaces.
const DEFAULT_BIND: &str = "127.0.0.1:7001";

#[tokio::main]
async fn main() {
    // `stremio-subsync calibrate [dir]` replays saved dumps,
    // `stremio-subsync pgs-check [dir]` checks saved PGS times and
    // `stremio-subsync mp4-check <file> [srt]` reads a local MP4; no server.
    let mut args = std::env::args().skip(1);
    let command = args.next();
    if command.as_deref() == Some("mp4-check") {
        let (Some(file), srt) = (args.next(), args.next()) else {
            eprintln!("usage: mp4-check <file> [reference.srt]");
            std::process::exit(2);
        };
        let srt = srt.as_deref().map(std::path::Path::new);
        if let Err(err) = mp4::check(std::path::Path::new(&file), srt).await {
            eprintln!("mp4-check: {err}");
            std::process::exit(1);
        }
        return;
    }
    type Offline = fn(&std::path::Path) -> std::io::Result<()>;
    let offline: Option<(&str, &str, Offline)> = match command.as_deref() {
        Some("calibrate") => Some(("calibrate", "dumps", dump::calibrate)),
        Some("pgs-check") => Some(("pgs-check", "dumps/pgs", pgs::check)),
        _ => None,
    };
    if let Some((name, default_dir, run)) = offline {
        let dir = args.next().unwrap_or_else(|| default_dir.to_owned());
        if let Err(err) = run(std::path::Path::new(&dir)) {
            eprintln!("{name} {dir}: {err}");
            std::process::exit(1);
        }
        return;
    }
    let token = std::env::var("SUBSYNC_TOKEN").unwrap_or_default();
    if token.len() < MIN_TOKEN_LEN {
        eprintln!("SUBSYNC_TOKEN must be set to at least {MIN_TOKEN_LEN} characters");
        std::process::exit(2);
    }
    let bind: SocketAddr = std::env::var("SUBSYNC_BIND")
        .unwrap_or_else(|_| DEFAULT_BIND.to_owned())
        .parse()
        .unwrap_or_else(|_| {
            eprintln!("SUBSYNC_BIND is not a valid socket address");
            std::process::exit(2);
        });

    let mut state = addon::AppState::new(token.clone()).expect("token checked above");
    // The manifest link embeds the AIOStreams configuration; it is never printed.
    match std::env::var("SUBSYNC_AIOSTREAMS_URL") {
        Ok(raw) if !raw.trim().is_empty() => {
            if raw.contains(token.as_str()) {
                eprintln!(
                    "SUBSYNC_AIOSTREAMS_URL refused: this is SubSync's own install link, \
                     not the AIOStreams one"
                );
                std::process::exit(2);
            }
            let source = match streams::AioStreams::from_manifest_url(&raw) {
                Ok(source) => source,
                Err(reason) => {
                    // Only the failed check is printed, never the link.
                    eprintln!("SUBSYNC_AIOSTREAMS_URL refused: {reason}");
                    std::process::exit(2);
                }
            };
            state = state.with_stream_source(std::sync::Arc::new(source));
            event!("start", "aiostreams probe: on");
            // The ruler is read from the stream with Range requests. On by
            // default since M5; `SUBSYNC_RANGE_PROBE=0` turns it off.
            if std::env::var("SUBSYNC_RANGE_PROBE").as_deref() != Ok("0") {
                let reader = range::HttpRangeReader::new().unwrap_or_else(|| {
                    eprintln!("cannot build the range probe HTTP client");
                    std::process::exit(1);
                });
                state = state.with_range_reader(std::sync::Arc::new(reader));
                event!(
                    "start",
                    "range probe: on ({} bytes per match)",
                    range::HEAD_BYTES
                );
                // Reads one small range per subtitle line; off by default.
                if std::env::var("SUBSYNC_BLOCK_PROBE").as_deref() == Ok("1") {
                    state = state.with_block_probe();
                    event!(
                        "start",
                        "block probe: on (up to {} blocks, {} at a time)",
                        blocks::MAX_BLOCKS,
                        blocks::CONCURRENCY
                    );
                }
            } else {
                event!("start", "range probe: off");
            }
        }
        _ => event!("start", "aiostreams probe: off"),
    }
    let lang = std::env::var("SUBSYNC_LANG").unwrap_or_else(|_| "tur".to_owned());
    if lang.len() != 3 || !lang.bytes().all(|b| b.is_ascii_lowercase()) {
        eprintln!("SUBSYNC_LANG must be a three-letter ISO 639-2 code such as tur");
        std::process::exit(2);
    }
    // Candidate search and download. On by default since M5;
    // `SUBSYNC_SOURCES_PROBE=0` turns it off.
    if std::env::var("SUBSYNC_SOURCES_PROBE").as_deref() != Ok("0") {
        let mut found: Vec<Arc<dyn SubtitleSource>> = Vec::new();
        if let Some(source) = sources::OpenSubtitlesV3::new() {
            found.push(Arc::new(source));
        }
        // The key is never printed; only which sources are on.
        if let Ok(key) = std::env::var("SUBDL_API_KEY")
            && !key.trim().is_empty()
            && let Some(source) = sources::SubDl::new(key.trim().to_owned())
        {
            found.push(Arc::new(source));
        }
        let kinds: Vec<_> = found.iter().map(|s| s.kind()).collect();
        event!("start", "sources probe: on (lang {lang}, {kinds:?})");
        state = state.with_subtitle_sources(found, lang);
    } else {
        event!("start", "sources probe: off");
    }
    match std::env::var("SUBSYNC_DUMP_DIR") {
        Ok(dir) if !dir.trim().is_empty() => {
            event!("start", "timing dump: on ({dir})");
            state = state.with_dump_dir(dir.trim().into());
        }
        _ => event!("start", "timing dump: off"),
    }
    if std::env::var("SUBSYNC_LABEL_MODE").as_deref() == Ok("1") {
        event!(
            "start",
            "label mode: on (every candidate listed in Stremio, marked; no disk cache)"
        );
        state = state.with_label_mode();
    }
    // Answered menus survive restarts here; `off` keeps them in memory only.
    let cache_dir = std::env::var("SUBSYNC_CACHE_DIR").unwrap_or_else(|_| "cache".to_owned());
    match cache_dir.trim() {
        "" | "off" => event!("start", "subtitle cache: memory only"),
        dir => {
            event!("start", "subtitle cache: {dir}");
            state = state.with_cache_dir(dir.into());
        }
    }
    let listener = tokio::net::TcpListener::bind(bind)
        .await
        .unwrap_or_else(|err| {
            eprintln!("cannot bind {bind}: {err}");
            if err.kind() == std::io::ErrorKind::AddrInUse {
                eprintln!(
                    "the port is taken by another program (on macOS, AirPlay Receiver \
                     holds 7000); pick a free one with SUBSYNC_BIND=127.0.0.1:<port>"
                );
            }
            std::process::exit(1);
        });
    // The token is deliberately not printed: it is the install link's secret.
    event!("start", "listening on {bind}");
    axum::serve(listener, addon::router(state))
        .with_graceful_shutdown(async {
            let _ = tokio::signal::ctrl_c().await;
        })
        .await
        .expect("server error");
}
