//! Alignment (M4): the frame-rate fix and shift that best lay a candidate
//! subtitle's lines over the ruler, and a test that the same fix holds at the
//! head, middle and end of the film. Works on times only.

use std::time::Instant;

use crate::ruler::Event;

/// Grid step of the overlap search. The final shift is refined to 1 ms.
const STEP_MS: i64 = 10;
/// Shifts searched in each direction for the whole film.
const MAX_SHIFT_MS: i64 = 10 * 60 * 1_000;
/// Shifts searched around the global one for a single window.
const WINDOW_SEARCH_MS: i64 = 60 * 1_000;
/// Head, middle and end.
const WINDOWS: usize = 3;
/// A window with fewer ruler lines than this cannot vouch for the fix.
const MIN_WINDOW_EVENTS: usize = 20;
/// A window's own best shift must land this close to the global one.
const AGREE_MS: i64 = 300;
/// A ruler line is hit when an aligned candidate line starts this close.
const HIT_MS: i64 = 300;
/// Share of lines each window must hit at the global fix, counted on the
/// side with fewer lines: a translation that joins two lines into one can
/// hit at most one of the pair.
const MIN_HIT_RATE: f64 = 0.5;
/// Candidate lines each window must have, as a share of its ruler lines.
/// Keeps a half subtitle (CD1 of two) from passing on its few last lines.
const MIN_COVERAGE: f64 = 0.4;

/// Frame-rate fixes tried, as `time * num / den`: a subtitle timed for one
/// rate laid over a release that plays at another.
pub const SCALES: [(u64, u64); 7] = [
    (1, 1),
    // 25 fps (PAL speed-up) <-> 23.976 fps: 25 / (24000 / 1001).
    (25_025, 24_000),
    (24_000, 25_025),
    // 24 fps <-> 23.976 fps: 24 / (24000 / 1001).
    (1_001, 1_000),
    (1_000, 1_001),
    // 25 fps <-> 24 fps.
    (25, 24),
    (24, 25),
];

/// Maps a candidate time onto the ruler: `ms * num / den + shift_ms`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Fix {
    pub num: u64,
    pub den: u64,
    pub shift_ms: i64,
}

impl Fix {
    pub fn apply(&self, ms: u64) -> i64 {
        scale(ms, self.num, self.den).saturating_add(self.shift_ms)
    }
}

/// Why a candidate is not served.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reject {
    /// The ruler or the candidate is empty, or no shift makes them overlap.
    NoOverlap,
    /// Too few ruler lines in this window to judge it.
    ThinWindow(usize),
    /// Too few candidate lines in this window for its ruler lines.
    Sparse(usize),
    /// This window's own best shift is somewhere else.
    Disagree(usize),
    /// Too few ruler lines in this window are hit at the fix.
    LowHits(usize),
}

/// One of the head / middle / end checks. Safe to log: numbers only.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct WindowCheck {
    pub ruler_events: usize,
    /// Aligned candidate lines that start in this window.
    pub candidate_events: usize,
    /// The window's own best shift, searched around the global one.
    pub best_shift_ms: Option<i64>,
    /// Hits over the smaller of the two line counts.
    pub hit_rate: f64,
    /// Candidate lines over ruler lines.
    pub coverage: f64,
}

/// Outcome of laying one candidate over the ruler. Safe to log.
#[derive(Debug, Clone, PartialEq)]
pub struct Verdict {
    pub fix: Option<Fix>,
    /// Hits over the smaller of the two line counts, whole film.
    pub hit_rate: f64,
    pub windows: Vec<WindowCheck>,
    pub reject: Option<Reject>,
}

impl Verdict {
    /// The fix, only when every check passed.
    pub fn accepted(&self) -> Option<Fix> {
        self.fix.filter(|_| self.reject.is_none())
    }

    fn rejected(reject: Reject) -> Self {
        Self {
            fix: None,
            hit_rate: 0.0,
            windows: Vec::new(),
            reject: Some(reject),
        }
    }
}

/// Lays `candidate` over `ruler` and decides whether it may be served.
pub fn align(ruler: &[Event], candidate: &[Event]) -> Verdict {
    let mut ruler = ruler.to_vec();
    ruler.sort_unstable();
    if ruler.is_empty() || candidate.is_empty() {
        return Verdict::rejected(Reject::NoOverlap);
    }
    let ruler_spans = spans(&ruler, 1, 1);

    let range = MAX_SHIFT_MS / STEP_MS;
    let mut best: Option<(i64, (u64, u64), i64)> = None;
    for &(num, den) in &SCALES {
        let curve = overlap_curve(&ruler_spans, &spans(candidate, num, den), -range, range);
        if let Some(i) = peak(&curve)
            && best.is_none_or(|(value, _, _)| curve[i] > value)
        {
            best = Some((curve[i], (num, den), i as i64 - range));
        }
    }
    let Some((_, (num, den), ticks)) = best else {
        return Verdict::rejected(Reject::NoOverlap);
    };
    let fix = refine(
        &ruler,
        candidate,
        Fix {
            num,
            den,
            shift_ms: ticks * STEP_MS,
        },
    );
    let starts = aligned_starts(candidate, fix);
    let cand_spans = spans(candidate, num, den);

    let first = ruler[0].start_ms;
    let last = ruler.iter().map(|e| e.end_ms).max().unwrap_or(first);
    let length = (last - first).max(1);
    let center = fix.shift_ms.div_euclid(STEP_MS);
    let reach = WINDOW_SEARCH_MS / STEP_MS;
    let mut windows = Vec::with_capacity(WINDOWS);
    let mut reject = None;
    for w in 0..WINDOWS {
        let lo = first + length * w as u64 / WINDOWS as u64;
        let hi = first + length * (w as u64 + 1) / WINDOWS as u64;
        let part: Vec<Event> = ruler
            .iter()
            .filter(|e| e.start_ms >= lo && (e.start_ms < hi || w + 1 == WINDOWS))
            .copied()
            .collect();
        let curve = overlap_curve(
            &spans(&part, 1, 1),
            &cand_spans,
            center - reach,
            center + reach,
        );
        let best_shift_ms = peak(&curve).map(|i| (center - reach + i as i64) * STEP_MS);
        let (lo_ms, hi_ms) = (lo as i64, hi as i64);
        let candidate_events = starts
            .iter()
            .filter(|&&s| s >= lo_ms && (s < hi_ms || w + 1 == WINDOWS))
            .count();
        let hit_rate = rate(hits(&part, &starts).len(), part.len().min(candidate_events));
        let coverage = rate(candidate_events, part.len());
        windows.push(WindowCheck {
            ruler_events: part.len(),
            candidate_events,
            best_shift_ms,
            hit_rate,
            coverage,
        });
        let failed = if part.len() < MIN_WINDOW_EVENTS {
            Some(Reject::ThinWindow(w))
        } else if coverage < MIN_COVERAGE {
            Some(Reject::Sparse(w))
        } else if best_shift_ms.is_none_or(|s| (s - fix.shift_ms).abs() > AGREE_MS) {
            Some(Reject::Disagree(w))
        } else if hit_rate < MIN_HIT_RATE {
            Some(Reject::LowHits(w))
        } else {
            None
        };
        reject = reject.or(failed);
    }

    Verdict {
        fix: Some(fix),
        hit_rate: rate(
            hits(&ruler, &starts).len(),
            ruler.len().min(candidate.len()),
        ),
        windows,
        reject,
    }
}

/// One ruler track against one downloaded candidate. Safe to log.
#[derive(Debug, Clone, PartialEq)]
pub struct Trial {
    pub track: u64,
    /// Index among the downloads, best ranked first.
    pub candidate: usize,
    pub accepted: bool,
    pub elapsed_ms: u128,
    pub verdict: Verdict,
}

/// Every ruler track against every candidate that could be read.
pub fn trials(rulers: &[(u64, Vec<Event>)], candidates: &[Option<Vec<Event>>]) -> Vec<Trial> {
    let mut out = Vec::new();
    for (track, ruler) in rulers {
        for (candidate, events) in candidates.iter().enumerate() {
            let Some(events) = events else { continue };
            let started = Instant::now();
            let verdict = align(ruler, events);
            out.push(Trial {
                track: *track,
                candidate,
                accepted: verdict.accepted().is_some(),
                elapsed_ms: started.elapsed().as_millis(),
                verdict,
            });
        }
    }
    out
}

fn scale(ms: u64, num: u64, den: u64) -> i64 {
    let scaled = (u128::from(ms) * u128::from(num) + u128::from(den) / 2) / u128::from(den);
    i64::try_from(scaled).unwrap_or(i64::MAX)
}

fn rate(part: usize, whole: usize) -> f64 {
    if whole == 0 {
        0.0
    } else {
        (part as f64 / whole as f64).min(1.0)
    }
}

/// An event on the search grid, at least one step long.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
struct Span {
    start: i64,
    end: i64,
}

fn to_ticks(ms: i64) -> i64 {
    (ms + STEP_MS / 2).div_euclid(STEP_MS)
}

/// Events scaled by `num / den`, on the grid, sorted by start.
fn spans(events: &[Event], num: u64, den: u64) -> Vec<Span> {
    let mut spans: Vec<Span> = events
        .iter()
        .map(|e| {
            let start = to_ticks(scale(e.start_ms, num, den));
            let end = to_ticks(scale(e.end_ms, num, den)).max(start + 1);
            Span { start, end }
        })
        .collect();
    spans.sort_unstable();
    spans
}

/// Total overlap, in grid steps, of `cand` shifted by every `d` in `lo..=hi`
/// with `ruler` (sorted by start).
///
/// One pair's overlap is a trapezoid in `d` with four corners, so each pair
/// adds four slope changes and two running sums rebuild the whole curve.
fn overlap_curve(ruler: &[Span], cand: &[Span], lo: i64, hi: i64) -> Vec<i64> {
    let max_len = ruler
        .iter()
        .chain(cand)
        .map(|s| s.end - s.start)
        .max()
        .unwrap_or(0);
    // Every corner of a pair that touches lo..=hi lies within this margin.
    let base = lo - 2 * max_len;
    let mut bends = vec![0i64; (hi + 2 * max_len - base + 1) as usize];
    for b in cand {
        let from = ruler.partition_point(|a| a.start < b.start + lo - max_len);
        for a in &ruler[from..] {
            if a.start - b.end > hi {
                break;
            }
            if a.end - b.start < lo {
                continue;
            }
            let short = (a.end - a.start).min(b.end - b.start);
            let rise = a.start - b.end;
            let fall = a.end - b.start;
            for (corner, bend) in [(rise, 1), (rise + short, -1), (fall - short, -1), (fall, 1)] {
                bends[(corner - base) as usize] += bend;
            }
        }
    }
    let mut curve = Vec::with_capacity((hi - lo + 1) as usize);
    let (mut slope, mut value) = (0i64, 0i64);
    for (i, bend) in bends.iter().enumerate() {
        let d = base + i as i64;
        if (lo..=hi).contains(&d) {
            curve.push(value);
        }
        slope += bend;
        value += slope;
    }
    curve
}

/// Middle of the first highest plateau, if the curve rises above zero.
fn peak(curve: &[i64]) -> Option<usize> {
    let top = *curve.iter().max()?;
    if top <= 0 {
        return None;
    }
    let first = curve.iter().position(|&v| v == top)?;
    let run = curve[first..].iter().take_while(|&&v| v == top).count();
    Some(first + (run - 1) / 2)
}

/// Candidate starts after `fix`, sorted.
fn aligned_starts(candidate: &[Event], fix: Fix) -> Vec<i64> {
    let mut starts: Vec<i64> = candidate.iter().map(|e| fix.apply(e.start_ms)).collect();
    starts.sort_unstable();
    starts
}

/// For each ruler line with an aligned candidate start within `HIT_MS`, the
/// signed gap from that start to the ruler line's start.
fn hits(ruler: &[Event], starts: &[i64]) -> Vec<i64> {
    ruler
        .iter()
        .filter_map(|e| {
            let t = i64::try_from(e.start_ms).ok()?;
            let i = starts.partition_point(|&s| s < t);
            [i.checked_sub(1), Some(i)]
                .into_iter()
                .flatten()
                .filter_map(|j| starts.get(j))
                .map(|&s| t - s)
                .min_by_key(|gap| gap.abs())
                .filter(|gap| gap.abs() <= HIT_MS)
        })
        .collect()
}

/// Moves the grid shift by the median start gap of the lines it hits.
fn refine(ruler: &[Event], candidate: &[Event], coarse: Fix) -> Fix {
    let mut gaps = hits(ruler, &aligned_starts(candidate, coarse));
    if gaps.is_empty() {
        return coarse;
    }
    gaps.sort_unstable();
    Fix {
        shift_ms: coarse.shift_ms + gaps[gaps.len() / 2],
        ..coarse
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::synth::{self, Label};

    fn ev(start_ms: u64, end_ms: u64) -> Event {
        Event { start_ms, end_ms }
    }

    fn span(start: i64, end: i64) -> Span {
        Span { start, end }
    }

    fn brute_overlap(ruler: &[Span], cand: &[Span], d: i64) -> i64 {
        let mut total = 0;
        for a in ruler {
            for b in cand {
                total += (a.end.min(b.end + d) - a.start.max(b.start + d)).max(0);
            }
        }
        total
    }

    #[test]
    fn overlap_curve_matches_brute_force() {
        let ruler = [span(0, 5), span(3, 4), span(10, 30), span(31, 33)];
        let cand = [span(-4, 2), span(6, 7), span(20, 21), span(25, 40)];
        for (lo, hi) in [(-50, 50), (-3, 7), (12, 12), (40, 90)] {
            let curve = overlap_curve(&ruler, &cand, lo, hi);
            assert_eq!(curve.len() as i64, hi - lo + 1);
            for (i, &value) in curve.iter().enumerate() {
                let d = lo + i as i64;
                assert_eq!(value, brute_overlap(&ruler, &cand, d), "shift {d}");
            }
        }
    }

    #[test]
    fn peak_takes_plateau_middle_and_needs_overlap() {
        assert_eq!(peak(&[0, 2, 5, 5, 5, 1]), Some(3));
        assert_eq!(peak(&[0, 5, 5, 1, 5]), Some(1));
        assert_eq!(peak(&[0, 0, 0]), None);
        assert_eq!(peak(&[]), None);
    }

    #[test]
    fn fix_scales_then_shifts() {
        let pal = Fix {
            num: 25_025,
            den: 24_000,
            shift_ms: -500,
        };
        assert_eq!(pal.apply(24_000), 25_025 - 500);
        assert_eq!(pal.apply(0), -500);
        let huge = Fix {
            num: 2,
            den: 1,
            shift_ms: 1,
        };
        assert_eq!(huge.apply(u64::MAX), i64::MAX);
    }

    #[test]
    fn hits_take_nearest_start_within_tolerance() {
        let ruler = [ev(1_000, 2_000), ev(5_000, 6_000), ev(9_000, 9_500)];
        let starts = [1_100, 4_900, 5_250, 20_000];
        assert_eq!(hits(&ruler, &starts), vec![-100, 100]);
    }

    #[test]
    fn empty_inputs_are_rejected() {
        let one = [ev(0, 1_000)];
        assert_eq!(align(&[], &one).reject, Some(Reject::NoOverlap));
        assert_eq!(align(&one, &[]).reject, Some(Reject::NoOverlap));
        assert_eq!(align(&one, &[]).accepted(), None);
    }

    #[test]
    fn exact_copy_is_accepted_with_identity() {
        let mut rng = synth::Rng::new(7);
        let speech = synth::speech(&mut rng, 100);
        let ruler = synth::lines(&mut rng, &speech, &synth::Style::RULER);
        let verdict = align(&ruler, &ruler);
        assert_eq!(
            verdict.accepted(),
            Some(Fix {
                num: 1,
                den: 1,
                shift_ms: 0
            })
        );
        assert_eq!(verdict.hit_rate, 1.0);
    }

    #[test]
    fn trials_cover_every_track_and_readable_candidate() {
        let mut rng = synth::Rng::new(3);
        let speech = synth::speech(&mut rng, 95);
        let full = synth::lines(&mut rng, &speech, &synth::Style::RULER);
        let sdh = synth::lines(&mut rng, &speech, &synth::Style::OTHER);
        let good = synth::lines(&mut rng, &speech, &synth::Style::OTHER);
        let elsewhere = synth::speech(&mut rng, 95);
        let wrong = synth::lines(&mut rng, &elsewhere, &synth::Style::OTHER);

        let rulers = [(5, full), (4, sdh)];
        let found = trials(&rulers, &[Some(good), None, Some(wrong)]);
        let shape: Vec<(u64, usize, bool)> = found
            .iter()
            .map(|t| (t.track, t.candidate, t.accepted))
            .collect();
        assert_eq!(
            shape,
            vec![(5, 0, true), (5, 2, false), (4, 0, true), (4, 2, false)]
        );
    }

    #[test]
    fn labeled_set_has_no_false_accepts() {
        let mut accepted = 0;
        let mut positives = 0;
        for seed in 0..8 {
            for case in synth::labeled_set(seed) {
                let verdict = align(&case.ruler, &case.candidate);
                let context = format!("seed {seed} {}: {verdict:?}", case.name);
                match case.label {
                    Label::Reject => assert_eq!(verdict.accepted(), None, "{context}"),
                    Label::Accept(truth) => {
                        positives += 1;
                        let Some(fix) = verdict.accepted() else {
                            continue;
                        };
                        accepted += 1;
                        assert_eq!((fix.num, fix.den), (truth.num, truth.den), "{context}");
                        assert!(
                            (fix.shift_ms - truth.shift_ms).abs() <= 100,
                            "{context}, truth {truth:?}"
                        );
                    }
                }
            }
        }
        assert_eq!(accepted, positives, "every positive should be accepted");
    }
}
