//! VP9 WebM stickers, with alpha: the contract between the sticker core and the video track. The
//! core calls only what is declared here; everything under `src/video/` and `crates/sk-vpx-sys` is
//! the video track's.
//!
//! `VideoSource::open` demuxes (a block index and each block's BlockAddID 1 alpha packet) without
//! decoding. `VideoSource::decoder` makes one libvpx VP9 context per stream (colour, alpha).
//! `decode_next` decodes the next shown frame; `convert` writes it as STRAIGHT BGRA.
//!
//! Media Foundation is not an option: its only VP9 decoder is the removable "VP9 Video Extensions"
//! Store package, and it drops the alpha stream.
//!
//! # How the core drives it
//!
//! * One [`VideoSource`] per `src`, shared by `Arc`. The registry counts the file bytes; the source
//!   counts only its index ([`VideoSource::memory_bytes`]).
//! * One [`VideoDecoder`] per source and mode, never per place on screen. A decoded frame is
//!   converted once per tile size that shows it, so [`VideoDecoder::convert`] may be called several
//!   times between two [`VideoDecoder::decode_next`] calls.
//! * Frames are taken in order; a frame the clock has passed is decoded and not converted. Past the
//!   last frame the core calls [`VideoDecoder::rewind`].
//! * A small loop (<= 64 px) is decoded once into the loop cache and its decoder dropped.
//!
//! # Threading
//!
//! [`VideoSource`] is `Send + Sync` (immutable after `open`). [`VideoDecoder`] is `Send` and not
//! `Sync`: it is driven on the sticker worker threads, one call at a time. Nothing here runs on the
//! UI or JS thread, touches gpui, spawns a thread or blocks on a lock. libvpx runs with
//! `threads = 1`: the worker pool is the parallelism.
//!
//! # Pixels
//!
//! [`Target`] is BGRA with STRAIGHT alpha, rows packed (what `LiveTile::write_bgra` takes): gpui's
//! polychrome pipeline blends straight alpha, so premultiplied bytes come out dark.
//!
//! * Colour: the conversion of the stream's colour space and range that Chromium uses.
//! * Alpha: the alpha stream's Y plane as is, as Chromium reads it; 255 with no alpha stream.
//! * Scaling: the whole frame into the whole target; the core picks the source's aspect ratio. Down
//!   by area average with colour weighted by alpha (as `sk-resample`), so a transparent pixel's
//!   colour never bleeds into an edge. Up bilinear.
//! * [`DecodeOptions::alpha_only`] (a tinted sticker): only the alpha stream is decoded and
//!   B = G = R = 255; the tint replaces the colour and keeps the alpha.

use std::sync::Arc;

mod colour;
mod index;
mod vp9;
mod webm;

use index::Packet;
use sk_vpx_sys::{Decoder, ErrorCode, Format};

/// What a file is, known after [`VideoSource::open`] without decoding a frame.
#[derive(Clone, Debug, PartialEq)]
pub struct VideoMeta {
    /// Display size in pixels (`DisplayWidth` × `DisplayHeight`, else the pixel size).
    pub width: u32,
    pub height: u32,
    /// Each shown frame's presentation time in ms from the first frame, ascending; `len()` is the
    /// frame count. A lone hidden frame has no entry: it decodes with the next shown one.
    pub pts_ms: Arc<[u32]>,
    /// One loop in ms: `Info/Duration`, else the last pts plus the gap before it.
    pub duration_ms: u32,
    /// The track says `AlphaMode = 1` and its blocks carry alpha packets.
    pub has_alpha: bool,
}

impl VideoMeta {
    pub fn frame_count(&self) -> u32 {
        self.pts_ms.len() as u32
    }

    pub fn fps(&self) -> f32 {
        if self.duration_ms == 0 {
            return 0.0;
        }
        self.frame_count() as f32 * 1000.0 / self.duration_ms as f32
    }

    /// The frame shown `t_ms` after the loop started, looping.
    pub fn frame_at(&self, t_ms: u64) -> u32 {
        if self.pts_ms.is_empty() || self.duration_ms == 0 {
            return 0;
        }
        let t = (t_ms % self.duration_ms as u64) as u32;
        (self.pts_ms.partition_point(|&p| p <= t) as u32).saturating_sub(1)
    }
}

/// How a decoder decodes. Fixed for its life: a stream skipped for one frame cannot be resumed
/// (its reference frames are missing), so a change is a new decoder.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct DecodeOptions {
    /// Decode only the alpha stream; `convert` writes B = G = R = 255 and the alpha (255 if none).
    pub alpha_only: bool,
}

/// Where [`VideoDecoder::convert`] writes: `width` × `height` BGRA pixels,
/// STRAIGHT alpha, rows packed, so `pixels.len() == width * height * 4`.
pub struct Target<'a> {
    pub pixels: &'a mut [u8],
    pub width: u32,
    pub height: u32,
}

/// Why a file does not play. The string is the advisory shown in devtools and with the element's
/// `error` event: say what is wrong in words a page author can act on.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum VideoError {
    /// Valid, but not played here: no video track, a codec other than VP9, a profile other than 0,
    /// a frame larger than 4096 px.
    Unsupported(String),
    /// Not a WebM file, a truncated one, or a VP9 bitstream libvpx rejects.
    Malformed(String),
    /// The decoder is not in this build.
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

/// One demuxed `.webm`: its [`VideoMeta`] and packet index. Immutable after `open`.
pub struct VideoSource {
    meta: VideoMeta,
    bytes: Arc<[u8]>,
    packets: Box<[Packet]>,
    colour: Option<webm::Colour>,
}

impl VideoSource {
    /// Demuxes `bytes`, the whole file, without decoding a frame. Cheap enough for a worker on a
    /// cache miss.
    pub fn open(bytes: Arc<[u8]>) -> Result<Arc<VideoSource>, VideoError> {
        let index = index::index(&bytes)?;
        Ok(Arc::new(VideoSource { meta: index.meta, bytes, packets: index.packets, colour: index.colour }))
    }

    pub fn meta(&self) -> &VideoMeta {
        &self.meta
    }

    /// Bytes held beyond the file itself: the block index.
    pub fn memory_bytes(&self) -> usize {
        std::mem::size_of::<Self>() + self.meta.pts_ms.len() * 4 + self.packets.len() * std::mem::size_of::<Packet>()
    }

    /// A decoder at frame 0. [`VideoError::Malformed`] when libvpx will not start.
    pub fn decoder(self: &Arc<Self>, options: DecodeOptions) -> Result<VideoDecoder, VideoError> {
        let start = || Decoder::new().map_err(|e| VideoError::Malformed(format!("the VP9 decoder would not start: {e}")));
        // A tinted decoder needs no colour; a file without alpha needs no alpha decoder.
        let color = if options.alpha_only { None } else { Some(start()?) };
        let alpha = if self.meta.has_alpha { Some(start()?) } else { None };
        Ok(VideoDecoder {
            source: self.clone(),
            options,
            position: 0,
            next: 0,
            color,
            alpha,
            current: false,
            alpha_now: false,
            failed: false,
            converter: sk_yuv::Converter::new(),
            _not_sync: std::marker::PhantomData,
        })
    }
}

/// A decoding position in one [`VideoSource`]. `Send`, not `Sync`.
pub struct VideoDecoder {
    source: Arc<VideoSource>,
    options: DecodeOptions,
    position: u32,
    next: usize,
    color: Option<Decoder>,
    alpha: Option<Decoder>,
    /// A frame was decoded since the decoder was made or rewound.
    current: bool,
    /// The current frame has an alpha frame; else it is opaque, as in Chromium.
    alpha_now: bool,
    /// A decode failed: only a rewind goes on.
    failed: bool,
    converter: sk_yuv::Converter,
    _not_sync: std::marker::PhantomData<std::cell::Cell<()>>,
}

fn yuv_plane<'a>(p: &sk_vpx_sys::Plane<'a>) -> sk_yuv::Plane<'a> {
    sk_yuv::Plane { data: p.data, stride: p.stride }
}

fn decode_error(what: String, e: sk_vpx_sys::Error) -> VideoError {
    match e.code {
        ErrorCode::Unsupported => VideoError::Unsupported(format!("{what}: {e}")),
        _ => VideoError::Malformed(format!("{what}: {e}")),
    }
}

impl VideoDecoder {
    pub fn source(&self) -> &Arc<VideoSource> {
        &self.source
    }

    pub fn options(&self) -> DecodeOptions {
        self.options
    }

    /// The index of the frame the next `decode_next` decodes (`frame_count()` past the last).
    pub fn position(&self) -> u32 {
        self.position
    }

    /// Decodes the next shown frame into the current frame. `Ok(None)` is past the last frame: call
    /// `rewind`. After an error the decoder is only good for `rewind` or dropping.
    pub fn decode_next(&mut self) -> Result<Option<u32>, VideoError> {
        if self.failed {
            return Err(VideoError::Malformed("an earlier frame failed to decode; the decoder needs a rewind".into()));
        }
        let source = self.source.clone();
        while let Some(packet) = source.packets.get(self.next) {
            self.next += 1;
            let frame = self.position;
            // Every packet goes to both streams, shown or not: a hidden
            // frame is a reference the next ones need.
            let color_shown = match self.color.as_mut().map(|c| c.decode(packet.color.of(&source.bytes))) {
                Some(Err(e)) => {
                    self.failed = true;
                    return Err(decode_error(format!("video frame {frame}"), e));
                }
                shown => shown.is_some_and(|s| s.unwrap_or(false)),
            };
            let alpha_shown = match (self.alpha.as_mut(), packet.alpha) {
                (Some(alpha), Some(span)) => match alpha.decode(span.of(&source.bytes)) {
                    Ok(shown) => shown,
                    Err(e) => {
                        self.failed = true;
                        return Err(decode_error(format!("the alpha of video frame {frame}"), e));
                    }
                },
                _ => false,
            };
            if !packet.shows {
                continue;
            }
            // The index and libvpx agree on what shows, or the file lies.
            if self.color.is_some() && !color_shown {
                self.failed = true;
                return Err(VideoError::Malformed(format!("video frame {frame} decoded to no picture")));
            }
            if self.alpha.is_some() && packet.alpha.is_some() && !alpha_shown {
                self.failed = true;
                return Err(VideoError::Malformed(format!("the alpha of video frame {frame} decoded to no picture")));
            }
            self.alpha_now = alpha_shown;
            self.current = true;
            self.position += 1;
            return Ok(Some(frame));
        }
        Ok(None)
    }

    /// Writes the current frame into `out`, scaled to its size. [`VideoError::Malformed`] when no
    /// frame has been decoded since the decoder was made or rewound.
    pub fn convert(&mut self, out: &mut Target<'_>) -> Result<(), VideoError> {
        debug_assert_eq!(
            out.pixels.len(),
            out.width as usize * out.height as usize * 4
        );
        if !self.current || self.failed {
            return Err(VideoError::Malformed("no video frame is decoded to convert".into()));
        }
        let plane = yuv_plane;
        let alpha = if self.alpha_now { self.alpha.as_ref().and_then(Decoder::frame) } else { None };
        let result = if self.options.alpha_only {
            let size = alpha.map_or((0, 0), |a| (a.width, a.height));
            self.converter.alpha_to_bgra(alpha.map(|a| plane(&a.planes[0])), size, out.pixels, out.width, out.height)
        } else {
            let frame = self
                .color
                .as_ref()
                .and_then(Decoder::frame)
                .ok_or_else(|| VideoError::Malformed("no video frame is decoded to convert".into()))?;
            if frame.format != Format::I420 {
                return Err(VideoError::Unsupported(format!("its frames are {:?}; video stickers are 4:2:0", frame.format)));
            }
            if let Some(a) = alpha {
                if (a.width, a.height) != (frame.width, frame.height) {
                    return Err(VideoError::Malformed(format!(
                        "its alpha frames are {}×{} and its colour frames {}×{}",
                        a.width, a.height, frame.width, frame.height
                    )));
                }
            }
            let colorimetry = colour::colorimetry(self.source.colour.as_ref(), frame.color_space, frame.full_range, self.source.meta.has_alpha);
            let i420 = sk_yuv::I420 {
                width: frame.width,
                height: frame.height,
                y: plane(&frame.planes[0]),
                u: plane(&frame.planes[1]),
                v: plane(&frame.planes[2]),
                a: alpha.map(|a| plane(&a.planes[0])),
            };
            self.converter.to_bgra(&i420, colorimetry, out.pixels, out.width, out.height)
        };
        result.map_err(|e| VideoError::Malformed(format!("converting a video frame: {e}")))
    }

    /// Back to frame 0; nothing is re-demuxed.
    pub fn rewind(&mut self) -> Result<(), VideoError> {
        // The contexts stay: the first packet is a keyframe (`open` checks),
        // which resets a VP9 decoder, and after an error sk-vpx-sys starts
        // the next decode from a fresh context.
        self.position = 0;
        self.next = 0;
        self.current = false;
        self.alpha_now = false;
        self.failed = false;
        Ok(())
    }

    /// Bytes this decoder holds (libvpx frame buffers and conversion scratch): what the budget
    /// charges a live decoder.
    pub fn memory_bytes(&self) -> usize {
        std::mem::size_of::<Self>()
            + [&self.color, &self.alpha].iter().filter_map(|d| d.as_ref()).map(|d| d.memory().live).sum::<usize>()
            + self.converter.memory_bytes()
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
    fn a_file_opens_and_its_frames_decode() {
        let bytes: Arc<[u8]> = Arc::from(&b"\x1a\x45\xdf\xa3"[..]);
        assert!(matches!(VideoSource::open(bytes), Err(VideoError::Malformed(_))));
        let source = VideoSource::open(Arc::from(&include_bytes!("testdata/alpha.webm")[..])).unwrap();
        assert_eq!(source.meta().frame_count(), 48);
        assert!(source.memory_bytes() > 48 * 4);
        let mut decoder = source.decoder(DecodeOptions::default()).unwrap();
        assert_eq!(decoder.decode_next(), Ok(Some(0)));
    }

    #[test]
    fn a_source_is_shared_across_workers_and_a_decoder_moves_between_them() {
        fn send_sync<T: Send + Sync>() {}
        fn send<T: Send>() {}
        send_sync::<VideoSource>();
        send::<VideoDecoder>();
    }
}

#[cfg(test)]
mod decode_tests;
