//! Source cleanup before translation (SS-057): the accepted English cues
//! lose what should not be translated before they reach the model. A cue
//! with a row that advertises a subtitle site or credits its makers goes
//! whole; marks for the hard of hearing (`[door creaks]`, `(sighs)`,
//! `JOHN:`, rows of only `♪`) and styling other than `<i>`, `<b>` and `<u>`
//! are cut. A cue left with nothing is dropped; times stay the source's.
//!
//! The patterns stay narrow: parentheses go only at a row's start or end or
//! when written in capitals, speaker names only in capitals, and a row that
//! had no letters to begin with (`...`) is kept. Only counts leave here.

use serde::Serialize;

use crate::srt::Cue;
use crate::translate::visible;

/// Part of the translation key while cleanup is on: other rules are
/// another translation.
pub const VERSION: u32 = 1;

/// What a cleanup took out. Counts only.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct Cleaned {
    /// Cues dropped as an advert or a credit.
    pub ads: usize,
    /// Marks for the hard of hearing cut (brackets, parentheses, speaker
    /// names, music rows).
    pub marks: usize,
    /// Styling tags cut.
    pub tags: usize,
    /// Cues left with nothing once their marks were cut.
    pub emptied: usize,
}

/// Row texts that only an advert or a credit has, matched in lower case.
const AD_PHRASES: [&str; 26] = [
    "subtitles by",
    "subtitle by",
    "subtitled by",
    "subtitles:",
    "synced by",
    "sync by",
    "synced and corrected",
    "sync and correct",
    "sync & correct",
    "resync by",
    "ripped by",
    "captioning by",
    "captioned by",
    "captions by",
    "captioning made possible",
    "captioning sponsored",
    "transcript by",
    "opensubtitles",
    "addic7ed",
    "subscene",
    "podnapisi",
    "yify",
    "become vip",
    "advertise your product",
    "rate this subtitle",
    "downloaded from",
];

/// Endings of a bare site name (`site.com`) in a row.
const SITE_ENDINGS: [&str; 14] = [
    "com", "org", "net", "tv", "io", "to", "info", "cc", "ws", "me", "co", "biz", "xyz", "site",
];

/// Styling tags kept; the model keeps them where they belong.
const KEPT_TAGS: [&str; 3] = ["i", "b", "u"];

pub fn clean(cues: &[Cue]) -> (Vec<Cue>, Cleaned) {
    let mut counts = Cleaned::default();
    let mut out = Vec::with_capacity(cues.len());
    for cue in cues {
        if cue.text.lines().any(is_ad) {
            counts.ads += 1;
            continue;
        }
        let text = clean_cue(&cue.text, &mut counts);
        if text.is_empty() {
            counts.emptied += 1;
        } else {
            out.push(Cue {
                text,
                ..cue.clone()
            });
        }
    }
    (out, counts)
}

fn is_ad(row: &str) -> bool {
    let shown = visible(row).to_lowercase();
    AD_PHRASES.iter().any(|phrase| shown.contains(phrase))
        || shown.contains("www.")
        || shown.contains("http://")
        || shown.contains("https://")
        || shown.split_whitespace().any(is_site)
}

/// `name.com` and the like, with letters or digits before the dot.
fn is_site(word: &str) -> bool {
    let word = word.trim_matches(|c: char| !c.is_alphanumeric());
    let Some((name, ending)) = word.rsplit_once('.') else {
        return false;
    };
    let name = name.rsplit('.').next().unwrap_or(name);
    name.chars().filter(|c| c.is_alphanumeric()).count() >= 2 && SITE_ENDINGS.contains(&ending)
}

fn clean_cue(text: &str, counts: &mut Cleaned) -> String {
    let rows_in = text.lines().count();
    let mut rows: Vec<String> = Vec::new();
    // Tags of a dropped row, given to the next row kept.
    let mut carried = String::new();
    let mut dropped_row = false;
    let mut in_bracket = false;
    for row in text.lines() {
        let mut cut = cut_tags(row, counts);
        cut = cut_brackets(&cut, &mut in_bracket, counts);
        // A name opens the row before a mark does, or the other way round.
        cut = cut_speaker(&cut, counts);
        cut = cut_parens(&cut, counts);
        cut = cut_speaker(&cut, counts);
        let music = is_music(&cut);
        if music {
            counts.marks += 1;
        }
        let had_words = visible(row).chars().any(char::is_alphanumeric);
        let has_words = visible(&cut).chars().any(char::is_alphanumeric);
        if music || (had_words && !has_words) {
            carried.push_str(&tags_only(&cut));
            dropped_row = true;
            continue;
        }
        let changed = cut != row;
        let tidy = tidy(&cut, changed);
        rows.push(format!("{}{tidy}", std::mem::take(&mut carried)));
    }
    if let Some(last) = rows.last_mut() {
        last.push_str(&carried);
    }
    if dropped_row && rows_in > 1 && rows.len() == 1 {
        rows[0] = strip_dash(&rows[0]);
    }
    let joined = drop_empty_pairs(&rows.join("\n"));
    if visible(&joined).trim().is_empty() {
        String::new()
    } else {
        joined
    }
}

/// Cuts `{...}` overrides and tags other than [`KEPT_TAGS`].
fn cut_tags(row: &str, counts: &mut Cleaned) -> String {
    let mut out = String::with_capacity(row.len());
    let mut rest = row;
    while let Some(ch) = rest.chars().next() {
        let tag_len = match ch {
            '{' => rest.find('}').map(|end| end + 1),
            '<' => tag_len(rest),
            _ => None,
        };
        match tag_len {
            Some(len) if ch == '{' || !kept_tag(&rest[..len]) => {
                counts.tags += 1;
                rest = &rest[len..];
            }
            Some(len) => {
                out.push_str(&rest[..len]);
                rest = &rest[len..];
            }
            None => {
                out.push(ch);
                rest = &rest[ch.len_utf8()..];
            }
        }
    }
    out
}

/// The length of the `<tag ...>` that `text` opens with, if it does: a
/// letter after `<` or `</`, and `>` before any other `<`.
fn tag_len(text: &str) -> Option<usize> {
    let name = text[1..].strip_prefix('/').unwrap_or(&text[1..]);
    if !name.starts_with(|c: char| c.is_ascii_alphabetic()) {
        return None;
    }
    let end = text[1..].find(['>', '<'])? + 1;
    text[end..].starts_with('>').then_some(end + 1)
}

fn kept_tag(tag: &str) -> bool {
    let name: String = tag
        .trim_start_matches(['<', '/'])
        .chars()
        .take_while(char::is_ascii_alphabetic)
        .collect();
    KEPT_TAGS.contains(&name.to_ascii_lowercase().as_str())
}

/// Cuts `[...]`, which may run on to the next row of the cue.
fn cut_brackets(row: &str, open: &mut bool, counts: &mut Cleaned) -> String {
    let mut out = String::with_capacity(row.len());
    for ch in row.chars() {
        if *open {
            *open = ch != ']';
        } else if ch == '[' {
            *open = true;
            counts.marks += 1;
        } else {
            out.push(ch);
        }
    }
    out
}

/// Cuts `(...)` at the row's start or end, or written in capitals.
fn cut_parens(row: &str, counts: &mut Cleaned) -> String {
    let mut out = row.to_owned();
    let mut from = 0;
    while let Some(open) = out[from..].find('(').map(|i| i + from) {
        let Some(close) = out[open..].find(')').map(|i| i + open) else {
            break;
        };
        let leading = visible(&out[..open])
            .chars()
            .all(|c| c.is_whitespace() || c == '-');
        let trailing = !visible(&out[close + 1..])
            .chars()
            .any(char::is_alphanumeric);
        if leading || trailing || shouted(&out[open + 1..close]) {
            out.replace_range(open..=close, "");
            counts.marks += 1;
            from = open;
        } else {
            from = close + 1;
        }
    }
    out
}

/// Two letters or more, none of them lower case.
fn shouted(text: &str) -> bool {
    let letters: Vec<char> = text.chars().filter(|c| c.is_alphabetic()).collect();
    letters.len() >= 2 && !letters.iter().any(|c| c.is_lowercase())
}

/// Cuts a speaker name in capitals opening the row (`JOHN:`, `- MAN 2:`),
/// keeping the dash and tags before it.
fn cut_speaker(row: &str, counts: &mut Cleaned) -> String {
    let start = lead_len(row);
    let rest = &row[start..];
    let Some(colon) = rest.find(':') else {
        return row.to_owned();
    };
    let name = &rest[..colon];
    let after = &rest[colon + 1..];
    let name_like = name.chars().count() <= 30
        && name
            .chars()
            .all(|c| c.is_alphanumeric() || matches!(c, ' ' | '.' | '\'' | '#' | '-' | '&'))
        && shouted(name);
    if !name_like || !(after.is_empty() || after.starts_with(char::is_whitespace)) {
        return row.to_owned();
    }
    counts.marks += 1;
    format!("{}{}", &row[..start], after.trim_start())
}

/// How much of `row` is tags, spaces and dashes before its first word.
fn lead_len(row: &str) -> usize {
    let mut at = 0;
    while let Some(ch) = row[at..].chars().next() {
        if ch == '<'
            && let Some(len) = tag_len(&row[at..])
        {
            at += len;
        } else if ch.is_whitespace() || ch == '-' {
            at += ch.len_utf8();
        } else {
            break;
        }
    }
    at
}

/// A row of only music notes (and punctuation): no lyrics to translate.
fn is_music(row: &str) -> bool {
    let shown = visible(row);
    shown.contains(['♪', '♫', '#']) && !shown.chars().any(char::is_alphanumeric)
}

/// The `<...>` tags of `row`, for a row dropped around them.
fn tags_only(row: &str) -> String {
    let mut out = String::new();
    let mut at = 0;
    while let Some(open) = row[at..].find('<').map(|i| i + at) {
        match tag_len(&row[open..]) {
            Some(len) => {
                out.push_str(&row[open..open + len]);
                at = open + len;
            }
            None => at = open + 1,
        }
    }
    out
}

/// One space between words; after a cut, none before punctuation.
fn tidy(row: &str, changed: bool) -> String {
    let mut out = row.split_whitespace().collect::<Vec<_>>().join(" ");
    if changed {
        for mark in [" ,", " .", " !", " ?", " ;", " :"] {
            out = out.replace(mark, &mark[1..]);
        }
    }
    out
}

/// A dialogue dash left on the only row of a cue.
fn strip_dash(row: &str) -> String {
    let start = lead_len(row);
    let lead: String = row[..start]
        .chars()
        .filter(|c| *c != '-' && !c.is_whitespace())
        .collect();
    let lead = if lead.contains('<') {
        tags_only(&row[..start])
    } else {
        lead
    };
    format!("{lead}{}", &row[start..])
}

/// `<i></i>` and the like, left when everything between was cut.
fn drop_empty_pairs(text: &str) -> String {
    let mut out = text.to_owned();
    for tag in KEPT_TAGS {
        out = out.replace(&format!("<{tag}></{tag}>"), "");
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cues(texts: &[&str]) -> Vec<Cue> {
        texts
            .iter()
            .zip(0u64..)
            .map(|(text, i)| Cue {
                start_ms: 1_000 + i * 2_000,
                end_ms: 2_000 + i * 2_000,
                text: (*text).to_owned(),
            })
            .collect()
    }

    fn texts(cues: &[Cue]) -> Vec<&str> {
        cues.iter().map(|c| c.text.as_str()).collect()
    }

    #[test]
    fn adverts_and_credits_drop_the_whole_cue() {
        let source = cues(&[
            "Subtitles by\n-=Team=-",
            "Hello there.",
            "Support us and become VIP member\nto remove all ads from opensubtitles.org",
            "Sync and corrections by someone",
            "Visit Site.Com today",
            "www.example.net",
            "Goodbye.",
        ]);
        let (out, counts) = clean(&source);
        assert_eq!(texts(&out), vec!["Hello there.", "Goodbye."]);
        assert_eq!(counts.ads, 5);
        assert_eq!(
            (out[1].start_ms, out[1].end_ms),
            (13_000, 14_000),
            "times kept"
        );
    }

    #[test]
    fn hearing_impaired_marks_are_cut() {
        let source = cues(&[
            "[door creaks]",
            "(sighs) I know.",
            "JOHN: Where were you?",
            "- MAN 2: Over here!\n- (LAUGHS) Sure.",
            "I said (WHISPERING) quiet.",
            "Wait for it (chuckles)",
            "JOHN: (sighs) Fine.",
            "(ON RADIO) MAN: Copy that.",
            "[man speaking\nindistinctly]",
            "♪ ♪",
            "# #",
        ]);
        let (out, counts) = clean(&source);
        assert_eq!(
            texts(&out),
            vec![
                "I know.",
                "Where were you?",
                "- Over here!\n- Sure.",
                "I said quiet.",
                "Wait for it",
                "Fine.",
                "Copy that.",
            ]
        );
        assert_eq!(counts.emptied, 4);
        assert_eq!(counts.marks, 14);
    }

    #[test]
    fn a_lone_line_left_of_a_dialogue_loses_its_dash() {
        let (out, _) = clean(&cues(&["- [sighs]\n- Yes.", "<i>- [laughs]\n- No.</i>"]));
        assert_eq!(texts(&out), vec!["Yes.", "<i>No.</i>"]);
    }

    #[test]
    fn styling_other_than_italics_bold_and_underline_is_cut() {
        let (out, counts) = clean(&cues(&[
            "{\\an8}<font color=\"#ffff00\">Up here</font>",
            "<i>Still</i> <b>here</b>",
            "I <3 you",
        ]));
        assert_eq!(
            texts(&out),
            vec!["Up here", "<i>Still</i> <b>here</b>", "I <3 you"]
        );
        assert_eq!(counts.tags, 3);
    }

    #[test]
    fn italics_around_a_cut_row_stay_balanced() {
        let (out, _) = clean(&cues(&["<i>[thunder]\nRun!</i>", "<i>(sighs)</i>"]));
        assert_eq!(texts(&out), vec!["<i>Run!</i>"]);
    }

    #[test]
    fn plain_speech_lyrics_and_bare_punctuation_stay() {
        let source = cues(&[
            "Mr. Smith said no.",
            "♪ We will rock you ♪",
            "...",
            "I told him (and he agreed) to go.",
            "Look: it's fine.",
            "It's 10:30 already.",
            "I: you know, me.",
            "U.S. Army.",
        ]);
        let (out, counts) = clean(&source);
        assert_eq!(out, source);
        assert_eq!(counts, Cleaned::default());
    }
}
