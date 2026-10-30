//! K4 probe (SS-032, SS-033): what a non-Matroska file is, and for MP4 its
//! track list and the line times of English text tracks.
//!
//! Walks the top-level boxes from the first bytes already read, skips `mdat`
//! without reading it, and reads `moov` whole inside a fixed byte and request
//! budget. Times come from the sample tables (`stts` durations, `stsz`
//! sizes: a 2-byte `tx3g` sample is an empty gap). Media and subtitle text
//! are never read; track names are never parsed.

use std::time::Instant;

use crate::mkv::{BYTE_BUDGET, REQUEST_BUDGET, Rulers};
use crate::pgs::MIN_RULER_LINES;
use crate::range::{ByteSource, ReadError};
use crate::ruler::Event;

/// First read at a box beyond what is already in hand: covers the header
/// and, usually, a whole `moov` in the same request.
const PROBE_CHUNK: usize = 256 * 1024;
/// Top-level boxes walked before giving up on finding `moov`.
const MAX_TOP_BOXES: usize = 32;
/// A text sample this size or smaller holds no characters (`tx3g` and
/// QuickTime `text` start with a 16-bit length).
const EMPTY_SAMPLE: u32 = 2;

/// What the first bytes say the file is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Container {
    Matroska,
    Mp4,
    Avi,
    MpegTs,
    Unknown,
}

pub fn sniff(head: &[u8]) -> Container {
    const TS: usize = 188;
    const M2TS: usize = 192;
    let synced =
        |first: usize, step: usize| (0..3).all(|i| head.get(first + i * step) == Some(&0x47));
    if head.starts_with(&[0x1A, 0x45, 0xDF, 0xA3]) {
        Container::Matroska
    } else if head
        .get(4..8)
        .is_some_and(|t| matches!(t, b"ftyp" | b"moov" | b"mdat" | b"wide" | b"free"))
    {
        Container::Mp4
    } else if head.starts_with(b"RIFF") && head.get(8..12) == Some(b"AVI ") {
        Container::Avi
    } else if synced(0, TS) || synced(4, M2TS) {
        Container::MpegTs
    } else {
        Container::Unknown
    }
}

/// One track. Only the handler, sample format and language codes; never the
/// name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Mp4Track {
    pub id: u64,
    /// `hdlr` type: `vide`, `soun`, `text`, `sbtl`, `subt`, …
    pub handler: String,
    /// First `stsd` entry: `avc1`, `mp4a`, `tx3g`, `wvtt`, …
    pub codec: String,
    /// ISO 639-2, `mac<n>` for an old QuickTime code, or empty.
    pub lang: String,
    pub english: bool,
    pub samples: usize,
    /// Text tracks only: samples that hold characters.
    pub non_empty: Option<usize>,
}

impl Mp4Track {
    fn is_text(&self) -> bool {
        matches!(self.codec.as_str(), "tx3g" | "text")
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Mp4Report {
    pub requests: usize,
    /// Reads that failed once and were tried again.
    pub retries: usize,
    pub bytes_read: usize,
    pub elapsed_ms: u128,
    /// `moov` was inside the first read (a "fast start" file).
    pub moov_in_head: bool,
    /// `mvex` is present: samples live in movie fragments, not in `moov`.
    pub fragmented: bool,
    pub tracks: Vec<Mp4Track>,
    /// English text tracks with enough lines, the longest first.
    pub rulers: Rulers,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Mp4Error {
    Read(ReadError),
    Malformed(&'static str),
    OverBudget,
    NoMoov,
    TooLarge { bytes: u64 },
}

pub type Mp4Outcome = Result<Mp4Report, Mp4Error>;

/// Box header: four-letter type, total size (`None`: runs to end of file)
/// and header length.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct BoxHeader {
    kind: [u8; 4],
    size: Option<u64>,
    len: usize,
}

fn parse_box(buf: &[u8]) -> Option<BoxHeader> {
    let size32 = be_u32(buf, 0)?;
    let kind: [u8; 4] = buf.get(4..8)?.try_into().ok()?;
    let (size, len) = match size32 {
        0 => (None, 8),
        1 => (Some(be_u64(buf, 8)?), 16),
        n => (Some(u64::from(n)), 8),
    };
    if size.is_some_and(|s| s < len as u64) {
        return None;
    }
    Some(BoxHeader { kind, size, len })
}

fn be_u32(buf: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_be_bytes(buf.get(at..at + 4)?.try_into().ok()?))
}

fn be_u64(buf: &[u8], at: usize) -> Option<u64> {
    Some(u64::from_be_bytes(buf.get(at..at + 8)?.try_into().ok()?))
}

/// Child boxes of a fully read parent; stops at the first malformed one.
fn children(data: &[u8]) -> impl Iterator<Item = ([u8; 4], &[u8])> {
    let mut rest = data;
    std::iter::from_fn(move || {
        let header = parse_box(rest)?;
        let total = header.size.map_or(rest.len(), |s| s as usize);
        let body = rest.get(header.len..total)?;
        rest = &rest[total..];
        Some((header.kind, body))
    })
}

fn child<'a>(data: &'a [u8], kind: &[u8; 4]) -> Option<&'a [u8]> {
    children(data)
        .find(|(k, _)| k == kind)
        .map(|(_, body)| body)
}

fn path<'a>(data: &'a [u8], kinds: &[&[u8; 4]]) -> Option<&'a [u8]> {
    kinds.iter().try_fold(data, |data, kind| child(data, kind))
}

/// A four-letter code as printable text; anything else becomes `?`.
fn fourcc(bytes: &[u8]) -> String {
    bytes
        .iter()
        .map(|&b| {
            if b.is_ascii_alphanumeric() {
                b as char
            } else {
                '?'
            }
        })
        .collect()
}

struct Budget<'a> {
    file: &'a dyn ByteSource,
    bytes_left: usize,
    requests_left: usize,
    bytes_read: usize,
    requests: usize,
}

impl Budget<'_> {
    async fn read(&mut self, start: u64, len: usize) -> Result<Vec<u8>, Mp4Error> {
        if self.requests_left == 0 || len > self.bytes_left {
            return Err(Mp4Error::OverBudget);
        }
        self.requests_left -= 1;
        self.requests += 1;
        let bytes = self.file.read(start, len).await.map_err(Mp4Error::Read)?;
        self.bytes_left -= bytes.len().min(self.bytes_left);
        self.bytes_read += bytes.len();
        Ok(bytes)
    }
}

pub async fn inspect(file: &dyn ByteSource, head: &[u8]) -> Mp4Outcome {
    let started = Instant::now();
    let mut budget = Budget {
        file,
        bytes_left: BYTE_BUDGET,
        requests_left: REQUEST_BUDGET,
        bytes_read: 0,
        requests: 0,
    };
    let (moov, moov_in_head) = find_moov(&mut budget, head).await?;
    let (fragmented, tracks, rulers) = parse_moov(&moov)?;
    Ok(Mp4Report {
        requests: budget.requests,
        retries: file.retries(),
        bytes_read: budget.bytes_read,
        elapsed_ms: started.elapsed().as_millis(),
        moov_in_head,
        fragmented,
        tracks,
        rulers: Rulers(rulers),
    })
}

/// The data of the top-level `moov` box, and whether the head held it.
async fn find_moov(budget: &mut Budget<'_>, head: &[u8]) -> Result<(Vec<u8>, bool), Mp4Error> {
    // Bytes in hand starting at `at`: the head, or the last chunk read.
    let mut at = 0u64;
    let mut held: Vec<u8> = head.to_vec();
    let mut pos = 0u64;
    for _ in 0..MAX_TOP_BOXES {
        let offset = pos.checked_sub(at).map(|o| o as usize);
        let in_hand = offset.filter(|&o| o + 16 <= held.len());
        let offset = match in_hand {
            Some(offset) => offset,
            None => {
                held = budget.read(pos, PROBE_CHUNK.min(budget.bytes_left)).await?;
                at = pos;
                0
            }
        };
        if held.len() <= offset {
            break;
        }
        let header = parse_box(&held[offset..]).ok_or(Mp4Error::Malformed("box header"))?;
        if &header.kind == b"moov" {
            let size = header.size.ok_or(Mp4Error::Malformed("moov size"))?;
            let end = offset as u64 + size;
            if end > held.len() as u64 {
                let missing = end - held.len() as u64;
                if missing > budget.bytes_left as u64 {
                    return Err(Mp4Error::TooLarge { bytes: size });
                }
                let more = budget
                    .read(at + held.len() as u64, missing as usize)
                    .await?;
                held.extend_from_slice(&more);
            }
            let data = held
                .get(offset + header.len..end as usize)
                .ok_or(Mp4Error::Malformed("moov cut short"))?;
            return Ok((data.to_vec(), budget.requests == 0));
        }
        match header.size {
            Some(size) => pos += size,
            None => break,
        }
    }
    Err(Mp4Error::NoMoov)
}

type Parsed = (bool, Vec<Mp4Track>, Vec<(u64, Vec<Event>)>);

fn parse_moov(moov: &[u8]) -> Result<Parsed, Mp4Error> {
    let mvhd = child(moov, b"mvhd").ok_or(Mp4Error::Malformed("mvhd"))?;
    let movie_scale = full_box_u32(mvhd, 12, 20).ok_or(Mp4Error::Malformed("mvhd"))?;
    let fragmented = child(moov, b"mvex").is_some();
    let mut tracks = Vec::new();
    let mut rulers = Vec::new();
    for (kind, trak) in children(moov) {
        if &kind != b"trak" {
            continue;
        }
        let Some((track, events)) = parse_trak(trak, movie_scale) else {
            continue;
        };
        if track.english && track.is_text() && events.len() >= MIN_RULER_LINES {
            rulers.push((track.id, events));
        }
        tracks.push(track);
    }
    rulers.sort_by_key(|(_, events)| std::cmp::Reverse(events.len()));
    Ok((fragmented, tracks, rulers))
}

/// A `u32` field of a full box, at `v0` bytes in version 0 and `v1` in
/// version 1 (where earlier fields grow to 64 bits).
fn full_box_u32(body: &[u8], v0: usize, v1: usize) -> Option<u32> {
    match body.first()? {
        0 => be_u32(body, v0),
        1 => be_u32(body, v1),
        _ => None,
    }
}

/// The track and, for text tracks, its non-empty samples as events.
fn parse_trak(trak: &[u8], movie_scale: u32) -> Option<(Mp4Track, Vec<Event>)> {
    let id = full_box_u32(child(trak, b"tkhd")?, 12, 20)?;
    let mdia = child(trak, b"mdia")?;
    let mdhd = child(mdia, b"mdhd")?;
    let scale = full_box_u32(mdhd, 12, 20)?;
    let lang_at = if mdhd.first() == Some(&1) { 32 } else { 20 };
    let (lang, english) = language(u16::from_be_bytes(
        mdhd.get(lang_at..lang_at + 2)?.try_into().ok()?,
    ));
    let handler = fourcc(child(mdia, b"hdlr")?.get(8..12)?);
    let stbl = path(mdia, &[b"minf", b"stbl"])?;
    let codec = child(stbl, b"stsd")
        .and_then(|stsd| stsd.get(12..16))
        .map(fourcc)
        .unwrap_or_default();
    let sizes = child(stbl, b"stsz").and_then(sample_sizes);
    let mut track = Mp4Track {
        id: u64::from(id),
        handler,
        codec,
        lang,
        english,
        samples: sizes.as_ref().map_or(0, SampleSizes::count),
        non_empty: None,
    };
    if !track.is_text() {
        return Some((track, Vec::new()));
    }
    let sizes = sizes?;
    let durations = child(stbl, b"stts").and_then(time_to_sample)?;
    let shift = edit_shift(child(trak, b"edts"), movie_scale, scale);
    track.non_empty = Some(
        (0..sizes.count())
            .filter(|&i| sizes.get(i) > EMPTY_SAMPLE)
            .count(),
    );
    Some((track, events(&sizes, &durations, scale, shift)))
}

/// Packed ISO 639-2 (three 5-bit letters), or an old QuickTime code below
/// 0x400 where 0 is English.
fn language(code: u16) -> (String, bool) {
    if code < 0x400 {
        return (format!("mac{code}"), code == 0);
    }
    let letters: String = [10u16, 5, 0]
        .iter()
        .map(|&shift| (((code >> shift) & 0x1F) as u8 + 0x60) as char)
        .filter(char::is_ascii_lowercase)
        .collect();
    let english = letters == "eng";
    (letters, english)
}

enum SampleSizes {
    Fixed { size: u32, count: usize },
    Each(Vec<u32>),
}

impl SampleSizes {
    fn count(&self) -> usize {
        match self {
            Self::Fixed { count, .. } => *count,
            Self::Each(sizes) => sizes.len(),
        }
    }

    fn get(&self, i: usize) -> u32 {
        match self {
            Self::Fixed { size, .. } => *size,
            Self::Each(sizes) => sizes[i],
        }
    }
}

fn sample_sizes(stsz: &[u8]) -> Option<SampleSizes> {
    let size = be_u32(stsz, 4)?;
    let count = be_u32(stsz, 8)? as usize;
    if size != 0 {
        return Some(SampleSizes::Fixed { size, count });
    }
    let table = stsz.get(12..12 + count.checked_mul(4)?)?;
    Some(SampleSizes::Each(
        table
            .as_chunks::<4>()
            .0
            .iter()
            .map(|&c| u32::from_be_bytes(c))
            .collect(),
    ))
}

/// `stts` runs: (sample count, duration in media ticks).
fn time_to_sample(stts: &[u8]) -> Option<Vec<(u32, u32)>> {
    let count = be_u32(stts, 4)? as usize;
    let table = stts.get(8..8 + count.checked_mul(8)?)?;
    Some(
        table
            .as_chunks::<4>()
            .0
            .as_chunks::<2>()
            .0
            .iter()
            .map(|&[count, delta]| (u32::from_be_bytes(count), u32::from_be_bytes(delta)))
            .collect(),
    )
}

/// Media ticks to subtract from decode times so they land on the movie
/// timeline: the first media edit's start, minus leading empty edits
/// (which delay the track and are counted in movie ticks).
fn edit_shift(edts: Option<&[u8]>, movie_scale: u32, scale: u32) -> i64 {
    let Some(elst) = edts.and_then(|edts| child(edts, b"elst")) else {
        return 0;
    };
    let wide = elst.first() == Some(&1);
    let entry = if wide { 20 } else { 12 };
    let count = be_u32(elst, 4).unwrap_or(0) as usize;
    let mut delay_movie: u64 = 0;
    for i in 0..count {
        let at = 8 + i * entry;
        let (duration, media_time) = if wide {
            match (be_u64(elst, at), be_u64(elst, at + 8)) {
                (Some(d), Some(m)) => (d, m as i64),
                _ => break,
            }
        } else {
            match (be_u32(elst, at), be_u32(elst, at + 4)) {
                (Some(d), Some(m)) => (u64::from(d), i64::from(m as i32)),
                _ => break,
            }
        };
        if media_time == -1 {
            delay_movie = delay_movie.saturating_add(duration);
            continue;
        }
        let delay = if movie_scale == 0 {
            0
        } else {
            (u128::from(delay_movie) * u128::from(scale) / u128::from(movie_scale)) as i64
        };
        return media_time - delay;
    }
    0
}

fn events(sizes: &SampleSizes, runs: &[(u32, u32)], scale: u32, shift: i64) -> Vec<Event> {
    if scale == 0 {
        return Vec::new();
    }
    let to_ms = |ticks: i64| -> u64 {
        let ticks = (ticks - shift).max(0) as u128;
        u64::try_from(ticks * 1000 / u128::from(scale)).unwrap_or(u64::MAX)
    };
    let mut out = Vec::new();
    let mut dts: i64 = 0;
    let mut i = 0;
    'runs: for &(count, delta) in runs {
        for _ in 0..count {
            if i >= sizes.count() {
                break 'runs;
            }
            let end = dts.saturating_add(i64::from(delta));
            if sizes.get(i) > EMPTY_SAMPLE && delta > 0 {
                out.push(Event {
                    start_ms: to_ms(dts),
                    end_ms: to_ms(end),
                });
            }
            dts = end;
            i += 1;
        }
    }
    out.sort_unstable();
    out.dedup();
    out
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::range::ReadFuture;
    use std::sync::Mutex;

    fn bx(kind: &[u8; 4], body: &[u8]) -> Vec<u8> {
        let mut out = ((body.len() + 8) as u32).to_be_bytes().to_vec();
        out.extend_from_slice(kind);
        out.extend_from_slice(body);
        out
    }

    fn full(kind: &[u8; 4], body: &[u8]) -> Vec<u8> {
        let mut data = vec![0, 0, 0, 0];
        data.extend_from_slice(body);
        bx(kind, &data)
    }

    fn u32s(values: &[u32]) -> Vec<u8> {
        values.iter().flat_map(|v| v.to_be_bytes()).collect()
    }

    fn packed(lang: &str) -> u16 {
        lang.bytes()
            .fold(0u16, |code, b| (code << 5) | u16::from(b - 0x60))
    }

    pub(crate) struct Track<'a> {
        pub id: u32,
        pub handler: &'a [u8; 4],
        pub codec: &'a [u8; 4],
        pub lang: u16,
        pub scale: u32,
        /// (duration in ticks, sample size) per sample.
        pub samples: Vec<(u32, u32)>,
        /// (segment duration in movie ticks, media time) per edit.
        pub edits: Vec<(u32, i32)>,
    }

    impl Track<'_> {
        pub(crate) fn text(id: u32, lang: &str, samples: Vec<(u32, u32)>) -> Self {
            Track {
                id,
                handler: b"sbtl",
                codec: b"tx3g",
                lang: packed(lang),
                scale: 1000,
                samples,
                edits: Vec::new(),
            }
        }
    }

    fn trak(t: &Track) -> Vec<u8> {
        let tkhd = full(b"tkhd", &u32s(&[0, 0, t.id, 0, 0]));
        let mut mdhd = u32s(&[0, 0, t.scale, 0]);
        mdhd.extend_from_slice(&t.lang.to_be_bytes());
        mdhd.extend_from_slice(&[0, 0]);
        let mdhd = full(b"mdhd", &mdhd);
        let mut hdlr = u32s(&[0]);
        hdlr.extend_from_slice(t.handler);
        hdlr.extend_from_slice(&[0; 12]);
        hdlr.extend_from_slice(b"Secret Name\0");
        let hdlr = full(b"hdlr", &hdlr);
        let mut stsd = u32s(&[1]);
        stsd.extend(bx(t.codec, &[0; 8]));
        let stsd = full(b"stsd", &stsd);
        let mut stts = vec![t.samples.len() as u32];
        for &(delta, _) in &t.samples {
            stts.extend([1, delta]);
        }
        let stts = full(b"stts", &u32s(&stts));
        let mut stsz = vec![0, t.samples.len() as u32];
        stsz.extend(t.samples.iter().map(|&(_, size)| size));
        let stsz = full(b"stsz", &u32s(&stsz));
        let stbl = bx(b"stbl", &[stsd, stts, stsz].concat());
        let minf = bx(b"minf", &stbl);
        let mdia = bx(b"mdia", &[mdhd, hdlr, minf].concat());
        let mut body = tkhd;
        if !t.edits.is_empty() {
            let mut elst = vec![t.edits.len() as u32];
            for &(duration, media) in &t.edits {
                elst.extend([duration, media as u32, 0x0001_0000]);
            }
            body.extend(bx(b"edts", &full(b"elst", &u32s(&elst))));
        }
        body.extend(mdia);
        bx(b"trak", &body)
    }

    pub(crate) fn moov(tracks: &[Track], fragmented: bool) -> Vec<u8> {
        let mut body = full(b"mvhd", &u32s(&[0, 0, 600, 0]));
        for t in tracks {
            body.extend(trak(t));
        }
        if fragmented {
            body.extend(bx(b"mvex", &[]));
        }
        bx(b"moov", &body)
    }

    fn ftyp() -> Vec<u8> {
        bx(b"ftyp", b"isom\0\0\0\0isomavc1")
    }

    fn video() -> Track<'static> {
        Track {
            id: 1,
            handler: b"vide",
            codec: b"avc1",
            lang: packed("und"),
            scale: 24000,
            samples: vec![(1001, 5000); 10],
            edits: Vec::new(),
        }
    }

    /// `n` lines of 1 s each, a 1 s empty gap after every one.
    fn lines(n: usize) -> Vec<(u32, u32)> {
        (0..n).flat_map(|_| [(1000, 20), (1000, 2)]).collect()
    }

    /// `ftyp`, then `moov` before or after an `mdat` of `media` bytes.
    pub(crate) fn file(moov: Vec<u8>, media: usize, moov_first: bool) -> Vec<u8> {
        let mdat = bx(b"mdat", &vec![0xAB; media]);
        let mut out = ftyp();
        if moov_first {
            out.extend(moov);
            out.extend(mdat);
        } else {
            out.extend(mdat);
            out.extend(moov);
        }
        out
    }

    pub(crate) struct MemFile {
        bytes: Vec<u8>,
        pub reads: Mutex<Vec<(u64, usize)>>,
    }

    impl MemFile {
        pub(crate) fn new(bytes: Vec<u8>) -> Self {
            Self {
                bytes,
                reads: Mutex::new(Vec::new()),
            }
        }
    }

    impl ByteSource for MemFile {
        fn read(&self, start: u64, len: usize) -> ReadFuture<'_> {
            self.reads.lock().unwrap().push((start, len));
            let start = (start as usize).min(self.bytes.len());
            let end = start.saturating_add(len).min(self.bytes.len());
            let out = self.bytes[start..end].to_vec();
            Box::pin(async move { Ok(out) })
        }
    }

    fn head(bytes: &[u8]) -> Vec<u8> {
        bytes[..bytes.len().min(crate::range::HEAD_BYTES)].to_vec()
    }

    #[test]
    fn sniff_tells_containers_apart() {
        assert_eq!(sniff(&[0x1A, 0x45, 0xDF, 0xA3, 0x01]), Container::Matroska);
        assert_eq!(sniff(&ftyp()), Container::Mp4);
        assert_eq!(sniff(b"RIFF\x10\0\0\0AVI LIST"), Container::Avi);
        assert_eq!(sniff(b"RIFF\x10\0\0\0WAVEfmt "), Container::Unknown);
        let mut ts = vec![0u8; 188 * 3];
        for i in 0..3 {
            ts[i * 188] = 0x47;
        }
        assert_eq!(sniff(&ts), Container::MpegTs);
        let mut m2ts = vec![0u8; 192 * 3 + 4];
        for i in 0..3 {
            m2ts[4 + i * 192] = 0x47;
        }
        assert_eq!(sniff(&m2ts), Container::MpegTs);
        assert_eq!(sniff(b"<html><body>"), Container::Unknown);
        assert_eq!(sniff(&[]), Container::Unknown);
    }

    #[tokio::test]
    async fn fast_start_moov_needs_no_read() {
        let moov = moov(&[video(), Track::text(2, "eng", lines(60))], false);
        let bytes = file(moov, 1_000_000, true);
        let mem = MemFile::new(bytes.clone());
        let report = inspect(&mem, &head(&bytes)).await.unwrap();
        assert_eq!(report.requests, 0);
        assert!(report.moov_in_head);
        assert!(!report.fragmented);
        assert_eq!(report.tracks.len(), 2);
        assert_eq!(report.tracks[0].handler, "vide");
        assert_eq!(report.tracks[0].codec, "avc1");
        assert_eq!(report.tracks[0].samples, 10);
        assert_eq!(report.tracks[0].non_empty, None);
        let text = &report.tracks[1];
        assert_eq!(
            (text.id, text.lang.as_str(), text.english),
            (2, "eng", true)
        );
        assert_eq!((text.samples, text.non_empty), (120, Some(60)));
        assert_eq!(report.rulers.0.len(), 1);
        let (track, events) = &report.rulers.0[0];
        assert_eq!(*track, 2);
        assert_eq!(events.len(), 60);
        assert_eq!(
            events[0],
            Event {
                start_ms: 0,
                end_ms: 1000
            }
        );
        assert_eq!(
            events[1],
            Event {
                start_ms: 2000,
                end_ms: 3000
            }
        );
    }

    #[tokio::test]
    async fn moov_after_mdat_is_found_without_reading_media() {
        let moov = moov(&[video(), Track::text(2, "eng", lines(60))], false);
        let media = 5_000_000;
        let bytes = file(moov, media, false);
        let mem = MemFile::new(bytes.clone());
        let report = inspect(&mem, &head(&bytes)).await.unwrap();
        assert!(!report.moov_in_head);
        assert_eq!(report.requests, 1, "one read at moov");
        let reads = mem.reads.lock().unwrap();
        let moov_at = (ftyp().len() + 8 + media) as u64;
        assert_eq!(reads[0].0, moov_at);
        assert!(report.bytes_read < 300_000);
        assert_eq!(report.rulers.0[0].1.len(), 60);
    }

    #[tokio::test]
    async fn large_mdat_size_and_large_moov_are_followed() {
        // 64-bit mdat size; moov larger than one probe chunk.
        let mut tracks = vec![video()];
        tracks[0].samples = vec![(1001, 5000); 80_000];
        tracks.push(Track::text(3, "eng", lines(60)));
        let moov = moov(&tracks, false);
        assert!(moov.len() > PROBE_CHUNK);
        let media = 100_000u64;
        let mut bytes = ftyp();
        bytes.extend(1u32.to_be_bytes());
        bytes.extend(b"mdat");
        bytes.extend((media + 16).to_be_bytes());
        bytes.extend(vec![0; media as usize]);
        bytes.extend(moov);
        let mem = MemFile::new(bytes.clone());
        let report = inspect(&mem, &head(&bytes)).await.unwrap();
        assert_eq!(report.requests, 2, "chunk, then the rest of moov");
        assert_eq!(report.tracks[0].samples, 80_000);
        assert_eq!(report.rulers.0[0].0, 3);
    }

    #[tokio::test]
    async fn edit_list_shifts_times_onto_the_movie_timeline() {
        let mut text = Track::text(2, "eng", lines(60));
        // 1.2 s empty edit (movie scale 600), then media from 0.5 s.
        text.edits = vec![(720, -1), (60_000, 500)];
        let bytes = file(moov(&[text], false), 10, true);
        let report = inspect(&MemFile::new(bytes.clone()), &bytes).await.unwrap();
        let events = &report.rulers.0[0].1;
        // dts 0 → 0 - (500 - 1200) = 700 ms.
        assert_eq!(
            events[0],
            Event {
                start_ms: 700,
                end_ms: 1700
            }
        );
    }

    #[tokio::test]
    async fn only_long_english_text_tracks_become_rulers() {
        let mut vtt = Track::text(5, "eng", lines(60));
        vtt.codec = b"wvtt";
        let mut old = Track::text(6, "eng", lines(70));
        old.lang = 0;
        old.codec = b"text";
        old.handler = b"text";
        let tracks = [
            Track::text(2, "tur", lines(60)),
            Track::text(3, "und", lines(60)),
            Track::text(4, "eng", lines(MIN_RULER_LINES - 1)),
            vtt,
            old,
            Track::text(7, "eng", lines(80)),
        ];
        let bytes = file(moov(&tracks, false), 10, true);
        let report = inspect(&MemFile::new(bytes.clone()), &bytes).await.unwrap();
        assert_eq!(report.tracks.len(), 6);
        assert_eq!(report.tracks[3].codec, "wvtt");
        assert_eq!(report.tracks[3].non_empty, None);
        assert_eq!(report.tracks[4].lang, "mac0");
        let order: Vec<u64> = report.rulers.0.iter().map(|(t, _)| *t).collect();
        assert_eq!(order, vec![7, 6], "longest first");
    }

    #[tokio::test]
    async fn fragmented_file_lists_tracks_but_has_no_samples() {
        let text = Track::text(2, "eng", Vec::new());
        let bytes = file(moov(&[video(), text], true), 10, true);
        let report = inspect(&MemFile::new(bytes.clone()), &bytes).await.unwrap();
        assert!(report.fragmented);
        assert_eq!(report.tracks.len(), 2);
        assert!(report.rulers.0.is_empty());
    }

    #[tokio::test]
    async fn missing_or_broken_moov_is_an_error_not_a_panic() {
        let bytes = [ftyp(), bx(b"mdat", &[0; 100])].concat();
        let mem = MemFile::new(bytes.clone());
        assert_eq!(inspect(&mem, &bytes).await, Err(Mp4Error::NoMoov));

        let mut bytes = ftyp();
        bytes.extend(4u32.to_be_bytes());
        bytes.extend(b"junk");
        let mem = MemFile::new(bytes.clone());
        assert_eq!(
            inspect(&mem, &bytes).await,
            Err(Mp4Error::Malformed("box header"))
        );

        let mut bytes = ftyp();
        bytes.extend(((BYTE_BUDGET as u32) * 2).to_be_bytes());
        bytes.extend(b"moov");
        bytes.extend([0; 100]);
        let mem = MemFile::new(bytes.clone());
        assert!(matches!(
            inspect(&mem, &bytes).await,
            Err(Mp4Error::TooLarge { .. })
        ));
    }

    #[tokio::test]
    async fn report_prints_counts_not_names_or_times() {
        let bytes = file(moov(&[Track::text(2, "eng", lines(60))], false), 10, true);
        let report = inspect(&MemFile::new(bytes.clone()), &bytes).await.unwrap();
        let printed = format!("{report:?}");
        assert!(!printed.contains("Secret"), "{printed}");
        assert!(printed.contains("rulers: [(2, 60)]"), "{printed}");
        assert!(!printed.contains("start_ms"), "{printed}");
        // The dashboard reads tracks from this line; keep the field order.
        assert!(
            printed.contains(
                r#"Mp4Track { id: 2, handler: "sbtl", codec: "tx3g", lang: "eng", english: true, samples: 120, non_empty: Some(60) }"#
            ),
            "{printed}"
        );
    }
}
