//! What draws a source's frames: a Lottie [`vn_lottie::Animation`] or a VP9
//! [`VideoSource`] with its decoders, behind one [`Engine`] trait so the core
//! schedules both alike (and tests can stand in a fake).

use std::sync::Arc;

use crate::clock::Timeline;
use crate::video::{DecodeOptions, Target as VideoTarget, VideoDecoder, VideoSource};

/// What a file is, by its first bytes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Kind {
    /// Lottie JSON, or a `.tgs` (gzipped Lottie JSON).
    Lottie,
    /// A WebM (VP9, maybe with alpha).
    Video,
}

impl Kind {
    pub fn sniff(bytes: &[u8]) -> Option<Kind> {
        if bytes.starts_with(&[0x1a, 0x45, 0xdf, 0xa3]) {
            return Some(Kind::Video);
        }
        if bytes.starts_with(&[0x1f, 0x8b]) {
            return Some(Kind::Lottie);
        }
        let text = bytes.strip_prefix(b"\xef\xbb\xbf").unwrap_or(bytes);
        (text.iter().find(|b| !b.is_ascii_whitespace()) == Some(&b'{')).then_some(Kind::Lottie)
    }

    pub fn name(self) -> &'static str {
        match self {
            Kind::Lottie => "lottie",
            Kind::Video => "video",
        }
    }
}

/// A loaded source. Lives on the worker threads (`Send`), one call at a time.
pub trait Engine: Send {
    fn kind(&self) -> Kind;
    /// Intrinsic size in pixels.
    fn size(&self) -> (u32, u32);
    fn timeline(&self) -> Timeline;
    /// Source frame `frame` filling `width × height`: BGRA, straight alpha,
    /// rows packed. `alpha_only`: B = G = R = 255 with the frame's alpha
    /// (a tinted video, whose colour stream is then never decoded).
    fn draw(&mut self, frame: u32, alpha_only: bool, width: u32, height: u32) -> Result<Vec<u8>, String>;
    /// Bytes held, for the stats.
    fn estimated_bytes(&self) -> usize;
}

/// Makes an engine from a file's bytes. The core's is [`load`]; tests pass
/// their own.
pub type Loader = fn(&Arc<[u8]>) -> Result<Box<dyn Engine>, String>;

/// The real loader: sniffs the bytes and loads a Lottie or opens a WebM.
pub fn load(bytes: &Arc<[u8]>) -> Result<Box<dyn Engine>, String> {
    match Kind::sniff(bytes) {
        Some(Kind::Lottie) => vn_lottie::Animation::load(bytes)
            .map(|animation| Box::new(LottieEngine { animation }) as Box<dyn Engine>)
            .map_err(|error| error.to_string()),
        Some(Kind::Video) => VideoSource::open(bytes.clone())
            .map(|source| Box::new(VideoEngine { source, color: None, alpha: None }) as Box<dyn Engine>)
            .map_err(|error| error.to_string()),
        None => Err("neither a Lottie (.json, .tgs) nor a WebM file".into()),
    }
}

struct LottieEngine {
    animation: vn_lottie::Animation,
}

impl Engine for LottieEngine {
    fn kind(&self) -> Kind {
        Kind::Lottie
    }

    fn size(&self) -> (u32, u32) {
        let meta = self.animation.meta();
        (meta.width.max(1), meta.height.max(1))
    }

    fn timeline(&self) -> Timeline {
        Timeline::Uniform { frames: self.animation.frame_count(), fps: self.animation.meta().frame_rate }
    }

    // The tint applies at paint time to a Lottie's own colours: no alpha-only mode.
    fn draw(&mut self, frame: u32, _alpha_only: bool, width: u32, height: u32) -> Result<Vec<u8>, String> {
        self.animation
            .render(frame, width, height)
            .map(<[u8]>::to_vec)
            .ok_or_else(|| format!("the {} renderer could not draw frame {frame}", self.animation.backend()))
    }

    fn estimated_bytes(&self) -> usize {
        self.animation.estimated_bytes()
    }
}

/// A decoder and the frame it holds.
struct Cursor {
    decoder: VideoDecoder,
    current: Option<u32>,
}

struct VideoEngine {
    source: Arc<VideoSource>,
    color: Option<Cursor>,
    alpha: Option<Cursor>,
}

impl VideoEngine {
    fn cursor(&mut self, alpha_only: bool) -> Result<&mut Cursor, String> {
        let slot = if alpha_only { &mut self.alpha } else { &mut self.color };
        if slot.is_none() {
            let decoder = self.source.decoder(DecodeOptions { alpha_only }).map_err(|e| e.to_string())?;
            *slot = Some(Cursor { decoder, current: None });
        }
        Ok(slot.as_mut().expect("just made"))
    }
}

impl Engine for VideoEngine {
    fn kind(&self) -> Kind {
        Kind::Video
    }

    fn size(&self) -> (u32, u32) {
        let meta = self.source.meta();
        (meta.width.max(1), meta.height.max(1))
    }

    fn timeline(&self) -> Timeline {
        let meta = self.source.meta();
        Timeline::Pts { pts_ms: meta.pts_ms.clone(), duration_ms: meta.duration_ms }
    }

    /// Decodes forward to `frame`. A frame just behind the one held (a 30 fps
    /// target next to a 60 fps one) is drawn as the one held rather than
    /// rewinding to the keyframe; an earlier one rewinds.
    fn draw(&mut self, frame: u32, alpha_only: bool, width: u32, height: u32) -> Result<Vec<u8>, String> {
        let c = self.cursor(alpha_only)?;
        if c.current.is_some_and(|held| frame < held && held - frame > 2) {
            c.decoder.rewind().map_err(|e| e.to_string())?;
            c.current = None;
        }
        while c.current.is_none_or(|held| held < frame) {
            match c.decoder.decode_next().map_err(|e| e.to_string())? {
                Some(index) => c.current = Some(index),
                None if c.current.is_some() => break, // past the end: the last frame
                None => return Err("the stream has no frames".into()),
            }
        }
        let mut pixels = vec![0u8; width as usize * height as usize * 4];
        c.decoder
            .convert(&mut VideoTarget { pixels: &mut pixels, width, height })
            .map_err(|e| e.to_string())?;
        Ok(pixels)
    }

    fn estimated_bytes(&self) -> usize {
        self.source.memory_bytes()
            + [&self.color, &self.alpha].iter().filter_map(|c| c.as_ref()).map(|c| c.decoder.memory_bytes()).sum::<usize>()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn files_are_told_apart_by_their_first_bytes() {
        assert_eq!(Kind::sniff(b"\x1a\x45\xdf\xa3\x01"), Some(Kind::Video));
        assert_eq!(Kind::sniff(b"\x1f\x8b\x08"), Some(Kind::Lottie));
        assert_eq!(Kind::sniff(b"  \n{\"v\":\"5\"}"), Some(Kind::Lottie));
        assert_eq!(Kind::sniff(b"\xef\xbb\xbf{}"), Some(Kind::Lottie), "a BOM");
        assert_eq!(Kind::sniff(b"\x89PNG"), None);
        assert_eq!(Kind::sniff(b""), None);
    }

    #[test]
    fn a_tgs_loads_and_draws_straight_bytes() {
        let bytes: Arc<[u8]> =
            std::fs::read(concat!(env!("CARGO_MANIFEST_DIR"), "/../vn-lottie/tests/fixtures/spinning-star.tgs"))
                .unwrap()
                .into();
        let mut engine = load(&bytes).unwrap();
        assert_eq!(engine.kind(), Kind::Lottie);
        assert_eq!(engine.size(), (512, 512));
        assert_eq!(engine.timeline(), Timeline::Uniform { frames: 120, fps: 60.0 });
        let px = engine.draw(3, false, 48, 48).unwrap();
        assert_eq!(px.len(), 48 * 48 * 4);
        assert!(px.chunks_exact(4).any(|p| p[3] == 255), "something opaque");
        assert!(engine.estimated_bytes() > 48 * 48 * 4);
    }

    #[test]
    fn a_video_waits_for_the_decoder_and_garbage_is_named() {
        let webm: Arc<[u8]> = Arc::from(&b"\x1a\x45\xdf\xa3\x9f"[..]);
        let error = load(&webm).err().unwrap();
        assert!(error.contains("not in this build"), "{error}");
        let png: Arc<[u8]> = Arc::from(&b"\x89PNG\r\n"[..]);
        assert!(load(&png).err().unwrap().contains("neither"));
    }
}
