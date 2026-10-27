//! M6 block probe (SS-028): can the ruler track's text be read line by line
//! from the block positions in Cues, without reading the rest of the file?
//!
//! Off unless `SUBSYNC_BLOCK_PROBE=1`; runs in the background after the K2
//! probe. Payloads are checked for size and UTF-8 only: subtitle text is
//! never kept, logged or returned.

use std::sync::Arc;
use std::time::Instant;

use tokio::sync::Semaphore;
use tokio::task::JoinSet;

use crate::mkv::{children, parse_header, vint_len};
use crate::range::{ByteSource, MultiRange};

const BLOCK_GROUP: u32 = 0xA0;
const BLOCK: u32 = 0xA1;
const SIMPLE_BLOCK: u32 = 0xA3;

/// Blocks read per file at most.
pub const MAX_BLOCKS: usize = 4000;
/// Range requests in flight at once.
pub const CONCURRENCY: usize = 16;
/// Bytes read per block; a larger block costs one more request.
const WINDOW: usize = 1024;
/// A subtitle block larger than this is taken for a false match.
const MAX_ELEMENT: usize = 64 * 1024;
/// A cluster header is a 4-byte ID and a 1 to 8 byte size, so the block
/// starts up to 7 bytes after its earliest possible position.
const CLUSTER_ID_LEN: u64 = 4;
const MAX_EXTRA_SIZE_BYTES: usize = 7;
/// Ranges in the single multi-range request, and bytes per range.
const MULTI_RANGES: usize = 3;
const MULTI_RANGE_LEN: u64 = 64;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct BlockPos {
    /// Absolute file offset of the cluster element.
    pub cluster: u64,
    /// `CueRelativePosition`: counted from the start of the cluster's data.
    pub relative: u64,
}

impl BlockPos {
    /// Where the block would start if the cluster's size field were 1 byte.
    fn earliest(self) -> u64 {
        self.cluster
            .saturating_add(CLUSTER_ID_LEN + 1)
            .saturating_add(self.relative)
    }
}

/// The chosen ruler track's block positions. `Debug` prints counts only.
#[derive(Clone, PartialEq, Eq)]
pub struct BlockTargets {
    pub track: u64,
    pub encoded: bool,
    pub cue_refs: usize,
    pub with_cluster: usize,
    pub with_relative: usize,
    pub positions: Vec<BlockPos>,
}

impl std::fmt::Debug for BlockTargets {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BlockTargets")
            .field("track", &self.track)
            .field("encoded", &self.encoded)
            .field("cue_refs", &self.cue_refs)
            .field("with_cluster", &self.with_cluster)
            .field("with_relative", &self.with_relative)
            .field("positions", &self.positions.len())
            .finish()
    }
}

/// Counts from one block probe. Safe to log: no text, no positions.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BlockReport {
    pub track: u64,
    pub encoded: bool,
    pub cue_refs: usize,
    pub with_cluster: usize,
    pub with_relative: usize,
    /// Distinct block positions; at most `MAX_BLOCKS` of them are read.
    pub targets: usize,
    pub attempted: usize,
    /// A block of the track was found where Cues pointed.
    pub found: usize,
    /// The read worked but no block of the track was there.
    pub missing: usize,
    pub failed_reads: usize,
    /// Found blocks larger than the window, finished with a second read.
    pub extended: usize,
    pub laced: usize,
    pub utf8: usize,
    pub empty: usize,
    pub payload_bytes: usize,
    /// Single-range requests for blocks; the multi-range check is extra.
    pub requests: usize,
    pub bytes_read: usize,
    pub elapsed_ms: u128,
    pub multi_range: MultiRange,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
struct Found {
    extended: bool,
    laced: bool,
    utf8: bool,
    payload: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Outcome {
    Found(Found),
    Missing,
    Failed,
}

/// Value of an EBML variable-size integer with its length marker removed.
fn vint_value(buf: &[u8]) -> Option<(u64, usize)> {
    let len = vint_len(*buf.first()?)?;
    let bytes = buf.get(..len)?;
    let mask = if len == 8 { 0 } else { 0xFFu8 >> len };
    let value = bytes[1..]
        .iter()
        .fold(u64::from(bytes[0] & mask), |acc, &b| {
            (acc << 8) | u64::from(b)
        });
    Some((value, len))
}

/// Track number of a Block or SimpleBlock body, which may be cut short.
fn block_track(body: &[u8]) -> Option<u64> {
    vint_value(body).map(|(track, _)| track)
}

/// Track number of the Block inside a BlockGroup body, which may be cut
/// short; earlier complete children are stepped over.
fn group_track(body: &[u8]) -> Option<u64> {
    let mut pos = 0;
    while let Some(header) = body.get(pos..).and_then(parse_header) {
        if header.id == BLOCK {
            return block_track(&body[pos + header.len..]);
        }
        pos = pos
            .checked_add(header.len)?
            .checked_add(usize::try_from(header.size?).ok()?)?;
    }
    None
}

/// Payload of a complete Block body: after track, timecode and flags.
fn block_payload(body: &[u8]) -> Option<(&[u8], bool)> {
    let (_, track_len) = vint_value(body)?;
    let flags = *body.get(track_len + 2)?;
    let laced = flags & 0x06 != 0;
    Some((body.get(track_len + 3..)?, laced))
}

async fn read_one(file: &dyn ByteSource, track: u64, pos: BlockPos) -> (Outcome, usize, usize) {
    let start = pos.earliest();
    let Ok(buf) = file.read(start, MAX_EXTRA_SIZE_BYTES + WINDOW).await else {
        return (Outcome::Failed, 1, 0);
    };
    let mut requests = 1;
    let mut bytes = buf.len();
    for skip in 0..=MAX_EXTRA_SIZE_BYTES.min(buf.len().saturating_sub(1)) {
        let Some(header) = parse_header(&buf[skip..]) else {
            continue;
        };
        let Some(size) = header.size.and_then(|s| usize::try_from(s).ok()) else {
            continue;
        };
        if size > MAX_ELEMENT || !matches!(header.id, BLOCK_GROUP | SIMPLE_BLOCK) {
            continue;
        }
        let body = &buf[(skip + header.len).min(buf.len())..];
        let seen = match header.id {
            SIMPLE_BLOCK => block_track(body),
            _ => group_track(body),
        };
        if seen != Some(track) {
            continue;
        }
        let mut body = body[..size.min(body.len())].to_vec();
        let extended = body.len() < size;
        if extended {
            let more_at = start + (skip + header.len + body.len()) as u64;
            let Ok(more) = file.read(more_at, size - body.len()).await else {
                return (Outcome::Failed, requests + 1, bytes);
            };
            requests += 1;
            bytes += more.len();
            body.extend_from_slice(&more);
        }
        let block = match header.id {
            SIMPLE_BLOCK => Some(&body[..]),
            _ => children(&body)
                .into_iter()
                .find(|(id, _)| *id == BLOCK)
                .map(|(_, data)| data),
        };
        let Some((payload, laced)) = block.and_then(block_payload) else {
            return (Outcome::Missing, requests, bytes);
        };
        let found = Found {
            extended,
            laced,
            utf8: std::str::from_utf8(payload).is_ok(),
            payload: payload.len(),
        };
        return (Outcome::Found(found), requests, bytes);
    }
    (Outcome::Missing, requests, bytes)
}

/// Reads every target block (up to `MAX_BLOCKS`, `CONCURRENCY` at a time)
/// and asks once whether the server serves several ranges per request.
pub async fn run(file: Arc<dyn ByteSource>, targets: BlockTargets) -> BlockReport {
    let started = Instant::now();
    let multi: Vec<(u64, u64)> = targets
        .positions
        .iter()
        .take(MULTI_RANGES)
        .map(|p| (p.earliest(), p.earliest() + MULTI_RANGE_LEN - 1))
        .collect();
    let multi_range = if multi.len() < 2 {
        MultiRange::NotTried
    } else {
        file.multi_range(multi).await
    };

    let permits = Arc::new(Semaphore::new(CONCURRENCY));
    let mut tasks = JoinSet::new();
    let attempted = targets.positions.len().min(MAX_BLOCKS);
    for &pos in targets.positions.iter().take(MAX_BLOCKS) {
        let file = Arc::clone(&file);
        let permits = Arc::clone(&permits);
        let track = targets.track;
        tasks.spawn(async move {
            let _permit = permits.acquire_owned().await.ok();
            read_one(file.as_ref(), track, pos).await
        });
    }

    let mut report = BlockReport {
        track: targets.track,
        encoded: targets.encoded,
        cue_refs: targets.cue_refs,
        with_cluster: targets.with_cluster,
        with_relative: targets.with_relative,
        targets: targets.positions.len(),
        attempted,
        found: 0,
        missing: 0,
        failed_reads: 0,
        extended: 0,
        laced: 0,
        utf8: 0,
        empty: 0,
        payload_bytes: 0,
        requests: 0,
        bytes_read: 0,
        elapsed_ms: 0,
        multi_range,
    };
    while let Some(joined) = tasks.join_next().await {
        let Ok((outcome, requests, bytes)) = joined else {
            report.failed_reads += 1;
            continue;
        };
        report.requests += requests;
        report.bytes_read += bytes;
        match outcome {
            Outcome::Found(found) => {
                report.found += 1;
                report.extended += usize::from(found.extended);
                report.laced += usize::from(found.laced);
                report.utf8 += usize::from(found.utf8);
                report.empty += usize::from(found.payload == 0);
                report.payload_bytes += found.payload;
            }
            Outcome::Missing => report.missing += 1,
            Outcome::Failed => report.failed_reads += 1,
        }
    }
    report.elapsed_ms = started.elapsed().as_millis();
    report
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::range::ReadFuture;
    use std::sync::Mutex;

    const CLUSTER: u32 = 0x1F43_B675;
    const CLUSTER_TIMECODE: u32 = 0xE7;
    const BLOCK_DURATION: u32 = 0x9B;

    /// Element with a minimal ID encoding and a size field of `size_len` bytes.
    fn el_sized(id: u32, data: &[u8], size_len: usize) -> Vec<u8> {
        let id_bytes = id.to_be_bytes();
        let skip = id_bytes.iter().take_while(|&&b| b == 0).count();
        let mut out = id_bytes[skip..].to_vec();
        let marker = 1u64 << (7 * size_len);
        let size = (marker | data.len() as u64).to_be_bytes();
        out.extend_from_slice(&size[8 - size_len..]);
        out.extend_from_slice(data);
        out
    }

    fn el(id: u32, data: &[u8]) -> Vec<u8> {
        el_sized(id, data, 8)
    }

    fn block_body(track: u8, flags: u8, text: &[u8]) -> Vec<u8> {
        let mut body = vec![0x80 | track, 0, 0, flags];
        body.extend_from_slice(text);
        body
    }

    fn group(track: u8, text: &[u8]) -> Vec<u8> {
        let mut data = el(BLOCK, &block_body(track, 0, text));
        data.extend(el(BLOCK_DURATION, &[0x07, 0xD0]));
        el(BLOCK_GROUP, &data)
    }

    struct MemFile {
        bytes: Vec<u8>,
        reads: Mutex<usize>,
    }

    impl ByteSource for MemFile {
        fn read(&self, start: u64, len: usize) -> ReadFuture<'_> {
            *self.reads.lock().unwrap() += 1;
            let start = (start as usize).min(self.bytes.len());
            let end = start.saturating_add(len).min(self.bytes.len());
            let out = self.bytes[start..end].to_vec();
            Box::pin(async move { Ok(out) })
        }
    }

    /// Padding, then one cluster per element in `blocks`, each after a
    /// cluster timecode. Returns the file and each block's position.
    fn file_with(blocks: &[Vec<u8>], size_len: usize) -> (Vec<u8>, Vec<BlockPos>) {
        let mut file = vec![0u8; 300];
        let mut positions = Vec::new();
        for block in blocks {
            let mut data = el(CLUSTER_TIMECODE, &[0x10]);
            let relative = data.len() as u64;
            data.extend_from_slice(block);
            positions.push(BlockPos {
                cluster: file.len() as u64,
                relative,
            });
            file.extend(el_sized(CLUSTER, &data, size_len));
            file.extend(vec![0u8; 500]);
        }
        (file, positions)
    }

    fn targets(track: u64, positions: Vec<BlockPos>) -> BlockTargets {
        BlockTargets {
            track,
            encoded: false,
            cue_refs: positions.len(),
            with_cluster: positions.len(),
            with_relative: positions.len(),
            positions,
        }
    }

    async fn probe(bytes: Vec<u8>, targets: BlockTargets) -> BlockReport {
        let file = Arc::new(MemFile {
            bytes,
            reads: Mutex::new(0),
        });
        run(file, targets).await
    }

    #[tokio::test]
    async fn reads_block_groups_and_simple_blocks_at_cue_positions() {
        for size_len in [1, 4, 8] {
            let blocks = [
                group(2, b"Hello there."),
                el(SIMPLE_BLOCK, &block_body(2, 0x80, b"General Kenobi!")),
            ];
            let (bytes, positions) = file_with(&blocks, size_len);
            let report = probe(bytes, targets(2, positions)).await;
            assert_eq!(report.found, 2, "size_len {size_len}");
            assert_eq!(report.utf8, 2);
            assert_eq!(report.payload_bytes, 12 + 15);
            assert_eq!(report.requests, 2);
            assert_eq!(report.missing + report.failed_reads + report.extended, 0);
            assert_eq!(report.multi_range, MultiRange::NotTried);
        }
    }

    #[tokio::test]
    async fn another_track_at_the_position_is_missing() {
        let (bytes, positions) = file_with(&[group(3, b"Bonjour")], 8);
        let report = probe(bytes, targets(2, positions)).await;
        assert_eq!((report.found, report.missing), (0, 1));
    }

    #[tokio::test]
    async fn large_block_costs_a_second_read() {
        let text = vec![b'a'; 3000];
        let (bytes, positions) = file_with(&[group(2, &text)], 8);
        let report = probe(bytes, targets(2, positions)).await;
        assert_eq!(report.found, 1);
        assert_eq!(report.extended, 1);
        assert_eq!(report.requests, 2);
        assert_eq!(report.payload_bytes, 3000);
    }

    #[tokio::test]
    async fn compressed_payload_is_not_utf8_and_lacing_is_counted() {
        let blocks = [
            group(2, &[0x78, 0x9C, 0xFF, 0xFE]),
            el(SIMPLE_BLOCK, &block_body(2, 0x02, b"x")),
        ];
        let (bytes, positions) = file_with(&blocks, 8);
        let report = probe(bytes, targets(2, positions)).await;
        assert_eq!(report.found, 2);
        assert_eq!(report.utf8, 1);
        assert_eq!(report.laced, 1);
    }

    #[tokio::test]
    async fn positions_past_the_end_are_missing_not_failed() {
        let report = probe(
            vec![0u8; 100],
            targets(
                2,
                vec![BlockPos {
                    cluster: 5_000,
                    relative: 3,
                }],
            ),
        )
        .await;
        assert_eq!((report.missing, report.failed_reads), (1, 0));
    }

    #[test]
    fn targets_debug_prints_counts_not_positions() {
        let shown = format!(
            "{:?}",
            targets(
                2,
                vec![BlockPos {
                    cluster: 987_654_321,
                    relative: 1,
                }]
            )
        );
        assert!(shown.contains("positions: 1"), "{shown}");
        assert!(!shown.contains("987654321"), "{shown}");
    }
}
