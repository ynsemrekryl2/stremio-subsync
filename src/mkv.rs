//! K3 probe: reads a Matroska file's track list and cue index through range
//! reads, to see whether subtitle tracks are indexed in Cues.
//!
//! Starts from the first bytes already read, follows the SeekHead to Tracks
//! and Cues, and stays inside a fixed byte and request budget. Track names
//! and subtitle text are never read into the report.

use std::collections::HashMap;
use std::time::Instant;

use crate::range::{ByteSource, ReadError};

const EBML: u32 = 0x1A45_DFA3;
const SEGMENT: u32 = 0x1853_8067;
const SEEK_HEAD: u32 = 0x114D_9B74;
const SEEK: u32 = 0x4DBB;
const SEEK_ID: u32 = 0x53AB;
const SEEK_POSITION: u32 = 0x53AC;
const TRACKS: u32 = 0x1654_AE6B;
const TRACK_ENTRY: u32 = 0xAE;
const TRACK_NUMBER: u32 = 0xD7;
const TRACK_TYPE: u32 = 0x83;
const CODEC_ID: u32 = 0x86;
const LANGUAGE: u32 = 0x22_B59C;
const LANGUAGE_BCP47: u32 = 0x22_B59D;
const FLAG_DEFAULT: u32 = 0x88;
const FLAG_FORCED: u32 = 0x55AA;
const CUES: u32 = 0x1C53_BB6B;
const CUE_POINT: u32 = 0xBB;
const CUE_TRACK_POSITIONS: u32 = 0xB7;
const CUE_TRACK: u32 = 0xF7;

const TRACK_TYPE_VIDEO: u64 = 1;
const TRACK_TYPE_AUDIO: u64 = 2;
const TRACK_TYPE_SUBTITLE: u64 = 0x11;

/// Bytes that may be read after the first chunk, per file.
pub const BYTE_BUDGET: usize = 8 * 1024 * 1024;
/// Range requests that may be made after the first one, per file.
pub const REQUEST_BUDGET: usize = 6;
/// First read when fetching an element whose size is not known yet.
const PROBE_CHUNK: usize = 256 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Codec {
    Srt,
    Ass,
    WebVtt,
    OtherText,
    Pgs,
    VobSub,
    Other,
}

impl Codec {
    fn from_id(id: &str) -> Self {
        match id {
            "S_TEXT/UTF8" | "S_TEXT/ASCII" => Self::Srt,
            "S_TEXT/ASS" | "S_TEXT/SSA" | "S_ASS" | "S_SSA" => Self::Ass,
            "S_TEXT/WEBVTT" => Self::WebVtt,
            "S_HDMV/PGS" => Self::Pgs,
            "S_VOBSUB" => Self::VobSub,
            other if other.starts_with("S_TEXT/") => Self::OtherText,
            _ => Self::Other,
        }
    }

    fn is_text(self) -> bool {
        matches!(self, Self::Srt | Self::Ass | Self::WebVtt | Self::OtherText)
    }
}

/// One subtitle track. Only codec, language code and flags; never the name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SubTrack {
    pub number: u64,
    pub codec: Codec,
    /// ISO 639-2 or BCP 47 code, cut to 8 safe characters.
    pub lang: String,
    pub english: bool,
    pub default: bool,
    pub forced: bool,
    /// Cue points that carry a position for this track.
    pub cue_points: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CuesState {
    /// Neither the SeekHead nor the scanned head points to Cues.
    NotReferenced,
    /// The Cues element is larger than the remaining budget.
    TooLarge {
        bytes: u64,
    },
    Failed(MkvError),
    Read {
        bytes: u64,
        points: usize,
        /// Cue points that carry a position for a video track.
        video_points: usize,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MkvReport {
    pub requests: usize,
    /// Reads that failed once and were tried again (included in `requests`
    /// only once).
    pub retries: usize,
    pub bytes_read: usize,
    pub elapsed_ms: u128,
    pub seek_entries: usize,
    pub video_tracks: usize,
    pub audio_tracks: usize,
    pub subtitles: Vec<SubTrack>,
    pub cues: CuesState,
    /// The K3 answer for this file: does any English text subtitle track
    /// appear in Cues? `None` when there is no English text track.
    pub english_text_indexed: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MkvError {
    Read(ReadError),
    Malformed(&'static str),
    OverBudget,
    NoTracks,
}

pub type MkvOutcome = Result<MkvReport, MkvError>;

/// Element header: ID, data size (`None` for unknown size) and header length.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Header {
    id: u32,
    size: Option<u64>,
    len: usize,
}

fn vint_len(first: u8) -> Option<usize> {
    let len = first.leading_zeros() as usize + 1;
    (len <= 8).then_some(len)
}

fn parse_header(buf: &[u8]) -> Option<Header> {
    let id_len = vint_len(*buf.first()?)?;
    if id_len > 4 || buf.len() < id_len {
        return None;
    }
    let id = buf[..id_len]
        .iter()
        .fold(0u32, |acc, &b| (acc << 8) | u32::from(b));
    let rest = &buf[id_len..];
    let size_len = vint_len(*rest.first()?)?;
    if rest.len() < size_len {
        return None;
    }
    let mask = if size_len == 8 { 0 } else { 0xFFu8 >> size_len };
    let value = rest[1..size_len]
        .iter()
        .fold(u64::from(rest[0] & mask), |acc, &b| {
            (acc << 8) | u64::from(b)
        });
    let unknown = value == (1u64 << (7 * size_len)) - 1;
    Some(Header {
        id,
        size: (!unknown).then_some(value),
        len: id_len + size_len,
    })
}

/// Child elements of a master element's data. Stops at the first header
/// that is malformed, of unknown size, or runs past the data.
fn children(data: &[u8]) -> Vec<(u32, &[u8])> {
    let mut out = Vec::new();
    let mut pos = 0usize;
    while pos < data.len() {
        let Some(header) = parse_header(&data[pos..]) else {
            break;
        };
        let Some(size) = header.size.and_then(|s| usize::try_from(s).ok()) else {
            break;
        };
        let start = pos + header.len;
        let Some(end) = start.checked_add(size).filter(|&end| end <= data.len()) else {
            break;
        };
        out.push((header.id, &data[start..end]));
        pos = end;
    }
    out
}

fn uint(data: &[u8]) -> Option<u64> {
    (data.len() <= 8).then(|| data.iter().fold(0u64, |acc, &b| (acc << 8) | u64::from(b)))
}

fn text(data: &[u8]) -> String {
    String::from_utf8_lossy(data)
        .trim_end_matches('\0')
        .to_owned()
}

fn safe_lang(raw: &str) -> String {
    raw.chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '-')
        .take(8)
        .collect()
}

fn parse_seek_head(data: &[u8]) -> Vec<(u32, u64)> {
    children(data)
        .into_iter()
        .filter(|(id, _)| *id == SEEK)
        .filter_map(|(_, seek)| {
            let mut target = None;
            let mut position = None;
            for (id, value) in children(seek) {
                match id {
                    SEEK_ID if value.len() <= 4 => target = uint(value).map(|v| v as u32),
                    SEEK_POSITION => position = uint(value),
                    _ => {}
                }
            }
            Some((target?, position?))
        })
        .collect()
}

#[derive(Default)]
struct TrackList {
    video: Vec<u64>,
    audio: usize,
    subtitles: Vec<SubTrack>,
}

fn parse_tracks(data: &[u8]) -> TrackList {
    let mut list = TrackList::default();
    for (_, entry) in children(data)
        .into_iter()
        .filter(|(id, _)| *id == TRACK_ENTRY)
    {
        let mut number = None;
        let mut kind = None;
        let mut codec = String::new();
        // Matroska defaults: language "eng", default flag on, forced flag off.
        let mut lang = "eng".to_owned();
        let mut bcp47 = None;
        let mut default = true;
        let mut forced = false;
        for (id, value) in children(entry) {
            match id {
                TRACK_NUMBER => number = uint(value),
                TRACK_TYPE => kind = uint(value),
                CODEC_ID => codec = text(value),
                LANGUAGE => lang = text(value),
                LANGUAGE_BCP47 => bcp47 = Some(text(value)),
                FLAG_DEFAULT => default = uint(value).is_some_and(|v| v != 0),
                FLAG_FORCED => forced = uint(value).is_some_and(|v| v != 0),
                _ => {}
            }
        }
        let Some(number) = number else { continue };
        match kind {
            Some(TRACK_TYPE_VIDEO) => list.video.push(number),
            Some(TRACK_TYPE_AUDIO) => list.audio += 1,
            Some(TRACK_TYPE_SUBTITLE) => {
                let code = bcp47.unwrap_or(lang);
                let lower = code.to_ascii_lowercase();
                let english = lower == "eng" || lower == "en" || lower.starts_with("en-");
                list.subtitles.push(SubTrack {
                    number,
                    codec: Codec::from_id(&codec),
                    lang: safe_lang(&code),
                    english,
                    default,
                    forced,
                    cue_points: 0,
                });
            }
            _ => {}
        }
    }
    list
}

/// Cue point count, and per track number the cue points that mention it.
type CueCounts = (usize, HashMap<u64, usize>);

fn parse_cues(data: &[u8]) -> CueCounts {
    let mut points = 0;
    let mut per_track: HashMap<u64, usize> = HashMap::new();
    for (_, point) in children(data)
        .into_iter()
        .filter(|(id, _)| *id == CUE_POINT)
    {
        points += 1;
        let mut seen = Vec::new();
        for (_, positions) in children(point)
            .into_iter()
            .filter(|(id, _)| *id == CUE_TRACK_POSITIONS)
        {
            for (id, value) in children(positions) {
                if id == CUE_TRACK
                    && let Some(track) = uint(value)
                    && !seen.contains(&track)
                {
                    seen.push(track);
                    *per_track.entry(track).or_default() += 1;
                }
            }
        }
    }
    (points, per_track)
}

struct Budget<'a> {
    file: &'a dyn ByteSource,
    bytes_left: usize,
    requests_left: usize,
    bytes_read: usize,
    requests: usize,
}

impl Budget<'_> {
    async fn read(&mut self, start: u64, len: usize) -> Result<Vec<u8>, MkvError> {
        if self.requests_left == 0 || len > self.bytes_left {
            return Err(MkvError::OverBudget);
        }
        self.requests_left -= 1;
        self.requests += 1;
        let bytes = self.file.read(start, len).await.map_err(MkvError::Read)?;
        self.bytes_left -= bytes.len().min(self.bytes_left);
        self.bytes_read += bytes.len();
        Ok(bytes)
    }

    /// Reads one element at `start` and returns its data. The size check
    /// happens before the full read, so an oversized element costs one chunk.
    async fn element(&mut self, start: u64, expect: u32) -> Result<Vec<u8>, ElementError> {
        let chunk = PROBE_CHUNK.min(self.bytes_left);
        let mut buf = self.read(start, chunk).await?;
        let header = parse_header(&buf).ok_or(MkvError::Malformed("element header"))?;
        if header.id != expect {
            return Err(MkvError::Malformed("seek target id").into());
        }
        let size = header.size.ok_or(MkvError::Malformed("unknown size"))?;
        let total = (header.len as u64).saturating_add(size);
        if total > buf.len() as u64 {
            let missing = total - buf.len() as u64;
            if missing > self.bytes_left as u64 {
                return Err(ElementError::TooLarge { bytes: size });
            }
            let more = self
                .read(start + buf.len() as u64, missing as usize)
                .await?;
            buf.extend_from_slice(&more);
        }
        let end = usize::try_from(total).map_err(|_| MkvError::Malformed("size"))?;
        if buf.len() < end {
            return Err(MkvError::Malformed("truncated element").into());
        }
        Ok(buf[header.len..end].to_vec())
    }
}

enum ElementError {
    TooLarge { bytes: u64 },
    Mkv(MkvError),
}

impl From<MkvError> for ElementError {
    fn from(err: MkvError) -> Self {
        Self::Mkv(err)
    }
}

/// What the head of the file told us before any further reads.
#[derive(Default)]
struct HeadScan {
    segment_data: u64,
    /// Segment-relative positions of top-level elements seen in the head.
    seen: HashMap<u32, u64>,
    seek: Vec<(u32, u64)>,
    tracks: Option<TrackList>,
    cues: Option<(u64, CueCounts)>,
}

fn scan_head(head: &[u8]) -> Result<HeadScan, MkvError> {
    let ebml = parse_header(head).ok_or(MkvError::Malformed("ebml header"))?;
    if ebml.id != EBML {
        return Err(MkvError::Malformed("not ebml"));
    }
    let ebml_end = ebml
        .size
        .and_then(|s| usize::try_from(s).ok())
        .and_then(|s| s.checked_add(ebml.len))
        .ok_or(MkvError::Malformed("ebml size"))?;
    let segment = head
        .get(ebml_end..)
        .and_then(parse_header)
        .ok_or(MkvError::Malformed("segment header"))?;
    if segment.id != SEGMENT {
        return Err(MkvError::Malformed("no segment"));
    }
    let data_start = ebml_end + segment.len;
    let mut scan = HeadScan {
        segment_data: data_start as u64,
        ..HeadScan::default()
    };
    let mut pos = data_start;
    // Clusters of known size are stepped over: Cues may follow them in small files.
    while let Some(header) = head.get(pos..).and_then(parse_header) {
        scan.seen
            .entry(header.id)
            .or_insert((pos - data_start) as u64);
        let Some(size) = header.size.and_then(|s| usize::try_from(s).ok()) else {
            break;
        };
        let start = pos + header.len;
        let Some(end) = start.checked_add(size).filter(|&end| end <= head.len()) else {
            break;
        };
        let data = &head[start..end];
        match header.id {
            SEEK_HEAD => scan.seek.extend(parse_seek_head(data)),
            TRACKS => scan.tracks = Some(parse_tracks(data)),
            CUES => scan.cues = Some((size as u64, parse_cues(data))),
            _ => {}
        }
        pos = end;
    }
    Ok(scan)
}

fn position_of(scan: &HeadScan, id: u32) -> Option<u64> {
    scan.seek
        .iter()
        .find(|(target, _)| *target == id)
        .map(|(_, pos)| *pos)
        .or_else(|| scan.seen.get(&id).copied())
}

pub async fn inspect(file: &dyn ByteSource, head: &[u8]) -> MkvOutcome {
    let started = Instant::now();
    let mut scan = scan_head(head)?;
    let mut budget = Budget {
        file,
        bytes_left: BYTE_BUDGET,
        requests_left: REQUEST_BUDGET,
        bytes_read: 0,
        requests: 0,
    };

    // A second SeekHead (often at the end) may hold the Cues entry.
    if position_of(&scan, CUES).is_none() {
        let first = scan.seen.get(&SEEK_HEAD).copied();
        let other = scan
            .seek
            .iter()
            .find(|(id, pos)| *id == SEEK_HEAD && Some(*pos) != first)
            .map(|(_, pos)| *pos);
        if let Some(pos) = other
            && let Ok(data) = budget.element(scan.segment_data + pos, SEEK_HEAD).await
        {
            scan.seek.extend(parse_seek_head(&data));
        }
    }

    let tracks = match scan.tracks.take() {
        Some(tracks) => tracks,
        None => {
            let pos = position_of(&scan, TRACKS).ok_or(MkvError::NoTracks)?;
            match budget.element(scan.segment_data + pos, TRACKS).await {
                Ok(data) => parse_tracks(&data),
                Err(ElementError::Mkv(err)) => return Err(err),
                Err(ElementError::TooLarge { .. }) => return Err(MkvError::OverBudget),
            }
        }
    };

    let cues = match scan.cues.take() {
        Some(found) => Ok(found),
        None => match position_of(&scan, CUES) {
            None => Err(CuesState::NotReferenced),
            Some(pos) => match budget.element(scan.segment_data + pos, CUES).await {
                Ok(data) => Ok((data.len() as u64, parse_cues(&data))),
                Err(ElementError::TooLarge { bytes }) => Err(CuesState::TooLarge { bytes }),
                Err(ElementError::Mkv(err)) => Err(CuesState::Failed(err)),
            },
        },
    };

    let mut subtitles = tracks.subtitles;
    let cues = match cues {
        Ok((bytes, (points, per_track))) => {
            for sub in &mut subtitles {
                sub.cue_points = per_track.get(&sub.number).copied().unwrap_or(0);
            }
            let video_points = tracks
                .video
                .iter()
                .map(|n| per_track.get(n).copied().unwrap_or(0))
                .max()
                .unwrap_or(0);
            CuesState::Read {
                bytes,
                points,
                video_points,
            }
        }
        Err(state) => state,
    };
    let english_text_indexed = matches!(cues, CuesState::Read { .. })
        .then(|| {
            let english: Vec<_> = subtitles
                .iter()
                .filter(|s| s.english && s.codec.is_text())
                .collect();
            (!english.is_empty()).then(|| english.iter().any(|s| s.cue_points > 0))
        })
        .flatten();

    Ok(MkvReport {
        requests: budget.requests,
        retries: file.retries(),
        bytes_read: budget.bytes_read,
        elapsed_ms: started.elapsed().as_millis(),
        seek_entries: scan.seek.len(),
        video_tracks: tracks.video.len(),
        audio_tracks: tracks.audio,
        subtitles,
        cues,
        english_text_indexed,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::range::ReadFuture;
    use std::sync::Mutex;

    const CLUSTER: u32 = 0x1F43_B675;

    /// Element with a minimal ID encoding and an 8-byte size.
    fn el(id: u32, data: &[u8]) -> Vec<u8> {
        let id_bytes = id.to_be_bytes();
        let skip = id_bytes.iter().take_while(|&&b| b == 0).count();
        let mut out = id_bytes[skip..].to_vec();
        out.push(0x01);
        out.extend_from_slice(&(data.len() as u64).to_be_bytes()[1..]);
        out.extend_from_slice(data);
        out
    }

    fn uint_el(id: u32, value: u64) -> Vec<u8> {
        el(id, &value.to_be_bytes())
    }

    fn track(number: u64, kind: u64, codec: &str, lang: Option<&str>) -> Vec<u8> {
        let mut data = uint_el(TRACK_NUMBER, number);
        data.extend(uint_el(TRACK_TYPE, kind));
        data.extend(el(CODEC_ID, codec.as_bytes()));
        if let Some(lang) = lang {
            data.extend(el(LANGUAGE, lang.as_bytes()));
        }
        el(TRACK_ENTRY, &data)
    }

    fn cue_point(time: u64, tracks: &[u64]) -> Vec<u8> {
        let mut data = uint_el(0xB3, time);
        for &t in tracks {
            let mut pos = uint_el(CUE_TRACK, t);
            pos.extend(uint_el(0xF1, 0));
            data.extend(el(CUE_TRACK_POSITIONS, &pos));
        }
        el(CUE_POINT, &data)
    }

    fn seek(id: u32, pos: u64) -> Vec<u8> {
        let id_bytes = id.to_be_bytes();
        let skip = id_bytes.iter().take_while(|&&b| b == 0).count();
        let mut data = el(SEEK_ID, &id_bytes[skip..]);
        data.extend(uint_el(SEEK_POSITION, pos));
        el(SEEK, &data)
    }

    /// EBML header + Segment with SeekHead, Tracks, a padding Cluster and Cues
    /// at the end. Track 1 video, 2 English SRT (indexed), 3 English PGS,
    /// 4 Turkish SRT. `padding` pushes Cues (and optionally Tracks) far away.
    fn sample_file(padding: usize, tracks_late: bool) -> Vec<u8> {
        let mut tracks = track(1, TRACK_TYPE_VIDEO, "V_MPEGH/ISO/HEVC", None);
        tracks.extend(track(2, TRACK_TYPE_SUBTITLE, "S_TEXT/UTF8", None));
        tracks.extend(track(3, TRACK_TYPE_SUBTITLE, "S_HDMV/PGS", Some("eng")));
        tracks.extend(track(4, TRACK_TYPE_SUBTITLE, "S_TEXT/UTF8", Some("tur")));
        let tracks = el(TRACKS, &tracks);
        let mut cues = Vec::new();
        cues.extend(cue_point(0, &[1, 2]));
        cues.extend(cue_point(1000, &[1]));
        cues.extend(cue_point(2000, &[1, 2]));
        let cues = el(CUES, &cues);
        let cluster = el(CLUSTER, &vec![0u8; padding]);

        // SeekHead has a fixed size, so positions can be computed up front.
        let seek_head_len = el(SEEK_HEAD, &[seek(TRACKS, 0), seek(CUES, 0)].concat()).len();
        let (tracks_pos, cues_pos) = if tracks_late {
            let tracks_pos = seek_head_len + cluster.len();
            (tracks_pos, tracks_pos + tracks.len())
        } else {
            (seek_head_len, seek_head_len + tracks.len() + cluster.len())
        };
        let seek_head = el(
            SEEK_HEAD,
            &[seek(TRACKS, tracks_pos as u64), seek(CUES, cues_pos as u64)].concat(),
        );
        let mut body = seek_head;
        if tracks_late {
            body.extend(cluster);
            body.extend(tracks);
        } else {
            body.extend(tracks);
            body.extend(cluster);
        }
        body.extend(cues);

        let mut file = el(EBML, &el(0x4282, b"matroska"));
        file.extend(el(SEGMENT, &body));
        file
    }

    struct MemFile {
        bytes: Vec<u8>,
        reads: Mutex<Vec<(u64, usize)>>,
    }

    impl MemFile {
        fn new(bytes: Vec<u8>) -> Self {
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

    #[tokio::test]
    async fn finds_tracks_in_head_and_cues_by_seek() {
        let bytes = sample_file(200_000, false);
        let file = MemFile::new(bytes.clone());
        let report = inspect(&file, &head(&bytes)).await.unwrap();
        assert_eq!(report.requests, 1, "only Cues needs a read");
        assert_eq!(report.video_tracks, 1);
        assert!(
            matches!(
                report.cues,
                CuesState::Read {
                    points: 3,
                    video_points: 3,
                    ..
                }
            ),
            "{:?}",
            report.cues
        );
        let subs: Vec<_> = report
            .subtitles
            .iter()
            .map(|s| (s.number, s.codec, s.lang.as_str(), s.english, s.cue_points))
            .collect();
        assert_eq!(
            subs,
            vec![
                (2, Codec::Srt, "eng", true, 2),
                (3, Codec::Pgs, "eng", true, 0),
                (4, Codec::Srt, "tur", false, 0),
            ]
        );
        assert_eq!(report.english_text_indexed, Some(true));
    }

    #[tokio::test]
    async fn reads_tracks_through_seek_when_not_in_head() {
        let bytes = sample_file(200_000, true);
        let file = MemFile::new(bytes.clone());
        let report = inspect(&file, &head(&bytes)).await.unwrap();
        assert_eq!(report.requests, 2);
        assert_eq!(report.subtitles.len(), 3);
        assert_eq!(report.english_text_indexed, Some(true));
    }

    #[tokio::test]
    async fn everything_in_head_needs_no_reads() {
        let bytes = sample_file(10, false);
        let file = MemFile::new(bytes.clone());
        let report = inspect(&file, &head(&bytes)).await.unwrap();
        assert_eq!(report.requests, 0);
        assert!(file.reads.lock().unwrap().is_empty());
        assert_eq!(report.english_text_indexed, Some(true));
    }

    #[tokio::test]
    async fn missing_cues_entry_is_not_referenced() {
        let mut tracks = track(1, TRACK_TYPE_VIDEO, "V_MPEG4/ISO/AVC", None);
        tracks.extend(track(2, TRACK_TYPE_SUBTITLE, "S_TEXT/ASS", Some("eng")));
        let mut file = el(EBML, &[]);
        file.extend(el(SEGMENT, &el(TRACKS, &tracks)));
        let mem = MemFile::new(file.clone());
        let report = inspect(&mem, &file).await.unwrap();
        assert_eq!(report.cues, CuesState::NotReferenced);
        assert_eq!(report.english_text_indexed, None);
        assert_eq!(report.subtitles[0].codec, Codec::Ass);
    }

    #[tokio::test]
    async fn oversized_cues_are_reported_not_read() {
        // Tracks right after the SeekHead, whose length does not depend on positions.
        let seek_len = el(SEEK_HEAD, &[seek(TRACKS, 0), seek(CUES, 0)].concat()).len();
        let tracks = el(TRACKS, &track(1, TRACK_TYPE_VIDEO, "V", None));
        let cues_pos = seek_len + tracks.len();
        let mut body = el(
            SEEK_HEAD,
            &[seek(TRACKS, seek_len as u64), seek(CUES, cues_pos as u64)].concat(),
        );
        body.extend(tracks);
        // Claimed Cues size far above the budget; only its header exists.
        let mut cues_header = vec![0x1C, 0x53, 0xBB, 0x6B, 0x01];
        cues_header.extend_from_slice(&(64u64 << 20).to_be_bytes()[1..]);
        body.extend(cues_header);
        let mut file = el(EBML, &[]);
        file.extend(el(SEGMENT, &body));
        let mem = MemFile::new(file.clone());
        let report = inspect(&mem, &file[..file.len() - 12]).await.unwrap();
        assert_eq!(report.cues, CuesState::TooLarge { bytes: 64 << 20 });
        assert_eq!(report.requests, 1);
    }

    #[tokio::test]
    async fn truncated_or_garbage_input_never_panics() {
        let bytes = sample_file(100, false);
        for cut in 0..bytes.len() {
            let file = MemFile::new(bytes[..cut].to_vec());
            let _ = inspect(&file, &bytes[..cut]).await;
        }
        let file = MemFile::new(Vec::new());
        assert!(inspect(&file, &[0xFF; 64]).await.is_err());
    }

    #[test]
    fn header_handles_unknown_size_and_long_ids() {
        let h = parse_header(&[
            0x18, 0x53, 0x80, 0x67, 0x01, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF,
        ])
        .unwrap();
        assert_eq!(h.id, SEGMENT);
        assert_eq!(h.size, None);
        assert_eq!(h.len, 12);
        let h = parse_header(&[0xAE, 0x85]).unwrap();
        assert_eq!((h.id, h.size, h.len), (0xAE, Some(5), 2));
        assert!(parse_header(&[0x00, 0x81]).is_none());
    }

    #[test]
    fn bcp47_language_wins_and_is_sanitized() {
        let mut data = uint_el(TRACK_NUMBER, 5);
        data.extend(uint_el(TRACK_TYPE, TRACK_TYPE_SUBTITLE));
        data.extend(el(CODEC_ID, b"S_TEXT/WEBVTT"));
        data.extend(el(LANGUAGE, b"und"));
        data.extend(el(LANGUAGE_BCP47, b"en-US\n<x>"));
        let list = parse_tracks(&el(TRACK_ENTRY, &data));
        let sub = &list.subtitles[0];
        assert!(sub.english);
        assert_eq!(sub.codec, Codec::WebVtt);
        assert_eq!(sub.lang, "en-USx");
    }
}
