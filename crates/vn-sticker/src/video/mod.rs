//! VP9 WebM stickers, with alpha: the contract between the sticker core and the
//! video track.
//!
//! The core (the rest of this crate) calls only what is declared here. The
//! video track owns everything under `src/video/` and `crates/vn-vpx-sys`, and
//! replaces the bodies marked `W3` without changing a signature. A signature
//! change is agreed with the core first.
//!
//! # Pipeline
//!
//! ```text
//! Arc<[u8]> (the whole .webm)
//!   └─ VideoSource::open      EBML/WebM demux: the VP9 track, a block index, and
//!                             each block's BlockAdditional (BlockAddID 1) alpha
//!                             packet. No frame is decoded.
//!        └─ VideoSource::decoder(DecodeOptions)
//!                             one libvpx VP9 context per stream (colour, alpha),
//!                             `threads = 1`
//!             ├─ decode_next  the next shown frame, in stream order
//!             └─ convert      that frame, I420 + A to STRAIGHT BGRA, scaled
//!                             to the caller's size
//! ```
//!
//! Media Foundation is not an option: its only VP9 decoder is the removable
//! "VP9 Video Extensions" Store package, and it drops the alpha stream (W0,
//! `research/reports/m26/stickers-1.md`).
//!
//! # How the core drives it
//!
//! * One [`VideoSource`] per `src`, shared through `Arc` by every decoder made
//!   from it. The registry holds the file bytes and counts them; the source
//!   counts only its index ([`VideoSource::memory_bytes`]).
//! * One [`VideoDecoder`] per source and mode, never per place on screen. A
//!   decoded frame is converted once per tile size that shows it, so
//!   [`VideoDecoder::convert`] may be called several times, at several sizes,
//!   between two [`VideoDecoder::decode_next`] calls.
//! * Frames are taken in order. A frame the clock has already passed is
//!   decoded and not converted. Past the last frame the core calls
//!   [`VideoDecoder::rewind`] and starts again, at most once per loop.
//! * A small loop (≤ 64 px) is decoded once into the core's compressed loop
//!   cache, and its decoder is dropped.
//! * At most 4 decoders of a 512 px source are alive at once (owner's budget).
//!   Dropping a decoder frees everything it holds.
//!
//! # Threading
//!
//! [`VideoSource`] is `Send + Sync`: immutable after `open`. [`VideoDecoder`]
//! is `Send` and not `Sync`: it is created, driven and dropped on the sticker
//! worker threads, one call at a time, and may move between workers between
//! calls. Nothing here runs on the UI or JS thread, touches gpui, spawns a
//! thread or blocks on a lock another decoder holds. libvpx runs with
//! `threads = 1`: the worker pool is the parallelism. Every call is CPU work
//! that returns. W0 measured, single-threaded, libvpx with intrinsics and no
//! nasm: 0.445 ms a 512² frame with alpha, 0.078 ms a 100² frame.
//!
//! # Pixels
//!
//! [`Target`] is BGRA with STRAIGHT alpha, rows packed: the bytes
//! `LiveTile::write_bgra` takes. gpui's polychrome pipeline blends straight
//! alpha, so premultiplied bytes come out dark wherever they are translucent.
//!
//! * Colour: the conversion of the stream's colour space and range (VP9
//!   `color_space`, `color_range`) that Chromium uses for the same file.
//!   Verify against Chromium rather than assume.
//! * Alpha: the alpha stream's Y plane, as is, the way Chromium reads it. With
//!   no alpha stream, 255.
//! * Scaling: the whole frame into the whole target, no letterboxing. The core
//!   picks a target with the source's aspect ratio. Down by area average,
//!   colour weighted by alpha, so a transparent pixel's colour never bleeds
//!   into an edge, as `vn-resample` does. Up bilinear.
//! * [`DecodeOptions::alpha_only`] (a tinted sticker): only the alpha stream
//!   is decoded, and B = G = R = 255. The tint replaces the colour and keeps
//!   the alpha (`rgb = mix(s.rgb, tint.rgb, tint.a)`, `a = s.a`), so the
//!   colour stream is never needed.

use std::sync::Arc;

mod index;
mod vp9;
mod webm;

use index::Packet;

/// What a file is, known after [`VideoSource::open`] without decoding a frame.
#[derive(Clone, Debug, PartialEq)]
pub struct VideoMeta {
    /// Display size in pixels: `DisplayWidth` × `DisplayHeight` when the track
    /// has them, else `PixelWidth` × `PixelHeight`.
    pub width: u32,
    pub height: u32,
    /// Each shown frame's presentation time in ms from the first frame
    /// (`pts_ms[0] == 0`), ascending. `len()` is the frame count. A block whose
    /// packet shows no frame (a lone hidden frame) has no entry: it is decoded
    /// as part of the next [`VideoDecoder::decode_next`].
    pub pts_ms: Arc<[u32]>,
    /// One loop, in ms: `Info/Duration` when the file has one, else the last
    /// pts plus the gap before it. Greater than the last pts.
    pub duration_ms: u32,
    /// The track says `AlphaMode = 1` and its blocks carry alpha packets.
    pub has_alpha: bool,
}

impl VideoMeta {
    /// Shown frames in one loop.
    pub fn frame_count(&self) -> u32 {
        self.pts_ms.len() as u32
    }

    /// The nominal frame rate: frames per second over one loop.
    pub fn fps(&self) -> f32 {
        if self.duration_ms == 0 {
            return 0.0;
        }
        self.frame_count() as f32 * 1000.0 / self.duration_ms as f32
    }

    /// The frame shown `t_ms` after the loop started, looping: the last frame
    /// whose pts is at or before `t_ms % duration_ms`.
    pub fn frame_at(&self, t_ms: u64) -> u32 {
        if self.pts_ms.is_empty() || self.duration_ms == 0 {
            return 0;
        }
        let t = (t_ms % self.duration_ms as u64) as u32;
        (self.pts_ms.partition_point(|&p| p <= t) as u32).saturating_sub(1)
    }
}

/// How a decoder decodes. Fixed for its life: a stream skipped for one frame
/// cannot be picked up later (its reference frames are missing), so a change
/// is a new decoder.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct DecodeOptions {
    /// Decode only the alpha stream: [`VideoDecoder::convert`] writes
    /// B = G = R = 255 and the alpha (255 when the file has none). For a
    /// tinted sticker, whose colour the tint replaces.
    pub alpha_only: bool,
}

/// Where [`VideoDecoder::convert`] writes: `width` × `height` BGRA pixels,
/// STRAIGHT alpha, rows packed, so `pixels.len() == width * height * 4`.
pub struct Target<'a> {
    pub pixels: &'a mut [u8],
    pub width: u32,
    pub height: u32,
}

/// Why a file does not play. The string is the advisory the core shows in
/// devtools and with the element's `error` event: name the file's problem in
/// words a page author can act on.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum VideoError {
    /// Valid, but not something this decoder plays: no video track, a codec
    /// other than VP9 (`V_VP8`, `V_AV1`), a VP9 profile other than 0, a frame
    /// larger than 4096 px.
    Unsupported(String),
    /// Not a WebM file, a truncated one, or a VP9 bitstream libvpx rejects.
    Malformed(String),
    /// The decoder is not in this build (the stub, until W3 lands).
    NotBuilt,
}

impl std::fmt::Display for VideoError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            VideoError::Unsupported(why) => write!(f, "unsupported video sticker: {why}"),
            VideoError::Malformed(why) => write!(f, "malformed video sticker: {why}"),
            VideoError::NotBuilt => f.write_str("video stickers are not in this build"),
        }
    }
}

impl std::error::Error for VideoError {}

/// One demuxed `.webm`: its [`VideoMeta`] and the index of its packets.
/// Immutable after [`VideoSource::open`].
pub struct VideoSource {
    meta: VideoMeta,
    #[allow(dead_code)] // W3: the decoder reads it (next step)
    bytes: Arc<[u8]>,
    /// Every block of the video track, in file order: where its colour and
    /// alpha packets are in `bytes`, and whether it shows a frame.
    packets: Box<[Packet]>,
    /// The track's `Colour`, which picks the YUV matrix when it is complete.
    #[allow(dead_code)] // W3: the conversion reads it (next step)
    colour: Option<webm::Colour>,
}

impl VideoSource {
    /// Demuxes `bytes`, the whole file. Reads the EBML header, the first VP9
    /// track and every block, and decodes no frame. Cheap enough for the
    /// worker to call on a cache miss: a sticker is at most a few hundred KB.
    pub fn open(bytes: Arc<[u8]>) -> Result<Arc<VideoSource>, VideoError> {
        let index = index::index(&bytes)?;
        Ok(Arc::new(VideoSource { meta: index.meta, bytes, packets: index.packets, colour: index.colour }))
    }

    pub fn meta(&self) -> &VideoMeta {
        &self.meta
    }

    /// Bytes held by this source beyond the file itself (the registry counts
    /// the file): the block index.
    pub fn memory_bytes(&self) -> usize {
        std::mem::size_of::<Self>() + self.meta.pts_ms.len() * 4 + self.packets.len() * std::mem::size_of::<Packet>()
    }

    /// A decoder at frame 0. [`VideoError::Malformed`] when libvpx will not
    /// start.
    pub fn decoder(self: &Arc<Self>, options: DecodeOptions) -> Result<VideoDecoder, VideoError> {
        Ok(VideoDecoder {
            source: self.clone(),
            options,
            position: 0,
            _not_sync: std::marker::PhantomData,
        })
    }
}

/// A decoding position in one [`VideoSource`]: the libvpx contexts (colour
/// unless [`DecodeOptions::alpha_only`], alpha when the file has it) and the
/// current frame. `Send`, not `Sync` (module docs, Threading).
pub struct VideoDecoder {
    source: Arc<VideoSource>,
    options: DecodeOptions,
    /// The frame the next [`VideoDecoder::decode_next`] decodes.
    position: u32,
    // W3: the libvpx contexts and any conversion scratch. Raw libvpx pointers
    // make the type neither Send nor Sync: give it back `unsafe impl Send`
    // (a context is used by one thread at a time) and keep this marker.
    _not_sync: std::marker::PhantomData<std::cell::Cell<()>>,
}

impl VideoDecoder {
    pub fn source(&self) -> &Arc<VideoSource> {
        &self.source
    }

    pub fn options(&self) -> DecodeOptions {
        self.options
    }

    /// The index of the frame the next [`VideoDecoder::decode_next`] decodes:
    /// 0 when new or rewound, `frame_count()` past the last.
    pub fn position(&self) -> u32 {
        self.position
    }

    /// Decodes the next shown frame, which becomes the current frame for
    /// [`VideoDecoder::convert`]. `Ok(Some(index))` is that frame's index;
    /// `Ok(None)` means past the last frame, with nothing decoded: call
    /// [`VideoDecoder::rewind`]. After an error the decoder is only good for
    /// [`VideoDecoder::rewind`] or dropping.
    pub fn decode_next(&mut self) -> Result<Option<u32>, VideoError> {
        Err(VideoError::NotBuilt) // W3
    }

    /// Writes the current frame into `out`, scaled to its size (module docs,
    /// Pixels). Callable any number of times per decoded frame, at any sizes.
    /// [`VideoError::Malformed`] when no frame has been decoded since the
    /// decoder was made or rewound.
    pub fn convert(&mut self, out: &mut Target<'_>) -> Result<(), VideoError> {
        debug_assert_eq!(
            out.pixels.len(),
            out.width as usize * out.height as usize * 4
        );
        Err(VideoError::NotBuilt) // W3
    }

    /// Back to frame 0. The next [`VideoDecoder::decode_next`] decodes the
    /// first keyframe again; nothing is re-demuxed.
    pub fn rewind(&mut self) -> Result<(), VideoError> {
        self.position = 0;
        Ok(()) // W3: reset the libvpx contexts
    }

    /// Bytes this decoder holds: libvpx's frame buffers for each stream and
    /// any conversion scratch. What the budget charges a live decoder; cheap
    /// to call. W0 measured 5.3 MB for a 512² stream with alpha, 0.63 MB at
    /// 100².
    pub fn memory_bytes(&self) -> usize {
        std::mem::size_of::<Self>() // W3
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn meta(pts: &[u32], duration_ms: u32) -> VideoMeta {
        VideoMeta {
            width: 512,
            height: 512,
            pts_ms: pts.into(),
            duration_ms,
            has_alpha: true,
        }
    }

    #[test]
    fn the_clock_picks_the_last_frame_at_or_before_it_and_loops() {
        let m = meta(&[0, 33, 67, 100], 133);
        assert_eq!(m.frame_count(), 4);
        assert_eq!(m.frame_at(0), 0);
        assert_eq!(m.frame_at(32), 0);
        assert_eq!(m.frame_at(33), 1);
        assert_eq!(m.frame_at(132), 3);
        assert_eq!(m.frame_at(133), 0, "a loop later");
        assert_eq!(m.frame_at(133 * 10 + 70), 2);
        assert!((m.fps() - 30.075).abs() < 0.01);
    }

    #[test]
    fn an_empty_or_zero_length_stream_stays_on_frame_zero() {
        assert_eq!(meta(&[], 0).frame_at(500), 0);
        assert_eq!(meta(&[0], 0).frame_at(500), 0);
        assert_eq!(meta(&[0], 0).fps(), 0.0);
    }

    #[test]
    fn a_file_is_demuxed_and_the_decoder_is_not_built_yet() {
        let bytes: Arc<[u8]> = Arc::from(&b"\x1a\x45\xdf\xa3"[..]);
        assert!(matches!(VideoSource::open(bytes), Err(VideoError::Malformed(_))));
        let source = VideoSource::open(Arc::from(&include_bytes!("testdata/alpha.webm")[..])).unwrap();
        assert_eq!(source.meta().frame_count(), 48);
        assert!(source.memory_bytes() > 48 * 4);
        let mut decoder = source.decoder(DecodeOptions::default()).unwrap();
        assert_eq!(decoder.decode_next(), Err(VideoError::NotBuilt));
    }

    #[test]
    fn a_source_is_shared_across_workers_and_a_decoder_moves_between_them() {
        fn send_sync<T: Send + Sync>() {}
        fn send<T: Send>() {}
        send_sync::<VideoSource>();
        send::<VideoDecoder>();
    }
}
