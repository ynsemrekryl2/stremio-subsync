//! Synthetic film timings for alignment tests: a speech track, subtitle
//! lines cut from it in a given style, and a labeled set of candidates that
//! must or must not be served. Deterministic per seed; no text anywhere.

use crate::align::Fix;
use crate::ruler::Event;

/// splitmix64; enough for test data and needs no dependency.
pub struct Rng(u64);

impl Rng {
    pub fn new(seed: u64) -> Self {
        Self(seed ^ 0xD1B5_4A32_D192_ED03)
    }

    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// Uniform in `lo..=hi`.
    pub fn range(&mut self, lo: u64, hi: u64) -> u64 {
        lo + self.next() % (hi - lo + 1)
    }

    /// Uniform in `-reach..=reach`.
    pub fn signed(&mut self, reach: u64) -> i64 {
        self.range(0, 2 * reach) as i64 - reach as i64
    }

    pub fn chance(&mut self, p: f64) -> bool {
        ((self.next() >> 11) as f64 / (1u64 << 53) as f64) < p
    }
}

/// When people speak: lines of 0.6–5 s, short pauses, rare long silences.
pub fn speech(rng: &mut Rng, minutes: u64) -> Vec<Event> {
    let end = minutes * 60_000;
    let mut t = rng.range(3_000, 90_000);
    let mut out = Vec::new();
    while t < end {
        if rng.chance(0.02) {
            t += rng.range(30_000, 150_000);
            continue;
        }
        let length = rng.range(600, 5_000);
        out.push(Event {
            start_ms: t,
            end_ms: t + length,
        });
        t += length + rng.range(100, 7_000);
    }
    out
}

/// How one subtitle source cuts speech into lines.
pub struct Style {
    /// Each edge moves up to this far.
    pub jitter_ms: u64,
    /// Share of speech left out.
    pub drop: f64,
    /// Chance of a sound cue or sign line after a line.
    pub extra: f64,
    /// Chance of joining a line to the previous one when they are close.
    pub merge: f64,
}

impl Style {
    /// The file's own English track.
    pub const RULER: Style = Style {
        jitter_ms: 40,
        drop: 0.0,
        extra: 0.0,
        merge: 0.05,
    };
    /// A subtitle from somewhere else, timed for another release.
    pub const OTHER: Style = Style {
        jitter_ms: 150,
        drop: 0.08,
        extra: 0.05,
        merge: 0.12,
    };
}

fn jitter(rng: &mut Rng, t: u64, reach: u64) -> u64 {
    (t as i64 + rng.signed(reach)).max(0) as u64
}

/// Subtitle lines for `speech` in `style`, sorted.
pub fn lines(rng: &mut Rng, speech: &[Event], style: &Style) -> Vec<Event> {
    let mut out: Vec<Event> = Vec::new();
    for said in speech {
        if rng.chance(style.drop) {
            continue;
        }
        let start = jitter(rng, said.start_ms, style.jitter_ms);
        let end = jitter(rng, said.end_ms, style.jitter_ms).max(start + 300);
        if let Some(prev) = out.last_mut()
            && start < prev.end_ms + 1_500
            && rng.chance(style.merge)
        {
            prev.end_ms = prev.end_ms.max(end);
            continue;
        }
        out.push(Event {
            start_ms: start,
            end_ms: end,
        });
        if rng.chance(style.extra) {
            let cue = end + rng.range(200, 3_000);
            out.push(Event {
                start_ms: cue,
                end_ms: cue + rng.range(800, 2_500),
            });
        }
    }
    out.sort_unstable();
    out
}

/// Times as another release has them: the lines `fix` maps back onto these.
/// Lines that would fall before that release starts are dropped.
pub fn distort(lines: &[Event], fix: Fix) -> Vec<Event> {
    let back = |t: u64| {
        let unshifted = t as i64 - fix.shift_ms;
        (unshifted >= 0).then(|| unshifted as u64 * fix.den / fix.num)
    };
    lines
        .iter()
        .filter_map(|e| {
            Some(Event {
                start_ms: back(e.start_ms)?,
                end_ms: back(e.end_ms)?,
            })
        })
        .collect()
}

pub enum Label {
    /// Must be served with this fix.
    Accept(Fix),
    /// No single fix makes it right; must not be served.
    Reject,
}

pub struct Case {
    pub name: &'static str,
    pub ruler: Vec<Event>,
    pub candidate: Vec<Event>,
    pub label: Label,
}

fn fix(num: u64, den: u64, shift_ms: i64) -> Fix {
    Fix { num, den, shift_ms }
}

/// One film, its ruler, and candidates labeled by how they were made.
pub fn labeled_set(seed: u64) -> Vec<Case> {
    let mut rng = Rng::new(seed);
    let minutes = rng.range(85, 130);
    let speech = speech(&mut rng, minutes);
    let ruler = lines(&mut rng, &speech, &Style::RULER);
    let end = minutes * 60_000;
    let mut cases = Vec::new();

    let positives = [
        ("same", fix(1, 1, 0)),
        ("shifted", fix(1, 1, rng.signed(90_000))),
        ("pal-fast", fix(25_025, 24_000, rng.signed(30_000))),
        ("pal-slow", fix(24_000, 25_025, rng.signed(30_000))),
        ("ntsc24", fix(1_001, 1_000, rng.signed(30_000))),
    ];
    for (name, truth) in positives {
        let other = lines(&mut rng, &speech, &Style::OTHER);
        cases.push(Case {
            name,
            ruler: ruler.clone(),
            candidate: distort(&other, truth),
            label: Label::Accept(truth),
        });
    }

    let mut negative = |name, ruler: Vec<Event>, candidate| {
        cases.push(Case {
            name,
            ruler,
            candidate,
            label: Label::Reject,
        })
    };

    let elsewhere = self::speech(&mut rng, minutes);
    let other = lines(&mut rng, &elsewhere, &Style::OTHER);
    let shift = rng.signed(30_000);
    negative(
        "other-film",
        ruler.clone(),
        distort(&other, fix(1, 1, shift)),
    );

    // Another cut: a scene of 20–90 s exists only in the candidate's release.
    let cut = rng.range(end / 4, end * 3 / 4);
    let scene = rng.range(20_000, 90_000);
    let recut: Vec<Event> = speech
        .iter()
        .map(|e| {
            if e.start_ms < cut {
                return *e;
            }
            Event {
                start_ms: e.start_ms + scene,
                end_ms: e.end_ms + scene,
            }
        })
        .collect();
    let other = lines(&mut rng, &recut, &Style::OTHER);
    let shift = rng.signed(30_000);
    negative(
        "other-cut",
        ruler.clone(),
        distort(&other, fix(1, 1, shift)),
    );

    let other = lines(&mut rng, &speech, &Style::OTHER);
    let partial = other
        .into_iter()
        .filter(|e| e.start_ms < end * 45 / 100)
        .collect();
    negative("partial", ruler.clone(), partial);

    // A steady drift no listed frame-rate fix explains.
    let other = lines(&mut rng, &speech, &Style::OTHER);
    let shift = rng.signed(30_000);
    negative(
        "drift",
        ruler.clone(),
        distort(&other, fix(1_003, 1_000, shift)),
    );

    let other = lines(&mut rng, &speech, &Style::OTHER);
    let scrambled = other
        .iter()
        .map(|e| {
            let moved = rng.signed(2_500);
            Event {
                start_ms: (e.start_ms as i64 + moved).max(0) as u64,
                end_ms: (e.end_ms as i64 + moved).max(0) as u64,
            }
        })
        .collect();
    negative("scrambled", ruler.clone(), scrambled);

    // A forced track: a few dozen lines spread over the film.
    let forced = ruler
        .iter()
        .step_by(ruler.len() / 30 + 1)
        .copied()
        .collect();
    let other = lines(&mut rng, &speech, &Style::OTHER);
    negative("forced-ruler", forced, other);

    cases
}
