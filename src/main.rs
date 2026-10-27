mod addon;
mod extra;

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

    let state = addon::AppState::new(token).expect("token checked above");
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
