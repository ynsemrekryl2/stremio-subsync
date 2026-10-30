//! Translation core (SS-037), ported from Nen's `nen-translate`: the lines
//! of an accepted English subtitle are split into overlapping blocks, the
//! whole document is analyzed once (summary, characters, glossary), every
//! block is translated with that analysis and its neighbours as context,
//! and each answer is checked locally and repaired a bounded number of
//! times. Only `id + text` goes to the translator; times stay here and are
//! copied from the source lines.
//!
//! Not taken from Nen: checkpoints and cache identity (the M5 cache keeps
//! finished translations), the WebVTT artifact (the SRT writer does) and
//! the cancellation gate (dropping the future cancels a run).
//!
//! Subtitle text never reaches `Debug` or `Display`: every type that holds
//! it prints counts and lengths only.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::fmt;
use std::future::Future;
use std::ops::Range;
use std::pin::Pin;

use crate::srt::Cue;

/// Lines per block, and lines a block shares with each neighbour as context
/// only (Nen's ADR-0015 defaults).
pub const BLOCK_SIZE: usize = 40;
pub const OVERLAP: usize = 6;

/// At most this many repeated, capitalized terms go to the analysis.
pub const TERM_LIMIT: usize = 32;

/// Repairs after the first answer: up to two asking only for the bad
/// lines, then one clean retry of the whole block.
pub const MAX_TARGETED_REPAIRS: usize = 2;
pub const MAX_FULL_RETRIES: usize = 1;

/// One subtitle line as the translator sees it: its number in the document
/// (1-based) and its text, lines joined by `\n`.
#[derive(Clone, PartialEq, Eq)]
pub struct Line {
    pub id: u32,
    pub text: String,
}

impl fmt::Debug for Line {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Line")
            .field("id", &self.id)
            .field("text_len", &self.text.chars().count())
            .finish()
    }
}

/// Languages as ISO 639-2 codes (`eng`, `tur`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Langs {
    pub source: String,
    pub target: String,
}

/// First stage: the whole document, read once.
#[derive(Clone, PartialEq, Eq)]
pub struct AnalysisRequest {
    pub langs: Langs,
    pub transcript: Vec<Line>,
    /// Repeated, capitalized terms (see [`context_terms`]).
    pub terms: Vec<String>,
}

impl fmt::Debug for AnalysisRequest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AnalysisRequest")
            .field("langs", &self.langs)
            .field("lines", &self.transcript.len())
            .field("terms", &self.terms.len())
            .finish()
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct Character {
    pub name: String,
    pub description: String,
}

#[derive(Clone, PartialEq, Eq)]
pub struct GlossaryEntry {
    pub source: String,
    pub target: String,
    pub note: String,
}

/// What the first stage returns; every block request carries it unchanged.
#[derive(Clone, PartialEq, Eq)]
pub struct Analysis {
    pub summary: String,
    pub characters: Vec<Character>,
    pub glossary: Vec<GlossaryEntry>,
}

impl Analysis {
    /// The translator's answer is untrusted: no empty summary or field.
    pub fn is_valid(&self) -> bool {
        let filled = |s: &str| !s.trim().is_empty();
        filled(&self.summary)
            && self
                .characters
                .iter()
                .all(|c| filled(&c.name) && filled(&c.description))
            && self
                .glossary
                .iter()
                .all(|g| filled(&g.source) && filled(&g.target) && filled(&g.note))
    }
}

impl fmt::Debug for Analysis {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Analysis")
            .field("summary_len", &self.summary.chars().count())
            .field("characters", &self.characters.len())
            .field("glossary", &self.glossary.len())
            .finish()
    }
}

/// Which attempt a block request is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Initial,
    /// Only `output_ids` are asked again; the rest already passed.
    TargetedRepair,
    FullRetry,
}

/// Second stage: one block.
#[derive(Clone, PartialEq, Eq)]
pub struct BlockRequest {
    pub langs: Langs,
    /// The block's lines plus its neighbours' overlap.
    pub context: Vec<Line>,
    /// The lines to translate, a subset of `context`.
    pub output_ids: Vec<u32>,
    pub analysis: Analysis,
    /// 1-based.
    pub block: usize,
    pub blocks: usize,
    pub mode: Mode,
}

impl fmt::Debug for BlockRequest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("BlockRequest")
            .field("langs", &self.langs)
            .field("context", &self.context.len())
            .field("output", &self.output_ids.len())
            .field("analysis", &self.analysis)
            .field("block", &self.block)
            .field("blocks", &self.blocks)
            .field("mode", &self.mode)
            .finish()
    }
}

/// Why a translator call failed. Carries no text.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TranslateError {
    /// Worth trying again later (timeout, rate limit, 5xx).
    Transient,
    Permanent,
    /// The answer could not be read in the asked shape (broken JSON, a
    /// refusal, a filtered or cut answer). A model slip, not a dead
    /// service: it is repaired like a wrong answer.
    BadAnswer,
}

pub type AnalyzeFuture<'a> =
    Pin<Box<dyn Future<Output = Result<Analysis, TranslateError>> + Send + 'a>>;
pub type TranslateFuture<'a> =
    Pin<Box<dyn Future<Output = Result<Vec<Line>, TranslateError>> + Send + 'a>>;

/// A translation model; faked in tests, OpenRouter in SS-038.
pub trait Translator: Send + Sync {
    fn analyze<'a>(&'a self, request: &'a AnalysisRequest) -> AnalyzeFuture<'a>;
    /// Translated lines for `request.output_ids`, by id.
    fn translate<'a>(&'a self, request: &'a BlockRequest) -> TranslateFuture<'a>;
}

/// Why a document was not translated. Carries no text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Failure {
    Empty,
    Translator(TranslateError),
    /// The analysis came back with an empty field.
    BadAnalysis,
    /// A block stayed invalid after every repair.
    Exhausted {
        block: usize,
        violations: Vec<Violation>,
    },
}

/// Translates `cues` from `langs.source` to `langs.target`. Each finished
/// block extends a prefix of translated cues, handed to `on_block` so a
/// caller can serve what is ready; the result is every cue, with the
/// source's times.
pub async fn translate(
    translator: &dyn Translator,
    cues: &[Cue],
    langs: &Langs,
    mut on_block: impl FnMut(&[Cue]),
) -> Result<Vec<Cue>, Failure> {
    let lines = lines_of(cues);
    let layout = layout(lines.len());
    if layout.is_empty() {
        return Err(Failure::Empty);
    }
    let analysis = translator
        .analyze(&AnalysisRequest {
            langs: langs.clone(),
            transcript: lines.clone(),
            terms: context_terms(&lines),
        })
        .await
        .map_err(|err| match err {
            TranslateError::BadAnswer => Failure::BadAnalysis,
            err => Failure::Translator(err),
        })?;
    if !analysis.is_valid() {
        return Err(Failure::BadAnalysis);
    }
    let mut done: Vec<Cue> = Vec::with_capacity(cues.len());
    for block in &layout {
        let request = BlockRequest {
            langs: langs.clone(),
            context: lines[block.context.clone()].to_vec(),
            output_ids: lines[block.output.clone()].iter().map(|l| l.id).collect(),
            analysis: analysis.clone(),
            block: block.index + 1,
            blocks: layout.len(),
            mode: Mode::Initial,
        };
        let texts = translate_block(translator, &request).await?;
        done.extend(
            cues[block.output.clone()]
                .iter()
                .zip(texts)
                .map(|(cue, text)| Cue {
                    start_ms: cue.start_ms,
                    end_ms: cue.end_ms,
                    text,
                }),
        );
        on_block(&done);
    }
    Ok(done)
}

fn lines_of(cues: &[Cue]) -> Vec<Line> {
    cues.iter()
        .zip(1u32..)
        .map(|(cue, id)| Line {
            id,
            text: cue.text.clone(),
        })
        .collect()
}

/// One block: the positions it sees and the positions it translates. Every
/// position is in exactly one block's `output`, so nothing is translated
/// twice; the overlap is context only.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Block {
    pub index: usize,
    pub context: Range<usize>,
    pub output: Range<usize>,
}

/// Splits `len` lines into blocks of [`BLOCK_SIZE`] stepping by
/// `BLOCK_SIZE - OVERLAP`; the last window is pinned to end at `len` so it
/// is full size, and each shared overlap goes to its two owners at the
/// midpoint. Deterministic; empty for no lines.
pub fn layout(len: usize) -> Vec<Block> {
    if len == 0 {
        return Vec::new();
    }
    if len <= BLOCK_SIZE {
        return vec![Block {
            index: 0,
            context: 0..len,
            output: 0..len,
        }];
    }
    let mut starts = Vec::new();
    let mut start = 0;
    loop {
        starts.push(start);
        if start + BLOCK_SIZE >= len {
            break;
        }
        start += BLOCK_SIZE - OVERLAP;
    }
    if let Some(last) = starts.last_mut() {
        *last = len - BLOCK_SIZE;
    }
    // OVERLAP < BLOCK_SIZE / 2 keeps these strictly increasing even after
    // the last window is pinned back.
    let mut bounds = vec![0];
    for pair in starts.windows(2) {
        let shared = (pair[0] + BLOCK_SIZE).saturating_sub(pair[1]);
        bounds.push(pair[1] + shared / 2);
    }
    bounds.push(len);
    starts
        .iter()
        .zip(bounds.windows(2))
        .enumerate()
        .map(|(index, (&start, bound))| Block {
            index,
            context: start..start + BLOCK_SIZE,
            output: bound[0]..bound[1],
        })
        .collect()
}

/// Terms that read as names: capitalized, at least 2 characters, seen at
/// least 3 times and at least once not as a line's first word (any word can
/// open a sentence). Case-sensitive, most frequent first, then
/// alphabetical, at most [`TERM_LIMIT`].
pub fn context_terms(lines: &[Line]) -> Vec<String> {
    let mut counts: HashMap<&str, (usize, bool)> = HashMap::new();
    for line in lines {
        for row in line.text.lines() {
            let words = row
                .split(|c: char| !c.is_alphanumeric())
                .filter(|w| !w.is_empty());
            for (position, word) in words.enumerate() {
                let mut chars = word.chars();
                let capitalized = chars.next().is_some_and(char::is_uppercase);
                if !capitalized || chars.next().is_none() {
                    continue;
                }
                let entry = counts.entry(word).or_insert((0, false));
                entry.0 += 1;
                entry.1 |= position > 0;
            }
        }
    }
    let mut terms: Vec<(&str, usize)> = counts
        .into_iter()
        .filter(|&(_, (seen, mid))| seen >= 3 && mid)
        .map(|(word, (seen, _))| (word, seen))
        .collect();
    terms.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(b.0)));
    terms.truncate(TERM_LIMIT);
    terms.into_iter().map(|(word, _)| word.to_owned()).collect()
}

/// Why an answer cannot be used as it is. Carries ids, never text.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Violation {
    CountMismatch {
        expected: usize,
        received: usize,
    },
    Missing {
        id: u32,
    },
    Duplicate {
        id: u32,
    },
    Unknown {
        id: u32,
    },
    Empty {
        id: u32,
    },
    /// The text would break the SRT around it (a `-->` time arrow).
    Malformed {
        id: u32,
    },
    /// The whole answer could not be read ([`TranslateError::BadAnswer`]).
    Unreadable,
}

/// A failed check: what was wrong, which ids to ask again, and the lines
/// that passed (kept for a targeted repair, never served on their own).
struct Invalid {
    violations: Vec<Violation>,
    repair_ids: Vec<u32>,
    accepted: BTreeMap<u32, String>,
}

/// Checks an answer against the expected ids. On success the texts come
/// back in `expected` order, each trimmed line by line with blank lines
/// dropped (a blank line would end the SRT cue early).
fn validate(expected: &[u32], answer: &[Line]) -> Result<Vec<String>, Invalid> {
    let wanted: BTreeSet<u32> = expected.iter().copied().collect();
    let mut by_id: BTreeMap<u32, Vec<&str>> = BTreeMap::new();
    for line in answer {
        by_id.entry(line.id).or_default().push(&line.text);
    }
    let mut violations = Vec::new();
    if expected.len() != answer.len() {
        violations.push(Violation::CountMismatch {
            expected: expected.len(),
            received: answer.len(),
        });
    }
    violations.extend(
        by_id
            .keys()
            .filter(|id| !wanted.contains(id))
            .map(|&id| Violation::Unknown { id }),
    );
    let mut repair_ids = Vec::new();
    let mut accepted = BTreeMap::new();
    for &id in expected {
        let bad = match by_id.get(&id).map(Vec::as_slice) {
            None => Violation::Missing { id },
            Some([text]) => {
                let clean = clean_text(text);
                if clean.is_empty() {
                    Violation::Empty { id }
                } else if clean.contains("-->") {
                    Violation::Malformed { id }
                } else {
                    accepted.insert(id, clean);
                    continue;
                }
            }
            Some(_) => Violation::Duplicate { id },
        };
        violations.push(bad);
        repair_ids.push(id);
    }
    if violations.is_empty() {
        Ok(expected
            .iter()
            .filter_map(|id| accepted.remove(id))
            .collect())
    } else {
        Err(Invalid {
            violations,
            repair_ids,
            accepted,
        })
    }
}

fn clean_text(text: &str) -> String {
    text.lines()
        .map(str::trim)
        .filter(|row| !row.is_empty())
        .collect::<Vec<_>>()
        .join("\n")
}

/// One block with the fixed repair budget: the first answer, up to
/// [`MAX_TARGETED_REPAIRS`] asking only for the bad lines (merged with
/// those that passed), then one clean full retry. An unreadable answer uses
/// the budget like a wrong one; any other translator error ends it at once.
/// Only a fully valid block comes back.
async fn translate_block(
    translator: &dyn Translator,
    request: &BlockRequest,
) -> Result<Vec<String>, Failure> {
    let expected = request.output_ids.clone();
    let mut invalid = match check(&expected, ask(translator, request).await?) {
        Ok(texts) => return Ok(texts),
        Err(invalid) => invalid,
    };
    for _ in 0..MAX_TARGETED_REPAIRS {
        if invalid.repair_ids.is_empty() {
            break;
        }
        let repair = BlockRequest {
            output_ids: invalid.repair_ids.clone(),
            mode: Mode::TargetedRepair,
            ..request.clone()
        };
        let answer = ask(translator, &repair).await?;
        let mut merged: Vec<Line> = invalid
            .accepted
            .iter()
            .map(|(&id, text)| Line {
                id,
                text: text.clone(),
            })
            .collect();
        merged.extend(answer.unwrap_or_default());
        invalid = match validate(&expected, &merged) {
            Ok(texts) => return Ok(texts),
            Err(invalid) => invalid,
        };
    }
    let retry = BlockRequest {
        mode: Mode::FullRetry,
        ..request.clone()
    };
    for _ in 0..MAX_FULL_RETRIES {
        invalid = match check(&expected, ask(translator, &retry).await?) {
            Ok(texts) => return Ok(texts),
            Err(invalid) => invalid,
        };
    }
    Err(Failure::Exhausted {
        block: request.block,
        violations: invalid.violations,
    })
}

/// The translator's answer, or `None` when it could not be read.
async fn ask(
    translator: &dyn Translator,
    request: &BlockRequest,
) -> Result<Option<Vec<Line>>, Failure> {
    match translator.translate(request).await {
        Ok(answer) => Ok(Some(answer)),
        Err(TranslateError::BadAnswer) => Ok(None),
        Err(err) => Err(Failure::Translator(err)),
    }
}

/// [`validate`], with an unreadable answer asking for every line again.
fn check(expected: &[u32], answer: Option<Vec<Line>>) -> Result<Vec<String>, Invalid> {
    match answer {
        Some(answer) => validate(expected, &answer),
        None => Err(Invalid {
            violations: vec![Violation::Unreadable],
            repair_ids: expected.to_vec(),
            accepted: BTreeMap::new(),
        }),
    }
}

/// A pair of consecutive cues counts as shifted when the first cue's share
/// of the pair's letters moves by more than this from source to
/// translation (SS-044).
pub const SHIFT_LIMIT: f64 = 0.35;

/// How a translation spreads sentences that run across cues (SS-044):
/// counts only, no text.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Balance {
    pub lines: usize,
    /// Consecutive source pairs whose first cue does not end a sentence.
    pub split_pairs: usize,
    /// Of those, pairs whose first translated cue took more than
    /// [`SHIFT_LIMIT`] over its source share (`forward`) or less (`back`).
    pub forward: usize,
    pub back: usize,
}

/// Compares a translation with its source cue by cue. `None` when they are
/// not the same cues (count or times differ).
pub fn balance(source: &[Cue], target: &[Cue]) -> Option<Balance> {
    let same = source.len() == target.len()
        && source
            .iter()
            .zip(target)
            .all(|(s, t)| s.start_ms == t.start_ms && s.end_ms == t.end_ms);
    if !same {
        return None;
    }
    let mut out = Balance {
        lines: source.len(),
        ..Balance::default()
    };
    for i in 1..source.len() {
        if ends_sentence(&source[i - 1].text) {
            continue;
        }
        let (a, b) = (letters(&source[i - 1].text), letters(&source[i].text));
        let (c, d) = (letters(&target[i - 1].text), letters(&target[i].text));
        if a + b == 0 || c + d == 0 {
            continue;
        }
        out.split_pairs += 1;
        let moved = c as f64 / (c + d) as f64 - a as f64 / (a + b) as f64;
        if moved > SHIFT_LIMIT {
            out.forward += 1;
        } else if moved < -SHIFT_LIMIT {
            out.back += 1;
        }
    }
    Some(out)
}

/// Text without `<...>` and `{...}` markup.
fn visible(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut closing = None;
    for ch in text.chars() {
        match (closing, ch) {
            (None, '<') => closing = Some('>'),
            (None, '{') => closing = Some('}'),
            (None, _) => out.push(ch),
            (Some(end), _) if ch == end => closing = None,
            _ => {}
        }
    }
    out
}

fn letters(text: &str) -> usize {
    visible(text)
        .chars()
        .filter(|c| c.is_alphanumeric())
        .count()
}

fn ends_sentence(text: &str) -> bool {
    let shown = visible(text);
    let trimmed = shown.trim_end_matches(|c: char| {
        c.is_whitespace()
            || matches!(
                c,
                '"' | '\'' | '\u{201d}' | '\u{2019}' | ')' | ']' | '\u{bb}'
            )
    });
    trimmed.ends_with(['.', '!', '?', '\u{2026}', '\u{266a}'])
}

/// `stremio-subsync balance-check <english.srt> <translated.srt>`: prints
/// [`Balance`] for a translation already on disk; no network.
pub fn balance_check(source: &std::path::Path, target: &std::path::Path) -> Result<(), String> {
    let read = |path: &std::path::Path, lang: &str| {
        std::fs::read(path)
            .map(|bytes| crate::srt::parse(&crate::srt::decode(&bytes, lang)))
            .map_err(|err| format!("read: {:?}", err.kind()))
    };
    let lang = std::env::var("SUBSYNC_LANG").unwrap_or_else(|_| "tur".to_owned());
    let report = balance(&read(source, "eng")?, &read(target, &lang)?)
        .ok_or("not the same cues (count or times differ)")?;
    println!("{}", balance_line(&report));
    Ok(())
}

/// One log line for a [`Balance`].
pub fn balance_line(b: &Balance) -> String {
    let shifted = b.forward + b.back;
    let pct = if b.split_pairs == 0 {
        0.0
    } else {
        100.0 * shifted as f64 / b.split_pairs as f64
    };
    format!(
        "balance lines={} split_pairs={} shifted={shifted} ({pct:.1}%) forward={} back={}",
        b.lines, b.split_pairs, b.forward, b.back
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::VecDeque;
    use std::sync::Mutex;

    fn cues(n: u64) -> Vec<Cue> {
        (0..n)
            .map(|i| Cue {
                start_ms: 1_000 + i * 2_000,
                end_ms: 2_500 + i * 2_000,
                text: format!("Source line {}", i + 1),
            })
            .collect()
    }

    fn langs() -> Langs {
        Langs {
            source: "eng".to_owned(),
            target: "tur".to_owned(),
        }
    }

    fn analysis() -> Analysis {
        Analysis {
            summary: "A summary".to_owned(),
            characters: vec![Character {
                name: "Speaker".to_owned(),
                description: "Informal".to_owned(),
            }],
            glossary: Vec::new(),
        }
    }

    fn line(id: u32, text: &str) -> Line {
        Line {
            id,
            text: text.to_owned(),
        }
    }

    /// Answers each block from a script; with the script empty, translates
    /// every asked line as `tr <source>`. Records every request.
    #[derive(Default)]
    struct Fake {
        analysis: Mutex<Option<Analysis>>,
        script: Mutex<VecDeque<Result<Vec<Line>, TranslateError>>>,
        analyzed: Mutex<Vec<AnalysisRequest>>,
        asked: Mutex<Vec<BlockRequest>>,
    }

    impl Fake {
        fn scripted(answers: Vec<Result<Vec<Line>, TranslateError>>) -> Self {
            Self {
                script: Mutex::new(answers.into()),
                ..Self::default()
            }
        }

        fn asked(&self) -> Vec<BlockRequest> {
            self.asked.lock().unwrap().clone()
        }
    }

    impl Translator for Fake {
        fn analyze<'a>(&'a self, request: &'a AnalysisRequest) -> AnalyzeFuture<'a> {
            self.analyzed.lock().unwrap().push(request.clone());
            let answer = self.analysis.lock().unwrap().clone();
            Box::pin(async move { Ok(answer.unwrap_or_else(analysis)) })
        }

        fn translate<'a>(&'a self, request: &'a BlockRequest) -> TranslateFuture<'a> {
            self.asked.lock().unwrap().push(request.clone());
            let scripted = self.script.lock().unwrap().pop_front();
            let answer = scripted.unwrap_or_else(|| {
                Ok(request
                    .context
                    .iter()
                    .filter(|l| request.output_ids.contains(&l.id))
                    .map(|l| line(l.id, &format!("tr {}", l.text)))
                    .collect())
            });
            Box::pin(async move { answer })
        }
    }

    fn block_request(ids: &[u32]) -> BlockRequest {
        BlockRequest {
            langs: langs(),
            context: ids.iter().map(|&id| line(id, &format!("s{id}"))).collect(),
            output_ids: ids.to_vec(),
            analysis: analysis(),
            block: 1,
            blocks: 1,
            mode: Mode::Initial,
        }
    }

    fn cue(i: u64, text: &str) -> Cue {
        Cue {
            start_ms: i * 2_000,
            end_ms: i * 2_000 + 1_500,
            text: text.to_owned(),
        }
    }

    fn pair(first: &str, second: &str) -> Vec<Cue> {
        vec![cue(0, first), cue(1, second)]
    }

    #[test]
    fn balance_counts_split_sentences_moved_between_cues() {
        let source = pair("I will not", "do this.");
        let even = balance(&source, &pair("Bunu", "yapmayacağım.")).unwrap();
        assert_eq!((even.split_pairs, even.forward, even.back), (1, 0, 0));
        let forward = balance(&source, &pair("Bunu yapmayacağım.", "-")).unwrap();
        assert_eq!((forward.forward, forward.back), (1, 0));
        let back = balance(&source, &pair("Ben", "bunu asla yapmayacağım.")).unwrap();
        assert_eq!((back.forward, back.back), (0, 1));
    }

    #[test]
    fn balance_skips_pairs_whose_first_cue_ends_a_sentence() {
        for first in [
            "Stop.",
            "Stop!\"",
            "<i>Really?</i>",
            "Wait\u{2026}",
            "\u{266a} la la \u{266a}",
        ] {
            let report = balance(&pair(first, "go"), &pair("Dur bakalım hemen", "-")).unwrap();
            assert_eq!(report.split_pairs, 0, "{first:?}");
        }
        let tagged = balance(
            &pair("<i>I will</i>", "go home"),
            &pair("{\\an8}Eve giderim", "-"),
        )
        .unwrap();
        assert_eq!((tagged.split_pairs, tagged.forward), (1, 1));
    }

    #[test]
    fn balance_needs_the_same_cues() {
        let source = pair("I will not", "do this.");
        assert_eq!(balance(&source, &source[..1]), None);
        let mut moved = source.clone();
        moved[1].start_ms += 1;
        assert_eq!(balance(&source, &moved), None);
        assert_eq!(balance(&[], &[]), Some(Balance::default()));
    }

    #[test]
    fn balance_line_prints_counts_only() {
        let line = balance_line(&Balance {
            lines: 10,
            split_pairs: 4,
            forward: 1,
            back: 0,
        });
        assert_eq!(
            line,
            "balance lines=10 split_pairs=4 shifted=1 (25.0%) forward=1 back=0"
        );
    }

    #[test]
    fn layout_partitions_every_line_once_in_order() {
        for len in [1, 25, 40, 41, 50, 79, 80, 81, 95, 121, 240, 1768] {
            let blocks = layout(len);
            let mut next = 0;
            for block in &blocks {
                assert_eq!(block.output.start, next, "len {len}");
                assert!(block.output.end > block.output.start, "len {len}");
                assert!(block.context.start <= block.output.start, "len {len}");
                assert!(block.output.end <= block.context.end, "len {len}");
                assert!(block.context.end <= len, "len {len}");
                next = block.output.end;
            }
            assert_eq!(next, len, "len {len}");
        }
        assert!(layout(0).is_empty());
    }

    #[test]
    fn layout_is_one_block_up_to_block_size_and_pins_the_last_window() {
        assert_eq!(
            layout(25),
            vec![Block {
                index: 0,
                context: 0..25,
                output: 0..25
            }]
        );
        let blocks = layout(95);
        assert_eq!(blocks.last().unwrap().context, 55..95);
        for pair in blocks.windows(2) {
            assert!(
                pair[1].context.start < pair[0].context.end,
                "windows overlap"
            );
        }
        assert_eq!(layout(95), blocks, "deterministic");
    }

    #[test]
    fn terms_are_repeated_names_not_sentence_openers() {
        let lines = [
            line(1, "Gon looked at Killua."),
            line(2, "Killua smiled back."),
            line(3, "Then Killua left.\nGon ran."),
            line(4, "Gon won."),
        ];
        assert_eq!(context_terms(&lines), vec!["Killua"]);
        let few = [line(1, "Hi Killua."), line(2, "Bye Killua.")];
        assert!(context_terms(&few).is_empty());
    }

    #[test]
    fn terms_are_sorted_by_count_then_name_and_capped() {
        let lines = [
            line(1, "Hi Zebra Apple."),
            line(2, "Hi Zebra Apple."),
            line(3, "Hi Zebra Apple."),
            line(4, "Hi Apple."),
        ];
        assert_eq!(context_terms(&lines), vec!["Apple", "Zebra"]);
        let many: Vec<Line> = (0..40u32)
            .flat_map(|n| (0..3).map(move |_| line(n, &format!("hello Term{n:02}"))))
            .collect();
        assert_eq!(context_terms(&many).len(), TERM_LIMIT);
    }

    #[test]
    fn valid_answer_comes_back_in_order_and_cleaned() {
        let answer = [
            line(3, "  Üç "),
            line(1, " Bir\n\n  satır "),
            line(2, "İki"),
        ];
        assert_eq!(
            validate(&[1, 2, 3], &answer).ok().unwrap(),
            vec!["Bir\nsatır", "İki", "Üç"]
        );
    }

    #[test]
    fn invalid_answers_name_the_lines_to_repair() {
        let Err(invalid) = validate(&[1, 2, 3], &[line(1, "one"), line(99, "extra")]) else {
            panic!("missing line accepted");
        };
        assert_eq!(invalid.repair_ids, vec![2, 3]);
        assert!(invalid.violations.contains(&Violation::Unknown { id: 99 }));
        assert!(invalid.violations.contains(&Violation::CountMismatch {
            expected: 3,
            received: 2
        }));

        let answer = [
            line(1, "one"),
            line(1, "again"),
            line(2, " \n "),
            line(3, "00:00:01,000 --> 00:00:02,000"),
        ];
        let Err(invalid) = validate(&[1, 2, 3], &answer) else {
            panic!("bad lines accepted");
        };
        assert_eq!(invalid.repair_ids, vec![1, 2, 3]);
        assert!(invalid.violations.contains(&Violation::Duplicate { id: 1 }));
        assert!(invalid.violations.contains(&Violation::Empty { id: 2 }));
        assert!(invalid.violations.contains(&Violation::Malformed { id: 3 }));
    }

    #[tokio::test]
    async fn targeted_repair_asks_only_for_the_missing_line() {
        let fake = Fake::scripted(vec![
            Ok(vec![line(1, "one"), line(3, "three")]),
            Ok(vec![line(2, "two")]),
        ]);
        let texts = translate_block(&fake, &block_request(&[1, 2, 3]))
            .await
            .unwrap();
        assert_eq!(texts, vec!["one", "two", "three"]);
        let asked = fake.asked();
        assert_eq!(asked.len(), 2);
        assert_eq!(asked[1].output_ids, vec![2]);
        assert_eq!(asked[1].mode, Mode::TargetedRepair);
        assert_eq!(asked[1].context, asked[0].context, "same context");
        assert_eq!(asked[1].analysis, asked[0].analysis, "same analysis");
    }

    #[tokio::test]
    async fn repair_budget_is_bounded_and_returns_nothing_partial() {
        let bad = || Ok(vec![line(1, "one"), line(3, "three")]);
        let fake = Fake::scripted(vec![bad(), Ok(Vec::new()), Ok(Vec::new()), bad()]);
        let err = translate_block(&fake, &block_request(&[1, 2, 3]))
            .await
            .unwrap_err();
        assert!(
            matches!(err, Failure::Exhausted { block: 1, .. }),
            "{err:?}"
        );
        let modes: Vec<(Mode, Vec<u32>)> = fake
            .asked()
            .into_iter()
            .map(|r| (r.mode, r.output_ids))
            .collect();
        assert_eq!(
            modes,
            vec![
                (Mode::Initial, vec![1, 2, 3]),
                (Mode::TargetedRepair, vec![2]),
                (Mode::TargetedRepair, vec![2]),
                (Mode::FullRetry, vec![1, 2, 3]),
            ]
        );
    }

    #[tokio::test]
    async fn unreadable_answers_use_the_repair_budget() {
        let good = || Ok(vec![line(1, "one"), line(2, "two")]);
        let fake = Fake::scripted(vec![Err(TranslateError::BadAnswer), good()]);
        let texts = translate_block(&fake, &block_request(&[1, 2]))
            .await
            .unwrap();
        assert_eq!(texts, vec!["one", "two"]);
        let asked = fake.asked();
        assert_eq!(asked[1].mode, Mode::TargetedRepair);
        assert_eq!(asked[1].output_ids, vec![1, 2], "every line asked again");

        let bad = || Err(TranslateError::BadAnswer);
        let fake = Fake::scripted(vec![bad(), bad(), bad(), bad(), good()]);
        let err = translate_block(&fake, &block_request(&[1, 2]))
            .await
            .unwrap_err();
        assert_eq!(fake.asked().len(), 4, "bounded like any wrong answer");
        assert_eq!(
            err,
            Failure::Exhausted {
                block: 1,
                violations: vec![Violation::Unreadable]
            }
        );
    }

    #[tokio::test]
    async fn translator_errors_are_not_retried() {
        for error in [TranslateError::Transient, TranslateError::Permanent] {
            let fake = Fake::scripted(vec![Err(error)]);
            let err = translate_block(&fake, &block_request(&[1, 2]))
                .await
                .unwrap_err();
            assert_eq!(err, Failure::Translator(error));
            assert_eq!(fake.asked().len(), 1);
        }
    }

    #[tokio::test]
    async fn document_is_analyzed_once_then_translated_block_by_block() {
        let source = cues(95);
        let fake = Fake::default();
        let mut prefixes = Vec::new();
        let out = translate(&fake, &source, &langs(), |done| prefixes.push(done.len()))
            .await
            .unwrap();

        let analyzed = fake.analyzed.lock().unwrap().clone();
        assert_eq!(analyzed.len(), 1);
        assert_eq!(analyzed[0].transcript.len(), 95);
        assert_eq!(analyzed[0].transcript[0], line(1, "Source line 1"));

        let asked = fake.asked();
        assert_eq!(asked.len(), layout(95).len());
        assert!(
            asked
                .iter()
                .all(|r| r.analysis == analysis() && r.blocks == asked.len())
        );
        assert_eq!(asked[0].block, 1);

        assert_eq!(out.len(), 95);
        for (got, want) in out.iter().zip(&source) {
            assert_eq!((got.start_ms, got.end_ms), (want.start_ms, want.end_ms));
            assert_eq!(got.text, format!("tr {}", want.text));
        }
        let ends: Vec<usize> = layout(95).iter().map(|b| b.output.end).collect();
        assert_eq!(prefixes, ends, "each block extends the served prefix");
    }

    #[tokio::test]
    async fn bad_analysis_stops_before_any_block() {
        let fake = Fake::default();
        *fake.analysis.lock().unwrap() = Some(Analysis {
            summary: "  ".to_owned(),
            ..analysis()
        });
        let err = translate(&fake, &cues(10), &langs(), |_| {})
            .await
            .unwrap_err();
        assert_eq!(err, Failure::BadAnalysis);
        assert!(fake.asked().is_empty());
        assert_eq!(
            translate(&fake, &[], &langs(), |_| {}).await,
            Err(Failure::Empty)
        );
    }

    #[tokio::test]
    async fn a_failed_block_keeps_earlier_blocks_served_but_fails_the_run() {
        // Block 1 passes; block 2 answers nothing on every attempt.
        let empty = || Ok(Vec::new());
        let fake = Fake::scripted(vec![
            Ok((1..=37).map(|id| line(id, "ok")).collect()),
            empty(),
            empty(),
            empty(),
            empty(),
        ]);
        let mut served = 0;
        let err = translate(&fake, &cues(80), &langs(), |done| served = done.len())
            .await
            .unwrap_err();
        assert!(
            matches!(err, Failure::Exhausted { block: 2, .. }),
            "{err:?}"
        );
        assert_eq!(served, layout(80)[0].output.end);
    }

    #[test]
    fn debug_never_prints_subtitle_text() {
        let secret = "Killua secret";
        let request = BlockRequest {
            context: vec![line(1, secret)],
            analysis: Analysis {
                summary: secret.to_owned(),
                characters: vec![Character {
                    name: secret.to_owned(),
                    description: secret.to_owned(),
                }],
                glossary: Vec::new(),
            },
            ..block_request(&[1])
        };
        let analysis_request = AnalysisRequest {
            langs: langs(),
            transcript: vec![line(1, secret)],
            terms: vec![secret.to_owned()],
        };
        for printed in [
            format!("{request:?}"),
            format!("{analysis_request:?}"),
            format!("{:?}", line(1, secret)),
            format!(
                "{:?}",
                Failure::Exhausted {
                    block: 1,
                    violations: vec![Violation::Empty { id: 1 }]
                }
            ),
        ] {
            assert!(!printed.contains("Killua"), "{printed}");
        }
    }
}
