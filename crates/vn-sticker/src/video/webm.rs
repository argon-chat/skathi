//! A minimal WebM (Matroska) demuxer: the first VP9 video track of a file, its
//! blocks in file order, and each block's alpha packet.
//!
//! What it reads, by element:
//!
//! * the EBML header's `DocType` (`webm`, or `matroska`);
//! * the first `Segment`, of known or unknown size (a live writer's, such as
//!   `MediaRecorder`'s);
//! * `Info`: `TimestampScale`, `Duration`;
//! * `Tracks`: the first video `TrackEntry` whose `CodecID` is `V_VP9`, with
//!   its `DefaultDuration`, and from `Video` the pixel and display sizes,
//!   `AlphaMode` and the `Colour` coefficients a player chooses its YUV matrix
//!   from;
//! * every `Cluster`, of known or unknown size: its `Timestamp`, and that
//!   track's `SimpleBlock`s and `BlockGroup`s (`Block`, `BlockDuration`, and
//!   the `BlockAdditional` with `BlockAddID` 1 in `BlockAdditions`: the alpha
//!   stream's packet, Matroska's "alpha channel" addition).
//!
//! Everything else (SeekHead, Cues, Tags, Chapters, Void, other tracks) is
//! skipped by its size. Laced video blocks are refused: no WebM writer laces
//! video (libwebm, ffmpeg and Chromium's MediaRecorder all write a frame a
//! block), so there is nothing to test a lacing reader against.
//!
//! Damaged input: nothing here indexes past a slice or allocates by a size
//! the file declares. The only allocation is the block list, one entry per
//! block found and at most [`MAX_BLOCKS`]. Masters are read by one function
//! each, at a fixed depth, so a file cannot nest its way into a deep stack.
//! A file cut short inside what a sticker needs (the headers, a Cluster) is
//! [`VideoError::Malformed`]; one cut in trailing metadata after its blocks
//! (Cues, Tags) plays.

use super::VideoError;

const EBML: u32 = 0x1A45_DFA3;
const DOC_TYPE: u32 = 0x4282;
const SEGMENT: u32 = 0x1853_8067;
const INFO: u32 = 0x1549_A966;
const TIMESTAMP_SCALE: u32 = 0x2A_D7B1;
const DURATION: u32 = 0x4489;
const TRACKS: u32 = 0x1654_AE6B;
const TRACK_ENTRY: u32 = 0xAE;
const TRACK_NUMBER: u32 = 0xD7;
const TRACK_TYPE: u32 = 0x83;
const CODEC_ID: u32 = 0x86;
const DEFAULT_DURATION: u32 = 0x23_E383;
const CONTENT_ENCODINGS: u32 = 0x6D80;
const VIDEO: u32 = 0xE0;
const PIXEL_WIDTH: u32 = 0xB0;
const PIXEL_HEIGHT: u32 = 0xBA;
const DISPLAY_WIDTH: u32 = 0x54B0;
const DISPLAY_HEIGHT: u32 = 0x54BA;
const DISPLAY_UNIT: u32 = 0x54B2;
const ALPHA_MODE: u32 = 0x53C0;
const COLOUR: u32 = 0x55B0;
const MATRIX_COEFFICIENTS: u32 = 0x55B1;
const RANGE: u32 = 0x55B9;
const TRANSFER_CHARACTERISTICS: u32 = 0x55BA;
const PRIMARIES: u32 = 0x55BB;
const CLUSTER: u32 = 0x1F43_B675;
const CLUSTER_TIMESTAMP: u32 = 0xE7;
const SIMPLE_BLOCK: u32 = 0xA3;
const BLOCK_GROUP: u32 = 0xA0;
const BLOCK: u32 = 0xA1;
const BLOCK_DURATION: u32 = 0x9B;
const BLOCK_ADDITIONS: u32 = 0x75A1;
const BLOCK_MORE: u32 = 0xA6;
const BLOCK_ADD_ID: u32 = 0xEE;
const BLOCK_ADDITIONAL: u32 = 0xA5;

/// What an unknown-size Cluster may hold: Timestamp, SilentTracks, Position,
/// PrevSize, SimpleBlock, BlockGroup, EncryptedBlock, Void, CRC-32. Any other
/// ID is the element after the Cluster.
const CLUSTER_CHILDREN: [u32; 9] = [CLUSTER_TIMESTAMP, 0x5854, 0xA7, 0xAB, SIMPLE_BLOCK, BLOCK_GROUP, 0xAF, 0xEC, 0xBF];

/// Blocks a file may have: 18 minutes at 60 fps, far past a sticker's 3 s.
pub(crate) const MAX_BLOCKS: usize = 1 << 16;

/// A byte range of the file.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Span {
    pub start: u32,
    pub len: u32,
}

impl Span {
    /// `start..end` of a file under 4 GiB ([`demux`] checks).
    fn new(start: usize, end: usize) -> Span {
        Span { start: start as u32, len: (end - start) as u32 }
    }

    pub fn of<'a>(&self, bytes: &'a [u8]) -> &'a [u8] {
        &bytes[self.start as usize..self.start as usize + self.len as usize]
    }
}

/// The track's `Colour`: H.273 code points, `None` where the file has none.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Colour {
    pub matrix: Option<u64>,
    pub transfer: Option<u64>,
    pub primaries: Option<u64>,
    /// 0 unspecified, 1 broadcast (studio), 2 full, 3 defined by the others.
    pub range: Option<u64>,
}

/// The video track.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Track {
    pub number: u64,
    /// `PixelWidth` × `PixelHeight`, when the file has them.
    pub pixel: Option<(u64, u64)>,
    /// `DisplayWidth` × `DisplayHeight` (0 where absent) and `DisplayUnit`.
    pub display: (u64, u64),
    pub display_unit: u64,
    /// `AlphaMode` is 1.
    pub alpha_mode: bool,
    pub colour: Option<Colour>,
    /// `DefaultDuration`, in ns.
    pub default_duration: Option<u64>,
}

/// One block of the video track.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Block {
    /// Cluster timestamp plus the block's, in `TimestampScale` units.
    pub time: i64,
    /// The VP9 packet.
    pub data: Span,
    /// The alpha stream's VP9 packet (`BlockAddID` 1), when not empty.
    pub alpha: Option<Span>,
    /// `BlockDuration`, in `TimestampScale` units.
    pub duration: Option<u64>,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Webm {
    /// ns per timestamp unit.
    pub timestamp_scale: u64,
    /// `Info/Duration`, in timestamp units.
    pub duration: Option<f64>,
    pub track: Track,
    pub blocks: Vec<Block>,
}

fn malformed(why: impl Into<String>) -> VideoError {
    VideoError::Malformed(why.into())
}

fn unsupported(why: impl Into<String>) -> VideoError {
    VideoError::Unsupported(why.into())
}

/// An element's header: its ID, where its data starts and its data size
/// (`None`: unknown, all value bits set).
#[derive(Clone, Copy, Debug)]
struct Element {
    id: u32,
    data: usize,
    size: Option<u64>,
}

impl Element {
    /// Where its data ends, when that is at or before `end`.
    fn end_within(&self, end: usize) -> Option<usize> {
        let size = usize::try_from(self.size?).ok()?;
        self.data.checked_add(size).filter(|&e| e <= end)
    }
}

/// The element header at `pos`, read from `bytes[..end]`. `None` when those
/// bytes do not hold a whole, valid one.
fn element(bytes: &[u8], pos: usize, end: usize) -> Option<Element> {
    let b = bytes.get(..end)?;
    let first = *b.get(pos)?;
    let id_len = first.leading_zeros() as usize + 1;
    if id_len > 4 {
        return None;
    }
    let id = b.get(pos..pos + id_len)?.iter().fold(0u32, |v, &x| v << 8 | x as u32);
    let (size, size_len) = vint(b, pos + id_len)?;
    Some(Element { id, data: pos + id_len + size_len, size })
}

/// The EBML variable-size integer at `pos`: its value (`None` when every value
/// bit is set, which a size uses for "unknown") and its length in bytes.
fn vint(b: &[u8], pos: usize) -> Option<(Option<u64>, usize)> {
    let first = *b.get(pos)?;
    let len = first.leading_zeros() as usize + 1;
    if len > 8 {
        return None;
    }
    let bytes = b.get(pos..pos + len)?;
    let mut v = (first & (0xFFu16 >> len) as u8) as u64;
    for &x in &bytes[1..] {
        v = v << 8 | x as u64;
    }
    let all_ones = v == (1u64 << (7 * len)) - 1;
    Some(((!all_ones).then_some(v), len))
}

fn uint(data: &[u8], what: &str) -> Result<u64, VideoError> {
    if data.len() > 8 {
        return Err(malformed(format!("its {what} is not a number")));
    }
    Ok(data.iter().fold(0u64, |v, &x| v << 8 | x as u64))
}

fn float(data: &[u8], what: &str) -> Result<f64, VideoError> {
    match data.len() {
        0 => Ok(0.0),
        4 => Ok(f32::from_be_bytes(data.try_into().expect("4 bytes")) as f64),
        8 => Ok(f64::from_be_bytes(data.try_into().expect("8 bytes"))),
        _ => Err(malformed(format!("its {what} is not a number"))),
    }
}

/// Why an element at `pos` does not fit in `parent_end`: the file ends, or
/// the element overruns its parent.
fn overrun(bytes: &[u8], parent_end: usize, pos: usize, inside: &str) -> VideoError {
    if parent_end >= bytes.len() {
        malformed(format!("the file is cut short inside {inside} (byte {pos} of {})", bytes.len()))
    } else {
        malformed(format!("an element in {inside} runs past its end (byte {pos})"))
    }
}

/// The children of a master whose data is `start..end`, in order, each whole.
fn children(
    bytes: &[u8],
    start: usize,
    end: usize,
    inside: &str,
    mut each: impl FnMut(u32, usize, usize) -> Result<(), VideoError>,
) -> Result<(), VideoError> {
    let mut pos = start;
    while pos < end {
        let e = element(bytes, pos, end).ok_or_else(|| overrun(bytes, end, pos, inside))?;
        if e.size.is_none() {
            return Err(malformed(format!("an element of unknown size inside {inside} (byte {pos})")));
        }
        let data_end = e.end_within(end).ok_or_else(|| overrun(bytes, end, pos, inside))?;
        each(e.id, e.data, data_end)?;
        pos = data_end;
    }
    Ok(())
}

/// The bytes of a string element, without the NUL padding EBML allows.
fn string(data: &[u8]) -> &[u8] {
    let end = data.iter().position(|&b| b == 0).unwrap_or(data.len());
    &data[..end]
}

/// Demuxes a whole file.
pub(crate) fn demux(bytes: &[u8]) -> Result<Webm, VideoError> {
    if u32::try_from(bytes.len()).is_err() {
        return Err(unsupported("the file is over 4 GiB"));
    }
    let len = bytes.len();
    let head = element(bytes, 0, len)
        .filter(|e| e.id == EBML)
        .ok_or_else(|| malformed("not a WebM file: it does not start with an EBML header"))?;
    let head_end = head.end_within(len).ok_or_else(|| malformed("not a WebM file: its EBML header is cut short or damaged"))?;
    let mut doc_type: &[u8] = b"matroska"; // EBML's default
    children(bytes, head.data, head_end, "the EBML header", |id, s, e| {
        if id == DOC_TYPE {
            doc_type = string(&bytes[s..e]);
        }
        Ok(())
    })?;
    if doc_type != b"webm" && doc_type != b"matroska" {
        return Err(malformed(format!("not a WebM file: its DocType is \"{}\"", String::from_utf8_lossy(doc_type))));
    }

    // The first Segment, past anything else at the top level (Void).
    let mut pos = head_end;
    let segment = loop {
        let e = element(bytes, pos, len).ok_or_else(|| malformed("not a WebM file: no Segment after its EBML header"))?;
        if e.id == SEGMENT {
            break e;
        }
        pos = e.end_within(len).ok_or_else(|| malformed("not a WebM file: no Segment after its EBML header"))?;
    };
    // A Segment that claims more than the file has was cut short: read what
    // is there, and judge the cut where it falls.
    let segment_end = segment.end_within(len).unwrap_or(len);
    let cut = segment.size.is_some() && segment.end_within(len).is_none();

    let mut parser = Parser { bytes, timestamp_scale: 1_000_000, duration: None, track: None, other_video: None, tracks_seen: false, blocks: Vec::new() };
    parser.segment(segment.data, segment_end, cut)?;
    let Parser { timestamp_scale, duration, track, other_video, blocks, .. } = parser;
    let Some(track) = track else {
        return Err(match other_video {
            Some(codec) => unsupported(codec_advice(&codec)),
            None => unsupported("the file has no video track"),
        });
    };
    if blocks.is_empty() {
        return Err(malformed("the video track has no frames"));
    }
    Ok(Webm { timestamp_scale, duration, track, blocks })
}

fn codec_advice(codec: &[u8]) -> String {
    let name = match codec {
        b"V_VP8" => "VP8".to_string(),
        b"V_AV1" => "AV1".to_string(),
        other => format!("\"{}\"", String::from_utf8_lossy(other)),
    };
    format!("its video is {name}; video stickers are VP9 (V_VP9)")
}

struct Parser<'a> {
    bytes: &'a [u8],
    timestamp_scale: u64,
    duration: Option<f64>,
    track: Option<Track>,
    /// The CodecID of the first video track that is not VP9, for the advisory.
    other_video: Option<Vec<u8>>,
    tracks_seen: bool,
    blocks: Vec<Block>,
}

impl Parser<'_> {
    /// The Segment's children in `start..end`. `cut`: the Segment claims
    /// more than the file has, which plays only when the cut falls in the
    /// metadata after the blocks (Cues, Tags), not where a Cluster could be.
    fn segment(&mut self, start: usize, end: usize, cut: bool) -> Result<(), VideoError> {
        let mut pos = start;
        // The last element read was metadata after the blocks.
        let mut past_blocks = false;
        while pos < end {
            let Some(e) = element(self.bytes, pos, end) else {
                if self.trailing_metadata(pos, end) {
                    return Ok(());
                }
                return Err(overrun(self.bytes, end, pos, "the Segment"));
            };
            if e.id == CLUSTER {
                if !self.tracks_seen {
                    return Err(malformed("a Cluster comes before the Tracks"));
                }
                pos = self.cluster(e, end)?;
                past_blocks = false;
                continue;
            }
            if e.size.is_none() {
                return Err(malformed(format!("an element of unknown size in the Segment (byte {pos})")));
            }
            let Some(data_end) = e.end_within(end) else {
                if self.trailing_metadata(pos, end) {
                    return Ok(());
                }
                return Err(overrun(self.bytes, end, pos, "the Segment"));
            };
            match e.id {
                INFO => self.info(e.data, data_end)?,
                TRACKS if !self.tracks_seen => {
                    self.tracks(e.data, data_end)?;
                    self.tracks_seen = true;
                }
                _ => {}
            }
            past_blocks = !self.blocks.is_empty();
            pos = data_end;
        }
        if cut && !past_blocks {
            return Err(malformed(format!("the file is cut short (it ends at byte {end}, its Segment claims more)")));
        }
        Ok(())
    }

    /// The element at `pos` runs past the end of the file, and it is
    /// metadata after the blocks: what is left of its ID is not (the start
    /// of) one a sticker needs (Cluster, Info, Tracks).
    fn trailing_metadata(&self, pos: usize, end: usize) -> bool {
        if self.blocks.is_empty() || end < self.bytes.len() || pos >= end {
            return false;
        }
        let id = &self.bytes[pos..end.min(pos + 4)];
        ![CLUSTER, INFO, TRACKS].iter().any(|needed| needed.to_be_bytes().starts_with(id))
    }

    fn info(&mut self, start: usize, end: usize) -> Result<(), VideoError> {
        let bytes = self.bytes;
        children(bytes, start, end, "the Info", |id, s, e| {
            match id {
                TIMESTAMP_SCALE => {
                    self.timestamp_scale = uint(&bytes[s..e], "TimestampScale")?;
                    if self.timestamp_scale == 0 {
                        return Err(malformed("its TimestampScale is 0"));
                    }
                }
                DURATION => {
                    let d = float(&bytes[s..e], "Duration")?;
                    self.duration = (d.is_finite() && d > 0.0).then_some(d);
                }
                _ => {}
            }
            Ok(())
        })
    }

    fn tracks(&mut self, start: usize, end: usize) -> Result<(), VideoError> {
        let bytes = self.bytes;
        children(bytes, start, end, "the Tracks", |id, s, e| {
            if id != TRACK_ENTRY {
                return Ok(());
            }
            let entry = track_entry(bytes, s, e)?;
            let video = entry.kind == Some(1) || (entry.kind.is_none() && entry.codec.starts_with(b"V_"));
            if !video || entry.track.number == 0 || self.track.is_some() {
                return Ok(());
            }
            if entry.codec != b"V_VP9" {
                self.other_video.get_or_insert_with(|| entry.codec.to_vec());
                return Ok(());
            }
            if entry.encoded {
                return Err(unsupported("its video track is encrypted or compressed (ContentEncodings)"));
            }
            self.track = Some(entry.track);
            Ok(())
        })
    }

    /// Reads a Cluster's blocks; returns where the element after it starts.
    fn cluster(&mut self, e: Element, segment_end: usize) -> Result<usize, VideoError> {
        let known = e.size.is_some();
        let end = if known { e.end_within(segment_end).ok_or_else(|| overrun(self.bytes, segment_end, e.data, "a Cluster"))? } else { segment_end };
        let bytes = self.bytes;
        let mut time = 0i64;
        let mut pos = e.data;
        while pos < end {
            let Some(c) = element(bytes, pos, end) else {
                return Err(overrun(bytes, end, pos, "a Cluster"));
            };
            if !known && !CLUSTER_CHILDREN.contains(&c.id) {
                return Ok(pos);
            }
            if c.size.is_none() {
                return Err(malformed(format!("an element of unknown size in a Cluster (byte {pos})")));
            }
            let data_end = c.end_within(end).ok_or_else(|| overrun(bytes, end, pos, "a Cluster"))?;
            match c.id {
                CLUSTER_TIMESTAMP => {
                    time = i64::try_from(uint(&bytes[c.data..data_end], "Cluster Timestamp")?)
                        .map_err(|_| malformed("a Cluster Timestamp is out of range"))?;
                }
                SIMPLE_BLOCK => {
                    if let Some((rel, payload)) = self.block_header(c.data, data_end)? {
                        self.push(time, rel, payload, data_end, None, None)?;
                    }
                }
                BLOCK_GROUP => self.block_group(c.data, data_end, time)?,
                _ => {}
            }
            pos = data_end;
        }
        Ok(end)
    }

    fn block_group(&mut self, start: usize, end: usize, cluster_time: i64) -> Result<(), VideoError> {
        let bytes = self.bytes;
        let mut block = None;
        let mut alpha = None;
        let mut duration = None;
        children(bytes, start, end, "a BlockGroup", |id, s, e| {
            match id {
                BLOCK => block = Some((s, e)),
                BLOCK_DURATION => duration = Some(uint(&bytes[s..e], "BlockDuration")?),
                BLOCK_ADDITIONS => children(bytes, s, e, "a BlockAdditions", |id, s, e| {
                    if id != BLOCK_MORE {
                        return Ok(());
                    }
                    let mut add_id = 1; // the default
                    let mut data = None;
                    children(bytes, s, e, "a BlockMore", |id, s, e| {
                        match id {
                            BLOCK_ADD_ID => add_id = uint(&bytes[s..e], "BlockAddID")?,
                            BLOCK_ADDITIONAL => data = Some((s, e)),
                            _ => {}
                        }
                        Ok(())
                    })?;
                    if add_id == 1 && alpha.is_none() {
                        alpha = data;
                    }
                    Ok(())
                })?,
                _ => {}
            }
            Ok(())
        })?;
        let Some((s, e)) = block else {
            return Ok(()); // no Block: nothing to show
        };
        if let Some((rel, payload)) = self.block_header(s, e)? {
            self.push(cluster_time, rel, payload, e, alpha, duration)?;
        }
        Ok(())
    }

    /// A Block's or SimpleBlock's header, `start..end` being its data:
    /// `Some((relative timestamp, where the frame starts))` for the video
    /// track's blocks, `None` for another track's.
    fn block_header(&self, start: usize, end: usize) -> Result<Option<(i16, usize)>, VideoError> {
        let b = &self.bytes[..end];
        let (track, n) = vint(b, start).ok_or_else(|| malformed(format!("a damaged block (byte {start})")))?;
        let number = self.track.as_ref().map_or(0, |t| t.number);
        if track != Some(number) {
            return Ok(None);
        }
        let head = b.get(start + n..start + n + 3).ok_or_else(|| malformed(format!("a damaged block (byte {start})")))?;
        if head[2] & 0x06 != 0 {
            return Err(unsupported("its video blocks are laced (several frames to a block), which WebM video does not use"));
        }
        Ok(Some((i16::from_be_bytes([head[0], head[1]]), start + n + 3)))
    }

    fn push(
        &mut self,
        cluster_time: i64,
        rel: i16,
        start: usize,
        end: usize,
        alpha: Option<(usize, usize)>,
        duration: Option<u64>,
    ) -> Result<(), VideoError> {
        if start >= end {
            return Err(malformed(format!("an empty video block (byte {start})")));
        }
        if self.blocks.len() >= MAX_BLOCKS {
            return Err(unsupported(format!("it has more than {MAX_BLOCKS} frames")));
        }
        self.blocks.push(Block {
            time: cluster_time.saturating_add(rel as i64),
            data: Span::new(start, end),
            alpha: alpha.filter(|(s, e)| e > s).map(|(s, e)| Span::new(s, e)),
            duration,
        });
        Ok(())
    }
}

struct Entry<'a> {
    track: Track,
    kind: Option<u64>,
    codec: &'a [u8],
    encoded: bool,
}

fn track_entry(bytes: &[u8], start: usize, end: usize) -> Result<Entry<'_>, VideoError> {
    let mut entry = Entry { track: Track::default(), kind: None, codec: b"", encoded: false };
    let (mut pw, mut ph) = (None, None);
    children(bytes, start, end, "a TrackEntry", |id, s, e| {
        let data = &bytes[s..e];
        match id {
            TRACK_NUMBER => entry.track.number = uint(data, "TrackNumber")?,
            TRACK_TYPE => entry.kind = Some(uint(data, "TrackType")?),
            CODEC_ID => entry.codec = string(data),
            DEFAULT_DURATION => entry.track.default_duration = Some(uint(data, "DefaultDuration")?).filter(|&d| d > 0),
            CONTENT_ENCODINGS => entry.encoded = true,
            VIDEO => children(bytes, s, e, "a Video", |id, s, e| {
                let data = &bytes[s..e];
                match id {
                    PIXEL_WIDTH => pw = Some(uint(data, "PixelWidth")?),
                    PIXEL_HEIGHT => ph = Some(uint(data, "PixelHeight")?),
                    DISPLAY_WIDTH => entry.track.display.0 = uint(data, "DisplayWidth")?,
                    DISPLAY_HEIGHT => entry.track.display.1 = uint(data, "DisplayHeight")?,
                    DISPLAY_UNIT => entry.track.display_unit = uint(data, "DisplayUnit")?,
                    ALPHA_MODE => entry.track.alpha_mode = uint(data, "AlphaMode")? == 1,
                    COLOUR => {
                        let mut c = Colour::default();
                        children(bytes, s, e, "a Colour", |id, s, e| {
                            let data = &bytes[s..e];
                            match id {
                                MATRIX_COEFFICIENTS => c.matrix = Some(uint(data, "MatrixCoefficients")?),
                                TRANSFER_CHARACTERISTICS => c.transfer = Some(uint(data, "TransferCharacteristics")?),
                                PRIMARIES => c.primaries = Some(uint(data, "Primaries")?),
                                RANGE => c.range = Some(uint(data, "Range")?),
                                _ => {}
                            }
                            Ok(())
                        })?;
                        entry.track.colour = Some(c);
                    }
                    _ => {}
                }
                Ok(())
            })?,
            _ => {}
        }
        Ok(())
    })?;
    if let (Some(w), Some(h)) = (pw, ph) {
        entry.track.pixel = Some((w, h)).filter(|&(w, h)| w > 0 && h > 0);
    }
    Ok(entry)
}

#[cfg(test)]
#[path = "webm_tests.rs"]
pub(crate) mod tests;
