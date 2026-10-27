//! SRT subtitles (M3): bytes to text, cue times, and rewriting with a
//! [`Fix`]. Cue text is kept only to write it back out; `Debug` never
//! prints it.

use encoding_rs::Encoding;
use std::fmt::Write as _;

use crate::align::Fix;
use crate::ruler::Event;

/// One subtitle line on screen.
#[derive(Clone, PartialEq, Eq)]
pub struct Cue {
    pub start_ms: u64,
    pub end_ms: u64,
    pub text: String,
}

impl std::fmt::Debug for Cue {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Cue")
            .field("start_ms", &self.start_ms)
            .field("end_ms", &self.end_ms)
            .field("text_len", &self.text.len())
            .finish()
    }
}

/// Text of a subtitle file: a byte-order mark wins, then UTF-8, then the
/// legacy code page usual for `lang` (ISO 639-2).
pub fn decode(bytes: &[u8], lang: &str) -> String {
    if let Some((encoding, bom)) = Encoding::for_bom(bytes) {
        return encoding
            .decode_without_bom_handling(&bytes[bom..])
            .0
            .into_owned();
    }
    match std::str::from_utf8(bytes) {
        Ok(text) => text.to_owned(),
        Err(_) => legacy_encoding(lang)
            .decode_without_bom_handling(bytes)
            .0
            .into_owned(),
    }
}

fn legacy_encoding(lang: &str) -> &'static Encoding {
    match lang {
        "tur" => encoding_rs::WINDOWS_1254,
        "ell" | "gre" => encoding_rs::WINDOWS_1253,
        "rus" | "ukr" | "bul" | "bel" | "mac" | "mkd" => encoding_rs::WINDOWS_1251,
        "pol" | "cze" | "ces" | "hun" | "hrv" | "slo" | "slk" | "slv" | "rum" | "ron" | "bos" => {
            encoding_rs::WINDOWS_1250
        }
        "ara" | "per" | "fas" => encoding_rs::WINDOWS_1256,
        "heb" => encoding_rs::WINDOWS_1255,
        _ => encoding_rs::WINDOWS_1252,
    }
}

/// Cues in start order. Blocks without a valid timing line or without text
/// are skipped; a missing blank line between cues is tolerated.
pub fn parse(text: &str) -> Vec<Cue> {
    let mut cues = Vec::new();
    let mut current: Option<(u64, u64)> = None;
    let mut body: Vec<&str> = Vec::new();
    for line in text.lines() {
        let line = line.trim_end_matches('\r');
        if let Some(times) = timing(line) {
            // A bare number right above a timing line is that cue's index.
            if body.last().is_some_and(|l| is_index(l)) {
                body.pop();
            }
            push_cue(&mut cues, current.take(), &mut body);
            current = Some(times);
        } else if current.is_some() {
            body.push(line);
        }
    }
    push_cue(&mut cues, current, &mut body);
    cues.sort_by_key(|c: &Cue| c.start_ms);
    cues
}

fn push_cue(cues: &mut Vec<Cue>, times: Option<(u64, u64)>, body: &mut Vec<&str>) {
    let lines = std::mem::take(body);
    let Some((start_ms, end_ms)) = times.filter(|(s, e)| e > s) else {
        return;
    };
    let first = lines.iter().position(|l| !l.trim().is_empty());
    let last = lines.iter().rposition(|l| !l.trim().is_empty());
    if let (Some(first), Some(last)) = (first, last) {
        cues.push(Cue {
            start_ms,
            end_ms,
            text: lines[first..=last].join("\n"),
        });
    }
}

fn is_index(line: &str) -> bool {
    let line = line.trim();
    !line.is_empty() && line.bytes().all(|b| b.is_ascii_digit())
}

/// `00:01:02,345 --> 00:01:04,000`, with optional position fields after it.
fn timing(line: &str) -> Option<(u64, u64)> {
    let (start, end) = line.split_once("-->")?;
    let end = end.split_whitespace().next()?;
    Some((timestamp(start.trim())?, timestamp(end)?))
}

/// `h:mm:ss,mmm` or `h:mm:ss.mmm`; the fraction may have 1–3 digits.
fn timestamp(text: &str) -> Option<u64> {
    let (clock, fraction) = match text.split_once([',', '.']) {
        Some((clock, fraction)) => (clock, fraction),
        None => (text, "0"),
    };
    let mut parts = clock.split(':');
    let (h, m, s) = (parts.next()?, parts.next()?, parts.next()?);
    if parts.next().is_some() || fraction.is_empty() || fraction.len() > 3 {
        return None;
    }
    let number = |part: &str| -> Option<u64> {
        let part = part.trim();
        (!part.is_empty() && part.bytes().all(|b| b.is_ascii_digit()))
            .then(|| part.parse().ok())
            .flatten()
    };
    let (h, m, s, frac) = (number(h)?, number(m)?, number(s)?, number(fraction)?);
    if m >= 60 || s >= 60 {
        return None;
    }
    let ms = frac * 10u64.pow(3 - fraction.len() as u32);
    Some(((h * 60 + m) * 60 + s) * 1_000 + ms)
}

/// Cue times, for alignment.
pub fn events(cues: &[Cue]) -> Vec<Event> {
    cues.iter()
        .map(|c| Event {
            start_ms: c.start_ms,
            end_ms: c.end_ms,
        })
        .collect()
}

/// SRT text with every cue moved by `fix`. Cues that end before the film
/// starts are dropped; one that straddles the start is cut at zero.
pub fn render(cues: &[Cue], fix: Fix) -> String {
    let mut out = String::new();
    let mut index = 0;
    for cue in cues {
        let end = fix.apply(cue.end_ms);
        if end <= 0 {
            continue;
        }
        let start = fix.apply(cue.start_ms).max(0);
        index += 1;
        let _ = write!(
            out,
            "{index}\n{} --> {}\n{}\n\n",
            clock(start),
            clock(end),
            cue.text
        );
    }
    out
}

fn clock(ms: i64) -> String {
    let (h, rest) = (ms / 3_600_000, ms % 3_600_000);
    let (m, rest) = (rest / 60_000, rest % 60_000);
    format!("{h:02}:{m:02}:{:02},{:03}", rest / 1_000, rest % 1_000)
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAME: Fix = Fix {
        num: 1,
        den: 1,
        shift_ms: 0,
    };

    fn cue(start_ms: u64, end_ms: u64, text: &str) -> Cue {
        Cue {
            start_ms,
            end_ms,
            text: text.to_owned(),
        }
    }

    #[test]
    fn parses_plain_srt() {
        let text = "1\n00:00:01,000 --> 00:00:02,500\nHello\nthere\n\n\
                    2\n01:02:03,004 --> 01:02:05,000\nBye\n";
        assert_eq!(
            parse(text),
            vec![
                cue(1_000, 2_500, "Hello\nthere"),
                cue(3_723_004, 3_725_000, "Bye")
            ]
        );
    }

    #[test]
    fn tolerates_crlf_dots_positions_and_missing_blank_lines() {
        let text = "1\r\n00:00:01.5 --> 00:00:02.25 X1:10 X2:20\r\nA\r\n\
                    2\r\n00:00:03,000-->00:00:04,000\r\nB\r\n\r\n\r\n";
        assert_eq!(
            parse(text),
            vec![cue(1_500, 2_250, "A"), cue(3_000, 4_000, "B")]
        );
    }

    #[test]
    fn skips_broken_and_empty_blocks_and_sorts() {
        let text = "1\n00:00:09,000 --> 00:00:10,000\nLate\n\n\
                    2\n00:00:05,000 --> 00:00:04,000\nBackwards\n\n\
                    3\n00:61:00,000 --> 00:62:00,000\nBad minutes\n\n\
                    4\n00:00:06,000 --> 00:00:07,000\n\n\n\
                    5\nnot a timing line\nStray\n\n\
                    6\n00:00:01,000 --> 00:00:02,000\nEarly\n";
        // A block with a bad timing line reads as text of the cue above it,
        // as a player would show it; the backwards cue goes with its text.
        assert_eq!(
            parse(text),
            vec![
                cue(1_000, 2_000, "Early"),
                cue(6_000, 7_000, "5\nnot a timing line\nStray"),
                cue(9_000, 10_000, "Late"),
            ]
        );
    }

    #[test]
    fn decodes_bom_utf8_and_turkish_code_page() {
        assert_eq!(decode("ğış".as_bytes(), "tur"), "ğış");
        assert_eq!(decode(b"\xef\xbb\xbfabc", "tur"), "abc");
        // Windows-1254: ğ ı ş İ
        assert_eq!(decode(b"\xf0\xfd\xfe\xdd", "tur"), "ğışİ");
        assert_eq!(decode(b"\xe9", "fre"), "é");
        let utf16: Vec<u8> = [0xff, 0xfe]
            .into_iter()
            .chain("Şu".encode_utf16().flat_map(u16::to_le_bytes))
            .collect();
        assert_eq!(decode(&utf16, "eng"), "Şu");
    }

    #[test]
    fn render_round_trips_and_applies_fix() {
        let cues = vec![cue(1_000, 2_500, "A\nB"), cue(3_723_004, 3_725_000, "C")];
        assert_eq!(parse(&render(&cues, SAME)), cues);

        let moved = render(
            &cues,
            Fix {
                num: 1,
                den: 1,
                shift_ms: -3_000,
            },
        );
        // The first cue ends before zero and is dropped; numbering restarts.
        assert_eq!(moved, "1\n01:02:00,004 --> 01:02:02,000\nC\n\n");

        let straddle = render(
            &[cue(500, 3_000, "D")],
            Fix {
                num: 1,
                den: 1,
                shift_ms: -1_000,
            },
        );
        assert_eq!(straddle, "1\n00:00:00,000 --> 00:00:02,000\nD\n\n");
    }

    #[test]
    fn debug_hides_text() {
        let shown = format!("{:?}", cue(0, 1, "secret line"));
        assert!(!shown.contains("secret"));
        assert!(shown.contains("text_len: 11"));
    }

    #[test]
    fn events_keep_times() {
        let cues = [cue(1, 2, "x"), cue(3, 4, "y")];
        assert_eq!(
            events(&cues),
            vec![
                Event {
                    start_ms: 1,
                    end_ms: 2
                },
                Event {
                    start_ms: 3,
                    end_ms: 4
                }
            ]
        );
    }
}
