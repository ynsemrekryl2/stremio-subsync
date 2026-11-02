//! The configure page's Test button (SS-062): tries a link or key once
//! against its service and says whether it works, without saving it. A
//! source key is tried with one search for a well-known film, which counts
//! against that source's daily quota like any search. Neither the value
//! nor the service's answer is logged or returned; only the outcome.

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use serde::Serialize;

use crate::secrets::Name;
use crate::sources::{self, Query, SourceError, SubtitleSource};
use crate::streams::{AioStreams, FetchError};

/// The film a source key is tried with: listed on every source.
const TRIAL_TITLE: &str = "tt0111161";
/// Answers 200 for a working key, 401 for any other (SS-062).
const OPENROUTER_KEY: &str = "https://openrouter.ai/api/v1/key";
const TIMEOUT: std::time::Duration = std::time::Duration::from_secs(15);

/// What a try showed. Carries no value and nothing the service said.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(tag = "result", rename_all = "snake_case")]
pub enum Outcome {
    /// Answered; `found` is how many subtitles the trial search listed.
    Works { found: Option<usize> },
    /// Refused the key or the link (401, 403).
    Invalid,
    /// Too many requests: the daily quota is used up (429).
    Quota,
    /// No answer in time, or no connection.
    Unreachable,
    /// The link answers but is not a stream add-on's manifest.
    NoStreams,
    /// Any other answer: an HTTP status, or none when it could not be read.
    Failed { status: Option<u16> },
}

impl Outcome {
    fn from_status(status: u16) -> Self {
        match status {
            401 | 403 => Outcome::Invalid,
            429 => Outcome::Quota,
            status => Outcome::Failed {
                status: Some(status),
            },
        }
    }

    fn from_source(err: SourceError) -> Self {
        match err {
            SourceError::Status(status) => Self::from_status(status),
            SourceError::Timeout | SourceError::Connect => Outcome::Unreachable,
            _ => Outcome::Failed { status: None },
        }
    }

    fn from_fetch(err: FetchError) -> Self {
        match err {
            FetchError::Status(status) => Self::from_status(status),
            FetchError::Timeout | FetchError::Connect => Outcome::Unreachable,
            _ => Outcome::Failed { status: None },
        }
    }
}

pub type CheckFuture = Pin<Box<dyn Future<Output = Outcome> + Send>>;

/// Tries one link or key; faked in tests.
pub trait Checker: Send + Sync {
    fn check(&self, name: Name, value: String) -> CheckFuture;
}

/// Tries the real services. `lang` is the language the add-on looks for,
/// so a source's trial search is the kind it will run.
pub struct Live {
    lang: String,
}

impl Live {
    pub fn new(lang: String) -> Self {
        Self { lang }
    }
}

impl Checker for Live {
    fn check(&self, name: Name, value: String) -> CheckFuture {
        let lang = self.lang.clone();
        Box::pin(async move {
            match name {
                Name::AioStreams => match AioStreams::from_manifest_url(&value) {
                    Ok(source) => match source.check_manifest().await {
                        Ok(true) => Outcome::Works { found: None },
                        Ok(false) => Outcome::NoStreams,
                        Err(err) => Outcome::from_fetch(err),
                    },
                    Err(_) => Outcome::Failed { status: None },
                },
                Name::OpenRouter => openrouter(&value).await,
                _ => {
                    let source: Option<Arc<dyn SubtitleSource>> = match name {
                        Name::SubDl => sources::SubDl::new(value).map(|s| Arc::new(s) as _),
                        Name::AltyaziDb => sources::AltyaziDb::new(value).map(|s| Arc::new(s) as _),
                        Name::SubSource => sources::SubSource::new(value).map(|s| Arc::new(s) as _),
                        Name::Wyzie => sources::Wyzie::new(value).map(|s| Arc::new(s) as _),
                        Name::AioStreams | Name::OpenRouter => None,
                    };
                    let query = Query::new("movie", TRIAL_TITLE, &lang);
                    match (source, query) {
                        (Some(source), Some(query)) => match source.search(&query).await {
                            Ok(found) => Outcome::Works {
                                found: Some(found.len()),
                            },
                            Err(err) => Outcome::from_source(err),
                        },
                        _ => Outcome::Failed { status: None },
                    }
                }
            }
        })
    }
}

/// OpenRouter's key endpoint: free, and spends no credit.
async fn openrouter(key: &str) -> Outcome {
    let Ok(client) = reqwest::Client::builder()
        .timeout(TIMEOUT)
        .user_agent(concat!("stremio-subsync/", env!("CARGO_PKG_VERSION")))
        .build()
    else {
        return Outcome::Failed { status: None };
    };
    match client.get(OPENROUTER_KEY).bearer_auth(key).send().await {
        Ok(response) if response.status().is_success() => Outcome::Works { found: None },
        Ok(response) => Outcome::from_status(response.status().as_u16()),
        Err(err) if err.is_timeout() || err.is_connect() => Outcome::Unreachable,
        Err(_) => Outcome::Failed { status: None },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn answers_map_to_what_the_page_says() {
        assert_eq!(Outcome::from_status(401), Outcome::Invalid);
        assert_eq!(Outcome::from_status(403), Outcome::Invalid);
        assert_eq!(Outcome::from_status(429), Outcome::Quota);
        assert_eq!(
            Outcome::from_status(500),
            Outcome::Failed { status: Some(500) }
        );
        assert_eq!(
            Outcome::from_source(SourceError::Status(403)),
            Outcome::Invalid
        );
        assert_eq!(
            Outcome::from_source(SourceError::Timeout),
            Outcome::Unreachable
        );
        assert_eq!(
            Outcome::from_source(SourceError::Decode),
            Outcome::Failed { status: None }
        );
        assert_eq!(
            Outcome::from_fetch(FetchError::Connect),
            Outcome::Unreachable
        );
        assert_eq!(Outcome::from_fetch(FetchError::Status(429)), Outcome::Quota);
        let shown = serde_json::to_value(Outcome::Works { found: Some(3) }).unwrap();
        assert_eq!(shown, serde_json::json!({ "result": "works", "found": 3 }));
    }

    #[test]
    fn the_trial_title_is_a_movie_query() {
        assert!(Query::new("movie", TRIAL_TITLE, "tur").is_some());
    }
}
