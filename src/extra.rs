//! Parsing of the `extra` path segment Stremio appends to subtitle requests.
//!
//! Stremio sends it as a URL-encoded query string, for example
//! `filename=Movie.2160p.mkv&videoSize=123&videoHash=abcd`.

/// What the player told us about the file being played.
///
/// `Debug` is implemented by hand: the filename may reveal what the user is
/// watching, so it is never printed.
#[derive(Clone, Default, PartialEq, Eq)]
pub struct PlaybackHints {
    pub filename: Option<String>,
    pub video_size: Option<u64>,
    pub video_hash: Option<String>,
}

impl std::fmt::Debug for PlaybackHints {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PlaybackHints")
            .field("has_filename", &self.filename.is_some())
            .field("has_video_size", &self.video_size.is_some())
            .field("has_video_hash", &self.video_hash.is_some())
            .finish()
    }
}

impl PlaybackHints {
    /// Parses the raw segment, with or without its trailing `.json`.
    /// Unknown keys are ignored; malformed values are treated as absent.
    pub fn parse(segment: &str) -> Self {
        let segment = segment.strip_suffix(".json").unwrap_or(segment);
        let mut hints = Self::default();
        for (key, value) in form_urlencoded::parse(segment.as_bytes()) {
            let value = value.trim();
            if value.is_empty() {
                continue;
            }
            match key.as_ref() {
                "filename" => hints.filename = Some(value.to_owned()),
                "videoSize" => hints.video_size = value.parse().ok(),
                "videoHash" => hints.video_hash = Some(value.to_owned()),
                _ => {}
            }
        }
        hints
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_all_known_fields() {
        let hints = PlaybackHints::parse(
            "filename=Movie.2160p.WEB-DL.mkv&videoSize=4294967296&videoHash=8e245d9679d31e12.json",
        );
        assert_eq!(hints.filename.as_deref(), Some("Movie.2160p.WEB-DL.mkv"));
        assert_eq!(hints.video_size, Some(4_294_967_296));
        assert_eq!(hints.video_hash.as_deref(), Some("8e245d9679d31e12"));
    }

    #[test]
    fn decodes_percent_encoded_filename() {
        let hints = PlaybackHints::parse("filename=The%20Movie%20(2024).mkv");
        assert_eq!(hints.filename.as_deref(), Some("The Movie (2024).mkv"));
    }

    #[test]
    fn malformed_size_and_empty_values_are_absent() {
        let hints = PlaybackHints::parse("videoSize=12.5&videoHash=&other=1");
        assert_eq!(hints, PlaybackHints::default());
    }

    #[test]
    fn debug_never_prints_filename() {
        let hints = PlaybackHints::parse("filename=Secret.Title.mkv&videoSize=1");
        let printed = format!("{hints:?}");
        assert!(!printed.contains("Secret"), "{printed}");
        assert!(printed.contains("has_filename: true"), "{printed}");
    }
}
