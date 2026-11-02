//! Subtitle text in any format a source or an archive gives (SRT, WebVTT,
//! ASS/SSA) as cues. The format is told from the text, not the file name.
//! Cue text keeps only line breaks and `<i>`, `<b>`, `<u>`; everything the
//! SRT writer cannot carry (positions, styles, karaoke) is dropped.

use crate::srt::{self, Cue};

/// Cues of `text`, in start order.
pub fn cues(text: &str) -> Vec<Cue> {
    let head = text.trim_start_matches('\u{feff}').trim_start();
    if head.starts_with("WEBVTT") {
        webvtt(head)
    } else if is_ass(head) {
        ass(head)
    } else {
        srt::parse(text)
    }
}

fn is_ass(text: &str) -> bool {
    let first = text.lines().next().unwrap_or("").trim();
    first.eq_ignore_ascii_case("[Script Info]")
        || first.eq_ignore_ascii_case("[V4+ Styles]")
        || first.eq_ignore_ascii_case("[V4 Styles]")
        || first.eq_ignore_ascii_case("[Events]")
}

// --- WebVTT ---------------------------------------------------------------

/// Blocks separated by blank lines; a block is a cue when it has a timing
/// line (an identifier may come before it). `NOTE`, `STYLE` and `REGION`
/// blocks have none and are skipped.
fn webvtt(text: &str) -> Vec<Cue> {
    let mut cues = Vec::new();
    let mut block: Vec<&str> = Vec::new();
    for line in text.lines().chain(std::iter::once("")) {
        let line = line.trim_end_matches('\r');
        if !line.trim().is_empty() {
            block.push(line);
            continue;
        }
        if let Some(at) = block.iter().position(|l| l.contains("-->"))
            && let Some((start_ms, end_ms)) = vtt_timing(block[at])
            && end_ms > start_ms
        {
            let body: Vec<String> = block[at + 1..].iter().map(|l| vtt_text(l)).collect();
            let body = body.join("\n");
            if !body.trim().is_empty() {
                cues.push(Cue {
                    start_ms,
                    end_ms,
                    text: body.trim().to_owned(),
                });
            }
        }
        block.clear();
    }
    cues.sort_by_key(|c| c.start_ms);
    cues
}

/// `00:01.000 --> 00:02.500 align:start`: hours are optional in WebVTT.
fn vtt_timing(line: &str) -> Option<(u64, u64)> {
    let (start, end) = line.split_once("-->")?;
    let end = end.split_whitespace().next()?;
    Some((vtt_time(start.trim())?, vtt_time(end)?))
}

fn vtt_time(text: &str) -> Option<u64> {
    let (clock, fraction) = text.split_once('.')?;
    if fraction.len() != 3 || !fraction.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let parts: Vec<u64> = clock
        .split(':')
        .map(|p| {
            (!p.is_empty() && p.bytes().all(|b| b.is_ascii_digit()))
                .then(|| p.parse().ok())
                .flatten()
        })
        .collect::<Option<_>>()?;
    let (h, m, s) = match parts.as_slice() {
        [m, s] => (0, *m, *s),
        [h, m, s] => (*h, *m, *s),
        _ => return None,
    };
    if m >= 60 || s >= 60 {
        return None;
    }
    Some(((h * 60 + m) * 60 + s) * 1000 + fraction.parse::<u64>().ok()?)
}

/// Keeps `<i>`, `<b>`, `<u>`; drops voice, class and timestamp tags and
/// reads the few entities WebVTT uses.
fn vtt_text(line: &str) -> String {
    let mut out = String::new();
    let mut rest = line;
    while let Some(open) = rest.find('<') {
        out.push_str(&rest[..open]);
        let Some(close) = rest[open..].find('>') else {
            out.push_str(&rest[open..]);
            rest = "";
            break;
        };
        let tag = &rest[open..open + close + 1];
        let name = tag
            .trim_start_matches(['<', '/'])
            .split(['.', ' ', '>'])
            .next()
            .unwrap_or("");
        if matches!(name, "i" | "b" | "u") {
            let slash = if tag.starts_with("</") { "/" } else { "" };
            out.push_str(&format!("<{slash}{name}>"));
        }
        rest = &rest[open + close + 1..];
    }
    out.push_str(rest);
    out.replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&nbsp;", " ")
        .replace("&lrm;", "")
        .replace("&rlm;", "")
        .replace("&amp;", "&")
}

// --- ASS / SSA ------------------------------------------------------------

/// `Dialogue:` lines of the `[Events]` section, read with its `Format:`
/// line (the standard one when it is missing). Drawings are skipped.
fn ass(text: &str) -> Vec<Cue> {
    let mut cues = Vec::new();
    let mut in_events = false;
    let mut format: Vec<String> = ["layer", "start", "end", "style", "name"]
        .into_iter()
        .chain(["marginl", "marginr", "marginv", "effect", "text"])
        .map(str::to_owned)
        .collect();
    for line in text.lines() {
        let line = line.trim_end_matches('\r').trim_start();
        if line.starts_with('[') {
            in_events = line.trim().eq_ignore_ascii_case("[Events]");
            continue;
        }
        if !in_events {
            continue;
        }
        let Some((key, value)) = line.split_once(':') else {
            continue;
        };
        if key.eq_ignore_ascii_case("Format") {
            format = value
                .split(',')
                .map(|f| f.trim().to_ascii_lowercase())
                .collect();
            continue;
        }
        if !key.eq_ignore_ascii_case("Dialogue") || format.is_empty() {
            continue;
        }
        // Text is the last field and may hold commas.
        let fields: Vec<&str> = value.splitn(format.len(), ',').collect();
        let field = |name: &str| {
            format
                .iter()
                .position(|f| f == name)
                .and_then(|i| fields.get(i))
                .map(|f| f.trim())
        };
        let (Some(start), Some(end), Some(raw)) = (field("start"), field("end"), field("text"))
        else {
            continue;
        };
        let (Some(start_ms), Some(end_ms)) = (ass_time(start), ass_time(end)) else {
            continue;
        };
        if end_ms <= start_ms {
            continue;
        }
        if let Some(text) = ass_text(raw) {
            cues.push(Cue {
                start_ms,
                end_ms,
                text,
            });
        }
    }
    cues.sort_by_key(|c| c.start_ms);
    cues
}

/// `h:mm:ss.cc` (centiseconds).
fn ass_time(text: &str) -> Option<u64> {
    let (clock, fraction) = text.split_once('.')?;
    let mut parts = clock.split(':');
    let (h, m, s) = (parts.next()?, parts.next()?, parts.next()?);
    if parts.next().is_some() || fraction.is_empty() || fraction.len() > 3 {
        return None;
    }
    let number = |p: &str| -> Option<u64> {
        (!p.is_empty() && p.bytes().all(|b| b.is_ascii_digit()))
            .then(|| p.parse().ok())
            .flatten()
    };
    let (h, m, s, frac) = (number(h)?, number(m)?, number(s)?, number(fraction)?);
    if m >= 60 || s >= 60 {
        return None;
    }
    let ms = frac * 10u64.pow(3 - fraction.len() as u32);
    Some(((h * 60 + m) * 60 + s) * 1000 + ms)
}

/// Override blocks `{...}` go, except italic, bold and underline switches;
/// `\N` and `\n` break the line, `\h` is a space. `None` for a drawing or
/// an empty line.
fn ass_text(raw: &str) -> Option<String> {
    let mut out = String::new();
    let mut open: Vec<&str> = Vec::new();
    let mut rest = raw;
    while let Some(start) = rest.find('{') {
        out.push_str(&rest[..start]);
        let Some(len) = rest[start..].find('}') else {
            rest = "";
            break;
        };
        let block = &rest[start + 1..start + len];
        for tag in block.split('\\').skip(1) {
            let tag = tag.trim();
            if let Some(level) = tag.strip_prefix('p')
                && level.parse::<u32>().is_ok_and(|n| n > 0)
            {
                return None;
            }
            for name in ["i", "b", "u"] {
                match tag.strip_prefix(name) {
                    Some("1") if !open.contains(&name) => {
                        out.push_str(&format!("<{name}>"));
                        open.push(name);
                    }
                    Some("0") if open.contains(&name) => {
                        out.push_str(&format!("</{name}>"));
                        open.retain(|n| *n != name);
                    }
                    _ => {}
                }
            }
        }
        rest = &rest[start + len + 1..];
    }
    out.push_str(rest);
    for name in open.iter().rev() {
        out.push_str(&format!("</{name}>"));
    }
    let text = out
        .replace("\\N", "\n")
        .replace("\\n", "\n")
        .replace("\\h", " ");
    let lines: Vec<&str> = text
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .collect();
    (!lines.is_empty()).then(|| lines.join("\n"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn shown(cues: &[Cue]) -> Vec<(u64, u64, &str)> {
        cues.iter()
            .map(|c| (c.start_ms, c.end_ms, c.text.as_str()))
            .collect()
    }

    #[test]
    fn srt_is_read_as_before() {
        let text = "1\r\n00:00:01,000 --> 00:00:02,000\r\nHi\r\n\r\n";
        assert_eq!(cues(text), srt::parse(text));
        assert_eq!(shown(&cues(text)), [(1000, 2000, "Hi")]);
    }

    #[test]
    fn webvtt_with_identifiers_notes_and_short_times() {
        let text = "\u{feff}WEBVTT - film\n\nSTYLE\n::cue { color: red }\n\n\
            NOTE a comment\nthat --> is not a cue\n\n\
            intro\n00:01.500 --> 00:03.000 align:start\n<v Neo>Who <i>are</i> you?</v>\n\
            <c.yellow>Tom &amp; Jerry</c>\n\n\
            01:00:00.000 --> 01:00:01.250\n&lt;3\n\n\
            00:05.000 --> 00:04.000\nbackwards\n";
        assert_eq!(
            shown(&cues(text)),
            [
                (1500, 3000, "Who <i>are</i> you?\nTom & Jerry"),
                (3_600_000, 3_601_250, "<3"),
            ]
        );
    }

    #[test]
    fn ass_dialogue_with_format_overrides_and_commas() {
        let text = "[Script Info]\nTitle: x\n\n[V4+ Styles]\nFormat: Name, Fontname\n\
            Style: Default,Arial\n\n[Events]\n\
            Format: Layer, Start, End, Style, Name, MarginL, MarginR, MarginV, Effect, Text\n\
            Dialogue: 0,0:00:02.50,0:00:04.00,Default,,0,0,0,,{\\an8\\i1}Hello,{\\i0} world\\Nsecond\n\
            Comment: 0,0:00:05.00,0:00:06.00,Default,,0,0,0,,hidden\n\
            Dialogue: 0,0:00:01.00,0:00:02.00,Default,,0,0,0,,{\\p1}m 0 0 l 100 0{\\p0}\n\
            Dialogue: 0,1:02:03.04,1:02:04.00,Default,,0,0,0,,{\\b1}Bold\\hend\n\
            Dialogue: 0,0:00:07.00,0:00:07.00,Default,,0,0,0,,zero length\n";
        assert_eq!(
            shown(&cues(text)),
            [
                (2500, 4000, "<i>Hello,</i> world\nsecond"),
                (3_723_040, 3_724_000, "<b>Bold end</b>"),
            ]
        );
    }

    #[test]
    fn ssa_with_its_own_field_order() {
        let text = "[Script Info]\nScriptType: v4.00\n\n[Events]\n\
            Format: Marked, Start, End, Style, Name, MarginL, MarginR, MarginV, Effect, Text\n\
            Dialogue: Marked=0,0:00:10.00,0:00:11.50,Default,NTP,0000,0000,0000,,Old style\n";
        assert_eq!(shown(&cues(text)), [(10_000, 11_500, "Old style")]);
    }
}
