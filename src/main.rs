mod addon;
// Wired into the subtitles route in M5.
#[cfg_attr(not(test), allow(dead_code))]
mod align;
mod extra;
mod mkv;
mod range;
mod rank;
mod ruler;
mod sources;
// Wired into the subtitles route in M5.
#[cfg_attr(not(test), allow(dead_code))]
mod srt;
mod streams;
#[cfg(test)]
mod synth;

use std::net::SocketAddr;
use std::sync::Arc;

use sources::SubtitleSource;

/// Minimum length for the secret path token. The install link is the only
/// thing protecting the server, so short tokens are refused.
const MIN_TOKEN_LEN: usize = 32;

#[tokio::main]
async fn main() {
    let token = std::env::var("SUBSYNC_TOKEN").unwrap_or_default();
    if token.len() < MIN_TOKEN_LEN {
        eprintln!("SUBSYNC_TOKEN must be set to at least {MIN_TOKEN_LEN} characters");
        std::process::exit(2);
    }
    let bind: SocketAddr = std::env::var("SUBSYNC_BIND")
        .unwrap_or_else(|_| "127.0.0.1:7000".to_owned())
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
            eprintln!("aiostreams probe: on");
            // Reading stream URLs touches the debrid service, so it is opt-in.
            if std::env::var("SUBSYNC_RANGE_PROBE").as_deref() == Ok("1") {
                let reader = range::HttpRangeReader::new().unwrap_or_else(|| {
                    eprintln!("cannot build the range probe HTTP client");
                    std::process::exit(1);
                });
                state = state.with_range_reader(std::sync::Arc::new(reader));
                eprintln!("range probe: on ({} bytes per match)", range::HEAD_BYTES);
            } else {
                eprintln!("range probe: off");
            }
        }
        _ => eprintln!("aiostreams probe: off"),
    }
    let lang = std::env::var("SUBSYNC_LANG").unwrap_or_else(|_| "tur".to_owned());
    if lang.len() != 3 || !lang.bytes().all(|b| b.is_ascii_lowercase()) {
        eprintln!("SUBSYNC_LANG must be a three-letter ISO 639-2 code such as tur");
        std::process::exit(2);
    }
    // Searching and downloading subtitles uses the providers' quota, so it is opt-in.
    if std::env::var("SUBSYNC_SOURCES_PROBE").as_deref() == Ok("1") {
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
        eprintln!("sources probe: on (lang {lang}, {kinds:?})");
        state = state.with_subtitle_sources(found, lang);
    } else {
        eprintln!("sources probe: off");
    }
    let listener = tokio::net::TcpListener::bind(bind)
        .await
        .unwrap_or_else(|err| {
            eprintln!("cannot bind {bind}: {err}");
            std::process::exit(1);
        });
    // The token is deliberately not printed: it is the install link's secret.
    eprintln!("listening on {bind}");
    axum::serve(listener, addon::router(state))
        .with_graceful_shutdown(async {
            let _ = tokio::signal::ctrl_c().await;
        })
        .await
        .expect("server error");
}
