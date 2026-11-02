//! Kitsu ids (what anime catalogs in Stremio send) as IMDb ids, season and
//! episode, so the subtitle sources can be searched for anime too.
//!
//! The map comes from Fribb's anime-lists (`anime-list-full.json`, MIT
//! licensed data): about 8 000 Kitsu titles carry an IMDb id, many with the
//! TVDB season and an episode offset for a season split into parts. Kept in
//! the cache directory and fetched again once a week. Titles without an
//! IMDb id are not found. A wrong season or episode costs nothing more than
//! no subtitle: a candidate for another episode does not fit the ruler.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};
use std::time::Duration;

use serde::Deserialize;

use crate::events::event;

const LIST_URL: &str =
    "https://raw.githubusercontent.com/Fribb/anime-lists/master/anime-list-full.json";
const FILE_NAME: &str = "anime-list-full.json";
/// The list is about 8 MB; anything far larger is not it.
const MAX_LIST: usize = 48 * 1024 * 1024;
/// A saved list older than this is fetched again.
const MAX_AGE: Duration = Duration::from_secs(7 * 24 * 3600);
/// Between checks of the list's age, and after a failed fetch.
const CHECK_EVERY: Duration = Duration::from_secs(6 * 3600);
const FETCH_TIMEOUT: Duration = Duration::from_secs(120);

/// One Kitsu title.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Title {
    imdb: String,
    movie: bool,
    /// TVDB season; none for a show numbered in one run.
    season: Option<u32>,
    /// Added to Kitsu's episode number (a second part of a season); a few
    /// are negative.
    offset: i64,
}

/// The Kitsu map, empty until a list is read.
#[derive(Default)]
pub struct AnimeIds {
    titles: RwLock<Arc<HashMap<u32, Title>>>,
}

impl AnimeIds {
    pub fn len(&self) -> usize {
        self.titles.read().map(|t| t.len()).unwrap_or(0)
    }

    fn replace(&self, titles: HashMap<u32, Title>) {
        if let Ok(mut current) = self.titles.write() {
            *current = Arc::new(titles);
        }
    }

    /// `kitsu:N` (a film) or `kitsu:N:E` (episode E) as the Stremio type and
    /// id of the same title on IMDb: `("movie", "tt…")` or
    /// `("series", "tt…:S:E")`. `None` for any other id, or a title the map
    /// does not know.
    pub fn resolve(&self, id: &str) -> Option<(&'static str, String)> {
        let mut parts = id.strip_prefix("kitsu:")?.split(':');
        let kitsu: u32 = parts.next()?.parse().ok()?;
        let episode: Option<u32> = match parts.next() {
            Some(e) => Some(e.parse().ok()?),
            None => None,
        };
        if parts.next().is_some() {
            return None;
        }
        let titles = self.titles.read().ok()?.clone();
        let title = titles.get(&kitsu)?;
        match (title.movie, episode) {
            (true, _) => Some(("movie", title.imdb.clone())),
            (false, Some(e)) if e > 0 => {
                let season = title.season.unwrap_or(1);
                let episode = u32::try_from(i64::from(e) + title.offset)
                    .ok()
                    .filter(|&e| e > 0)?;
                Some(("series", format!("{}:{season}:{episode}", title.imdb)))
            }
            (false, _) => None,
        }
    }
}

#[derive(Deserialize)]
struct Entry {
    kitsu_id: Option<u32>,
    #[serde(rename = "type")]
    kind: Option<String>,
    imdb_id: Option<Vec<String>>,
    season: Option<Numbers>,
    episode_offset: Option<Numbers>,
}

#[derive(Deserialize)]
struct Numbers {
    tvdb: Option<i64>,
}

fn is_imdb(id: &str) -> bool {
    id.strip_prefix("tt")
        .is_some_and(|d| !d.is_empty() && d.bytes().all(|b| b.is_ascii_digit()))
}

/// Kitsu titles with an IMDb id in Fribb's list; `None` when it is not
/// that list. An entry of another shape is skipped, not the whole list.
fn parse(body: &[u8]) -> Option<HashMap<u32, Title>> {
    let entries: Vec<serde_json::Value> = serde_json::from_slice(body).ok()?;
    let titles: HashMap<u32, Title> = entries
        .into_iter()
        .filter_map(|e| serde_json::from_value::<Entry>(e).ok())
        .filter_map(|e| {
            let imdb = e.imdb_id?.into_iter().find(|id| is_imdb(id))?;
            Some((
                e.kitsu_id?,
                Title {
                    imdb,
                    movie: e.kind.as_deref() == Some("MOVIE"),
                    season: e
                        .season
                        .and_then(|s| s.tvdb)
                        .and_then(|s| u32::try_from(s).ok()),
                    offset: e.episode_offset.and_then(|o| o.tvdb).unwrap_or(0),
                },
            ))
        })
        .collect();
    // An empty map would only mean a broken list.
    (!titles.is_empty()).then_some(titles)
}

/// Reads the saved list, then keeps it fresh for as long as the server
/// runs. Without a cache directory the list lives in memory only and is
/// fetched at every start.
pub async fn keep_fresh(ids: Arc<AnimeIds>, dir: Option<PathBuf>) {
    let file = dir.map(|d| d.join(FILE_NAME));
    if let Some(file) = &file
        && let Some(titles) = read_saved(file).await
    {
        ids.replace(titles);
        event!(
            "anime",
            "anime ids: {} kitsu titles (saved list)",
            ids.len()
        );
    }
    loop {
        let stale = match &file {
            Some(file) => age(file).await.is_none_or(|a| a > MAX_AGE),
            None => ids.len() == 0,
        };
        if stale {
            match fetch().await {
                Ok((body, titles)) => {
                    ids.replace(titles);
                    event!("anime", "anime ids: {} kitsu titles (fetched)", ids.len());
                    if let Some(file) = &file {
                        save(file, body).await;
                    }
                }
                Err(why) => event!("anime", "anime ids: fetch failed error={why}"),
            }
        }
        tokio::time::sleep(CHECK_EVERY).await;
    }
}

async fn read_saved(file: &Path) -> Option<HashMap<u32, Title>> {
    let file = file.to_owned();
    tokio::task::spawn_blocking(move || parse(&std::fs::read(file).ok()?))
        .await
        .ok()
        .flatten()
}

async fn age(file: &Path) -> Option<Duration> {
    let file = file.to_owned();
    tokio::task::spawn_blocking(move || {
        std::fs::metadata(file)
            .ok()?
            .modified()
            .ok()?
            .elapsed()
            .ok()
    })
    .await
    .ok()
    .flatten()
}

async fn save(file: &Path, body: Vec<u8>) {
    let file = file.to_owned();
    let saved = tokio::task::spawn_blocking(move || {
        if let Some(dir) = file.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let part = file.with_extension("json.part");
        std::fs::write(&part, body)?;
        std::fs::rename(&part, &file)
    })
    .await;
    if !matches!(saved, Ok(Ok(()))) {
        event!("anime", "anime ids: list not saved");
    }
}

/// The list and its map. Errors name a cause, never the URL.
async fn fetch() -> Result<(Vec<u8>, HashMap<u32, Title>), &'static str> {
    let client = reqwest::Client::builder()
        .timeout(FETCH_TIMEOUT)
        .user_agent(concat!("stremio-subsync/", env!("CARGO_PKG_VERSION")))
        .build()
        .map_err(|_| "client")?;
    let mut response = client
        .get(LIST_URL)
        .send()
        .await
        .and_then(reqwest::Response::error_for_status)
        .map_err(|_| "request")?;
    let mut body = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(|_| "read")? {
        body.extend_from_slice(&chunk);
        if body.len() > MAX_LIST {
            return Err("too large");
        }
    }
    let parsed = tokio::task::spawn_blocking(move || parse(&body).map(|t| (body, t)))
        .await
        .map_err(|_| "parse")?;
    parsed.ok_or("not the list")
}

#[cfg(test)]
pub mod tests {
    use super::*;

    pub const LIST: &str = r#"[
        { "type": "TV", "kitsu_id": 1, "imdb_id": ["tt0213338"],
          "season": { "tvdb": 1, "tmdb": 1 } },
        { "type": "TV", "kitsu_id": 8671, "imdb_id": ["tt2560140"],
          "season": { "tvdb": 2, "tmdb": 2 }, "episode_offset": null },
        { "type": "TV", "kitsu_id": 500, "imdb_id": ["tt1111111"],
          "season": { "tvdb": 3 }, "episode_offset": { "tvdb": 12, "tmdb": 12 } },
        { "type": "TV", "kitsu_id": 12, "imdb_id": ["tt0388629"] },
        { "type": "TV", "kitsu_id": 600, "imdb_id": ["tt2222222"],
          "season": { "tvdb": 1 }, "episode_offset": { "tvdb": -2 } },
        { "type": "TV", "kitsu_id": "odd", "imdb_id": "tt3" },
        { "type": "MOVIE", "kitsu_id": 142, "imdb_id": ["nm1", "tt0119698"],
          "themoviedb_id": { "movie": [128] } },
        { "type": "OVA", "kitsu_id": 7, "imdb_id": null },
        { "type": "TV", "mal_id": 5, "imdb_id": ["tt5"] }
    ]"#;

    /// A map with [`LIST`] in it.
    pub fn loaded() -> Arc<AnimeIds> {
        let ids = Arc::new(AnimeIds::default());
        ids.replace(parse(LIST.as_bytes()).unwrap());
        ids
    }

    #[test]
    fn list_keeps_kitsu_titles_with_an_imdb_id() {
        let titles = parse(LIST.as_bytes()).unwrap();
        assert_eq!(titles.len(), 6, "the odd entry is skipped");
        assert_eq!(
            titles[&142].imdb, "tt0119698",
            "the first IMDb id that is one"
        );
        assert!(titles[&142].movie);
        assert_eq!((titles[&500].season, titles[&500].offset), (Some(3), 12));
        assert_eq!(titles[&12].season, None);
        assert_eq!(parse(b"[]"), None);
        assert_eq!(parse(b"{\"not\":\"a list\"}"), None);
    }

    #[test]
    fn kitsu_ids_become_imdb_season_and_episode() {
        let ids = loaded();
        let r = |id: &str| ids.resolve(id);
        assert_eq!(r("kitsu:1:5"), Some(("series", "tt0213338:1:5".to_owned())));
        assert_eq!(r("kitsu:8671:3"), Some(("series", "tt2560140:2:3".into())));
        assert_eq!(r("kitsu:500:1"), Some(("series", "tt1111111:3:13".into())));
        // One long run: season 1, Kitsu's number.
        assert_eq!(
            r("kitsu:12:1000"),
            Some(("series", "tt0388629:1:1000".into()))
        );
        assert_eq!(r("kitsu:600:5"), Some(("series", "tt2222222:1:3".into())));
        assert_eq!(r("kitsu:142"), Some(("movie", "tt0119698".into())));
        assert_eq!(r("kitsu:142:1"), Some(("movie", "tt0119698".into())));
        for unknown in [
            "kitsu:7:1",
            "kitsu:999:1",
            "kitsu:1",
            "kitsu:1:0",
            "kitsu:600:2",
            "kitsu:1:x",
            "kitsu:1:2:3",
            "kitsu:",
            "tt0213338:1:5",
            "mal:5:1",
        ] {
            assert_eq!(r(unknown), None, "{unknown}");
        }
        assert_eq!(AnimeIds::default().resolve("kitsu:1:5"), None);
    }

    #[tokio::test]
    async fn saved_list_is_read_back() {
        let dir = std::env::temp_dir().join(format!("subsync-anime-{}", std::process::id()));
        let file = dir.join(FILE_NAME);
        save(&file, LIST.as_bytes().to_vec()).await;
        let titles = read_saved(&file).await.unwrap();
        assert_eq!(titles.len(), 6);
        assert!(age(&file).await.unwrap() < MAX_AGE);
        let _ = std::fs::remove_dir_all(dir);
    }
}
