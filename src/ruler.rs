//! Ruler (M2): when, and for how long, the file's own English text subtitle
//! shows each line, taken from Cues. Only times are kept; subtitle text is
//! never read.

/// One cue point of one track, in Matroska ticks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CueRef {
    pub time: u64,
    /// `CueDuration`: how long the referenced block lasts.
    pub duration: Option<u64>,
    /// `CueRelativePosition`: where the block sits inside its cluster.
    pub relative: Option<u64>,
}

/// One subtitle line on screen, in milliseconds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Event {
    pub start_ms: u64,
    pub end_ms: u64,
}

/// Counts and timing shape of a ruler. Safe to log: no text, no names.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RulerReport {
    pub track: u64,
    pub cue_refs: usize,
    pub with_duration: usize,
    pub with_relative: usize,
    /// Distinct events built from cue points that carry a duration.
    pub events: usize,
    pub first_ms: Option<u64>,
    pub last_end_ms: Option<u64>,
    pub median_ms: Option<u64>,
    /// Events that start before the previous one ends.
    pub overlaps: usize,
}

fn ticks_to_ms(ticks: u64, scale_ns: u64) -> u64 {
    let ms = u128::from(ticks) * u128::from(scale_ns) / 1_000_000;
    u64::try_from(ms).unwrap_or(u64::MAX)
}

/// Events from cue points with a non-zero duration, sorted, duplicates removed.
pub fn events_from_cues(refs: &[CueRef], scale_ns: u64) -> Vec<Event> {
    let mut events: Vec<Event> = refs
        .iter()
        .filter_map(|r| {
            let duration = r.duration.filter(|&d| d > 0)?;
            Some(Event {
                start_ms: ticks_to_ms(r.time, scale_ns),
                end_ms: ticks_to_ms(r.time.saturating_add(duration), scale_ns),
            })
        })
        .collect();
    events.sort_unstable();
    events.dedup();
    events
}

pub fn build(track: u64, refs: &[CueRef], scale_ns: u64) -> (RulerReport, Vec<Event>) {
    let events = events_from_cues(refs, scale_ns);
    let mut durations: Vec<u64> = events.iter().map(|e| e.end_ms - e.start_ms).collect();
    durations.sort_unstable();
    let overlaps = events
        .windows(2)
        .filter(|pair| pair[1].start_ms < pair[0].end_ms)
        .count();
    let report = RulerReport {
        track,
        cue_refs: refs.len(),
        with_duration: refs.iter().filter(|r| r.duration.is_some()).count(),
        with_relative: refs.iter().filter(|r| r.relative.is_some()).count(),
        events: events.len(),
        first_ms: events.first().map(|e| e.start_ms),
        last_end_ms: events.iter().map(|e| e.end_ms).max(),
        median_ms: durations.get(durations.len() / 2).copied(),
        overlaps,
    };
    (report, events)
}

#[cfg(test)]
mod tests {
    use super::*;

    const MS: u64 = 1_000_000;

    fn cue(time: u64, duration: Option<u64>) -> CueRef {
        CueRef {
            time,
            duration,
            relative: Some(10),
        }
    }

    #[test]
    fn events_use_scale_sort_and_dedup() {
        let refs = [
            cue(5_000, Some(2_000)),
            cue(1_000, Some(1_500)),
            cue(1_000, Some(1_500)),
            cue(9_000, None),
            cue(7_000, Some(0)),
        ];
        assert_eq!(
            events_from_cues(&refs, MS),
            vec![
                Event {
                    start_ms: 1_000,
                    end_ms: 2_500
                },
                Event {
                    start_ms: 5_000,
                    end_ms: 7_000
                },
            ]
        );
        // A coarser scale: one tick is 10 ms.
        assert_eq!(
            events_from_cues(&[cue(100, Some(50))], 10 * MS),
            vec![Event {
                start_ms: 1_000,
                end_ms: 1_500
            }]
        );
    }

    #[test]
    fn report_counts_and_shape() {
        let refs = [
            cue(1_000, Some(2_000)),
            cue(2_500, Some(1_000)),
            cue(6_000, Some(4_000)),
            CueRef {
                time: 12_000,
                duration: None,
                relative: None,
            },
        ];
        let (report, events) = build(3, &refs, MS);
        assert_eq!(events.len(), 3);
        assert_eq!(
            report,
            RulerReport {
                track: 3,
                cue_refs: 4,
                with_duration: 3,
                with_relative: 3,
                events: 3,
                first_ms: Some(1_000),
                last_end_ms: Some(10_000),
                median_ms: Some(2_000),
                overlaps: 1,
            }
        );
    }

    #[test]
    fn huge_values_do_not_overflow() {
        let events = events_from_cues(&[cue(u64::MAX - 1, Some(u64::MAX))], u64::MAX);
        assert_eq!(events[0].end_ms, u64::MAX);
    }
}
