//! Timing lines of an SRT file, read into ruler events. Subtitle text is
//! skipped line by line and never kept.

use crate::ruler::Event;

/// Counts from one parse. Safe to log: no text.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct SrtReport {
    /// Lines holding `-->`.
    pub timing_lines: usize,
    /// Timing lines whose times could not be read, or that end before they start.
    pub invalid: usize,
    pub events: usize,
}

/// `HH:MM:SS,mmm`, also with `.` before the fraction or a fraction of other
/// than three digits (`,5` is 500 ms, `.97` is 970 ms).
fn parse_time(raw: &str) -> Option<u64> {
    let mut parts = raw.trim().split(':');
    let (h, m, rest) = (parts.next()?, parts.next()?, parts.next()?);
    if parts.next().is_some() {
        return None;
    }
    let (s, fraction) = match rest.split_once([',', '.']) {
        Some((s, f)) => (s, f),
        None => (rest, ""),
    };
    let number = |text: &str| -> Option<u64> {
        let text = text.trim();
        (!text.is_empty() && text.bytes().all(|b| b.is_ascii_digit()))
            .then(|| text.parse().ok())
            .flatten()
    };
    let (h, m, s) = (number(h)?, number(m)?, number(s)?);
    if m >= 60 || s >= 60 {
        return None;
    }
    let ms = if fraction.is_empty() {
        0
    } else {
        let digits: String = fraction.chars().take(3).collect();
        let value = number(&digits)?;
        value * 10u64.pow(3 - u32::try_from(digits.len()).ok()?)
    };
    Some(((h * 60 + m) * 60 + s) * 1000 + ms)
}

/// Reads `start --> end` pairs. Anything after the end time (position
/// coordinates some files carry) is ignored.
pub fn parse(text: &str) -> (SrtReport, Vec<Event>) {
    let mut report = SrtReport::default();
    let mut events = Vec::new();
    for line in text.lines() {
        let Some((left, right)) = line.split_once("-->") else {
            continue;
        };
        report.timing_lines += 1;
        let end = right.split_whitespace().next().and_then(parse_time);
        match (parse_time(left), end) {
            (Some(start_ms), Some(end_ms)) if end_ms > start_ms => {
                events.push(Event { start_ms, end_ms });
            }
            _ => report.invalid += 1,
        }
    }
    events.sort_unstable();
    events.dedup();
    report.events = events.len();
    (report, events)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn event(start_ms: u64, end_ms: u64) -> Event {
        Event { start_ms, end_ms }
    }

    #[test]
    fn times_in_common_spellings() {
        assert_eq!(parse_time("00:01:51,978"), Some(111_978));
        assert_eq!(parse_time(" 01:00:00.500 "), Some(3_600_500));
        assert_eq!(parse_time("0:00:02,5"), Some(2_500));
        assert_eq!(parse_time("00:00:02.97"), Some(2_970));
        assert_eq!(parse_time("00:00:03"), Some(3_000));
        assert_eq!(parse_time("00:00:03,1234"), Some(3_123));
        for bad in [
            "",
            "00:61:00,000",
            "00:00:60,000",
            "1:2",
            "a:b:c",
            "00:00:-1,000",
        ] {
            assert_eq!(parse_time(bad), None, "{bad}");
        }
    }

    #[test]
    fn reads_crlf_bom_and_coordinates() {
        let text = "\u{feff}1\r\n00:00:01,000 --> 00:00:02,500\r\nHello\r\n\r\n\
                    2\r\n00:00:04,000 --> 00:00:05,000 X1:10 X2:20\r\nWorld\r\n";
        let (report, events) = parse(text);
        assert_eq!(events, vec![event(1_000, 2_500), event(4_000, 5_000)]);
        assert_eq!(
            report,
            SrtReport {
                timing_lines: 2,
                invalid: 0,
                events: 2
            }
        );
    }

    #[test]
    fn invalid_lines_are_counted_and_order_restored() {
        let text = "00:00:09,000 --> 00:00:10,000\n\
                    00:00:05,000 --> 00:00:04,000\n\
                    garbage --> 00:00:01,000\n\
                    00:00:01,000 --> 00:00:02,000\n\
                    00:00:01,000 --> 00:00:02,000\n";
        let (report, events) = parse(text);
        assert_eq!(events, vec![event(1_000, 2_000), event(9_000, 10_000)]);
        assert_eq!(report.timing_lines, 5);
        assert_eq!(report.invalid, 2);
        assert_eq!(report.events, 2);
    }

    #[test]
    fn non_srt_text_has_no_events() {
        let (report, events) = parse("{100}{200}MicroDVD line\n[Script Info]\n");
        assert!(events.is_empty());
        assert_eq!(report, SrtReport::default());
    }

    #[test]
    fn report_never_holds_text() {
        let (report, _) = parse("00:00:01,000 --> 00:00:02,000\nSecret line\n");
        assert!(!format!("{report:?}").contains("Secret"));
    }
}
