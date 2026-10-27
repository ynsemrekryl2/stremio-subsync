//! Ranking (M3): how close a subtitle's release name is to the played file.
//! A subtitle made for the same release often needs no fix at all; CAM and
//! telesync releases come last whatever their name.

use std::collections::BTreeSet;

const CAM_PENALTY: i32 = 200;

const EXTENSIONS: [&str; 9] = [
    ".mkv", ".mp4", ".avi", ".m4v", ".srt", ".sub", ".ass", ".ssa", ".zip",
];

/// The name without a known video, subtitle or archive extension.
pub fn strip_extension(name: &str) -> &str {
    EXTENSIONS
        .iter()
        .find_map(|ext| {
            let cut = name.len().checked_sub(ext.len())?;
            (name.is_char_boundary(cut) && name[cut..].eq_ignore_ascii_case(ext))
                .then(|| &name[..cut])
        })
        .unwrap_or(name)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Origin {
    BluRay,
    Web,
    Tv,
    Dvd,
}

/// Parts of a release name that tell releases apart.
struct Release {
    words: BTreeSet<String>,
    group: Option<String>,
    origin: Option<Origin>,
    resolution: Option<&'static str>,
    cam: bool,
}

impl Release {
    fn new(raw: &str) -> Self {
        let name = strip_extension(raw.rsplit(['/', '\\']).next().unwrap_or(raw));
        let words: BTreeSet<String> = name
            .split(|c: char| !c.is_alphanumeric())
            .filter(|w| !w.is_empty())
            .map(str::to_lowercase)
            .collect();
        let has = |list: &[&str]| list.iter().any(|w| words.contains(*w));
        let origin = if has(&["bluray", "bdrip", "brrip", "bdremux", "remux", "bd"]) {
            Some(Origin::BluRay)
        } else if has(&[
            "web", "webrip", "webdl", "amzn", "nf", "dsnp", "hmax", "atvp", "itunes",
        ]) {
            Some(Origin::Web)
        } else if has(&["hdtv", "pdtv", "sdtv"]) {
            Some(Origin::Tv)
        } else if has(&["dvdrip", "dvd", "dvd5", "dvd9"]) {
            Some(Origin::Dvd)
        } else {
            None
        };
        let resolution = ["2160p", "1080p", "720p", "576p", "480p"]
            .into_iter()
            .find(|r| words.contains(*r))
            .or_else(|| words.contains("4k").then_some("2160p"));
        let group = name
            .rsplit_once('-')
            .map(|(_, tail)| {
                tail.chars()
                    .filter(|c| c.is_alphanumeric())
                    .collect::<String>()
                    .to_lowercase()
            })
            .filter(|g| !g.is_empty() && !matches!(g.as_str(), "dl" | "rip" | "web"));
        let cam = has(&[
            "cam", "camrip", "hdcam", "ts", "hdts", "telesync", "tc", "hdtc", "telecine", "scr",
            "screener", "dvdscr",
        ]);
        Self {
            words,
            group,
            origin,
            resolution,
            cam,
        }
    }
}

/// Higher is closer. The share of the file name's words the release also
/// has counts up to 100, the same release group 40, the same origin (BluRay,
/// WEB, TV, DVD) 20 and resolution 5. A CAM or telesync release loses more
/// than any match earns.
pub fn score(release: &str, filename: Option<&str>) -> i32 {
    let rel = Release::new(release);
    let mut score = if rel.cam { -CAM_PENALTY } else { 0 };
    let Some(file) = filename.map(Release::new) else {
        return score;
    };
    let shared = rel.words.intersection(&file.words).count();
    score += (100 * shared / file.words.len().max(1)) as i32;
    if rel.group.is_some() && rel.group == file.group {
        score += 40;
    }
    if rel.origin.is_some() && rel.origin == file.origin {
        score += 20;
    }
    if rel.resolution.is_some() && rel.resolution == file.resolution {
        score += 5;
    }
    score
}

#[cfg(test)]
mod tests {
    use super::*;

    const FILE: &str = "The.Matrix.1999.1080p.BluRay.x264-AMIABLE.mkv";

    #[test]
    fn same_release_beats_same_origin_beats_other() {
        let same = score("The.Matrix.1999.1080p.BluRay.x264-AMIABLE", Some(FILE));
        let origin = score("The.Matrix.1999.720p.BRRip.x264-YIFY", Some(FILE));
        let web = score("The.Matrix.1999.1080p.WEB-DL.DD5.1.H264-FGT", Some(FILE));
        let bare = score("Matrix, The (1999)", Some(FILE));
        assert!(
            same > origin && origin > web && web > bare,
            "{same} {origin} {web} {bare}"
        );
        assert_eq!(same, 100 + 40 + 20 + 5);
    }

    #[test]
    fn cam_releases_rank_last() {
        let cam = score("The.Matrix.1999.HDCAM.x264-AMIABLE", Some(FILE));
        let ts = score("The Matrix 1999 TS XviD", None);
        let other = score("Something.Else.2001.DVDRip", Some(FILE));
        assert!(cam < other, "{cam} {other}");
        assert_eq!(ts, -CAM_PENALTY);
        assert_eq!(score("The Matrix", None), 0);
    }

    #[test]
    fn group_ignores_brackets_and_web_dl_tail() {
        assert_eq!(
            Release::new("Dune.Part.Two.2024.1080p.WEBRip.x264.AAC-[YTS.MX]").group,
            Some("ytsmx".to_owned())
        );
        assert_eq!(Release::new("Show.S01E01.WEB-DL").group, None);
        assert_eq!(
            Release::new("Show.S01E01.1080p.WEB-DL.DD5.1.H.264-NTb.mkv").origin,
            Some(Origin::Web)
        );
    }

    #[test]
    fn strip_extension_is_case_insensitive_and_char_safe() {
        assert_eq!(strip_extension("A.B.MKV"), "A.B");
        assert_eq!(strip_extension("tr.srt"), "tr");
        assert_eq!(strip_extension("no extension"), "no extension");
        assert_eq!(strip_extension("ğ.ş"), "ğ.ş");
    }
}
