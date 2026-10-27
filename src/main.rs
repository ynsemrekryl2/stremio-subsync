mod addon;
mod extra;
mod mkv;
mod range;
mod rank;
mod ruler;
mod sources;
mod srt;
mod streams;

use std::net::SocketAddr;

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
    // Lists and downloads subtitles for every request, so it is opt-in until
    // the results are served.
    if std::env::var("SUBSYNC_SOURCES_PROBE").as_deref() == Ok("1") {
        let languages = languages(&std::env::var("SUBSYNC_LANGUAGES").unwrap_or_default());
        let source = sources::OpenSubtitlesV3::new().unwrap_or_else(|| {
            eprintln!("cannot build the subtitle source HTTP client");
            std::process::exit(1);
        });
        eprintln!(
            "sources probe: on (languages {}, top {})",
            languages.join(","),
            addon::SOURCES_LIMIT
        );
        state = state.with_subtitle_source(std::sync::Arc::new(source), languages);
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

/// Comma-separated ISO 639-2 codes; Turkish when none is given.
fn languages(raw: &str) -> Vec<String> {
    let list: Vec<String> = raw
        .split(',')
        .map(|code| code.trim().to_ascii_lowercase())
        .filter(|code| code.len() == 3 && code.bytes().all(|b| b.is_ascii_lowercase()))
        .collect();
    if list.is_empty() {
        vec!["tur".to_owned()]
    } else {
        list
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn languages_default_to_turkish() {
        assert_eq!(languages(""), ["tur"]);
        assert_eq!(languages(" TUR, eng ,x,english"), ["tur", "eng"]);
    }
}
