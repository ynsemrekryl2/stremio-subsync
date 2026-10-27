//! Release-name likeness between the played file and a candidate subtitle.
//! Subtitles made for the same release usually need no shift at all, so the
//! closest names are downloaded and tested first.

use std::collections::BTreeSet;

/// Where a release was ripped from. Rips of one source share cuts and frame
/// rates far more often than rips of different sources.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Source {
    BluRay,
    Web,
    Tv,
    Dvd,
}

fn source_of(token: &str) -> Option<Source> {
    Some(match token {
        "bluray" | "bdrip" | "brrip" | "bdremux" | "remux" | "bd" | "blu" => Source::BluRay,
        "web" | "webdl" | "webrip" | "amzn" | "nf" | "dsnp" | "hmax" | "atvp" => Source::Web,
        "hdtv" | "pdtv" | "sdtv" => Source::Tv,
        "dvdrip" | "dvd" | "dvdscr" | "dvd5" | "dvd9" => Source::Dvd,
        _ => return None,
    })
}

/// Tokens and the parts of a release name that say most about timing.
/// `Debug` is by hand: the tokens spell out the title.
#[derive(Default, Clone, PartialEq, Eq)]
pub struct Release {
    tokens: BTreeSet<String>,
    group: Option<String>,
    source: Option<Source>,
}

/// Drops a short trailing extension such as `.mkv` or `.srt`.
fn strip_extension(name: &str) -> &str {
    match name.rsplit_once('.') {
        Some((stem, ext))
            if (2..=4).contains(&ext.len()) && ext.bytes().all(|b| b.is_ascii_alphanumeric()) =>
        {
            stem
        }
        _ => name,
    }
}

fn tokens(name: &str) -> Vec<String> {
    name.split(|c: char| !c.is_alphanumeric())
        .filter(|t| !t.is_empty())
        .map(str::to_lowercase)
        .collect()
}

/// `…x264-GROUP` → `group`. Bracketed or decorated groups (`-=Grp=-`) are
/// read too, since only letters and digits are kept.
fn group_of(stem: &str) -> Option<String> {
    let (_, tail) = stem.rsplit_once('-')?;
    // A dot or space after the dash means it was part of the title
    // (`Spider-Man.2002…`), not a group.
    if tail.contains(['.', ' ', '_']) {
        return None;
    }
    let group: String = tail
        .chars()
        .filter(|c| c.is_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect();
    (2..=24).contains(&group.chars().count()).then_some(group)
}

impl std::fmt::Debug for Release {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Release")
            .field("tokens", &self.tokens.len())
            .field("has_group", &self.group.is_some())
            .field("source", &self.source)
            .finish()
    }
}

impl Release {
    /// Reads a file or release name. `group` and `format` are the provider's
    /// own fields, used when the name itself does not carry them.
    pub fn parse(name: &str, group: Option<&str>, format: Option<&str>) -> Self {
        let stem = strip_extension(name.trim());
        let mut all = tokens(stem);
        // `WEB-DL` and `Blu-ray` are one word for matching.
        let joined: Vec<String> = all.windows(2).map(|w| w.concat()).collect();
        let source = all
            .iter()
            .chain(&joined)
            .chain(format.map(tokens).iter().flatten())
            .find_map(|t| source_of(t));
        let group = group
            .map(|g| tokens(g).concat())
            .filter(|g| !g.is_empty())
            .or_else(|| group_of(stem));
        all.extend(group.clone());
        Self {
            tokens: all.into_iter().collect(),
            group,
            source,
        }
    }

    /// `CD1`, `CD2`, `2CD`…: the file holds only part of the film.
    pub fn is_multi_part(&self) -> bool {
        self.tokens.iter().any(|t| {
            let digit_after = t
                .strip_prefix("cd")
                .is_some_and(|n| n.parse::<u8>().is_ok());
            let count_before = t
                .strip_suffix("cd")
                .and_then(|n| n.parse::<u8>().ok())
                .is_some_and(|n| n >= 2);
            digit_after || count_before
        })
    }

    pub fn is_empty(&self) -> bool {
        self.tokens.is_empty()
    }
}

/// Points, higher is closer. Same group outweighs everything but a hash
/// match; a known, different source costs a little; shared tokens break ties.
pub fn score(file: &Release, candidate: &Release) -> i32 {
    let mut points = 0;
    if file.group.is_some() && file.group == candidate.group {
        points += 300;
    }
    match (file.source, candidate.source) {
        (Some(a), Some(b)) if a == b => points += 200,
        (Some(_), Some(_)) => points -= 100,
        _ => {}
    }
    let shared = file.tokens.intersection(&candidate.tokens).count();
    let union = file.tokens.union(&candidate.tokens).count();
    if let Some(likeness) = (shared * 200).checked_div(union) {
        points += i32::try_from(likeness).unwrap_or(0);
    }
    points
}

#[cfg(test)]
mod tests {
    use super::*;

    fn release(name: &str) -> Release {
        Release::parse(name, None, None)
    }

    #[test]
    fn group_and_source_from_name() {
        let r = release("The.Movie.1994.1080p.BluRay.x264-AMIABLE.mkv");
        assert_eq!(r.group.as_deref(), Some("amiable"));
        assert_eq!(r.source, Some(Source::BluRay));
        assert!(r.tokens.contains("1080p"));
        assert!(!r.tokens.contains("mkv"));

        let r = release("Show.S01E02.Name.1080p.AMZN.WEB-DL.DDP5.1.H.264-NTb.srt");
        assert_eq!(r.group.as_deref(), Some("ntb"));
        assert_eq!(r.source, Some(Source::Web));

        let r = release("Shawshank_Redemption_DVDrip_DivX-=Chiara=-(CD1).srt");
        assert_eq!(r.source, Some(Source::Dvd));
        assert!(r.is_multi_part());
    }

    #[test]
    fn provider_fields_fill_gaps() {
        let r = Release::parse("Movie subtitle.srt", Some("Chiara"), Some("DVD"));
        assert_eq!(r.group.as_deref(), Some("chiara"));
        assert_eq!(r.source, Some(Source::Dvd));
        let r = Release::parse("Movie.Blu-ray.srt", None, None);
        assert_eq!(r.source, Some(Source::BluRay));
        assert_eq!(release("Spider-Man.2002.1080p.BluRay.x264.mkv").group, None);
    }

    #[test]
    fn multi_part_markers() {
        for name in ["Movie.CD1.srt", "Movie (cd2).srt", "Movie.2CD.DVDRip.srt"] {
            assert!(release(name).is_multi_part(), "{name}");
        }
        for name in [
            "Movie.1CD.srt",
            "Movie.CD.srt",
            "Movie.1080p.srt",
            "ACDC.Live.srt",
        ] {
            assert!(!release(name).is_multi_part(), "{name}");
        }
    }

    #[test]
    fn same_release_beats_same_source_beats_other_source() {
        let file = release("The.Movie.1994.1080p.BluRay.x264-AMIABLE.mkv");
        let same = score(
            &file,
            &release("The.Movie.1994.1080p.BluRay.x264-AMIABLE.srt"),
        );
        let other_group = score(
            &file,
            &release("The.Movie.1994.720p.BluRay.x264-SPARKS.srt"),
        );
        let other_source = score(&file, &release("The.Movie.1994.DVDRip.XviD-SPARKS.srt"));
        let bare = score(&file, &release("The Movie.srt"));
        assert!(same > other_group, "{same} {other_group}");
        assert!(other_group > bare, "{other_group} {bare}");
        assert!(bare > other_source, "{bare} {other_source}");
    }

    #[test]
    fn empty_names_score_zero() {
        assert_eq!(score(&release(""), &release("")), 0);
        assert!(release("  ").is_empty());
    }
}
