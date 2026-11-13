//! The persistent playback state behind one `<lottie>` node: the prepared animation, the transport
//! and the rendered-frame cache. Free of `Window` so transport and events are testable without one;
//! the only gpui type that leaks in is [`RenderImage`].

use std::sync::Arc;
use std::time::Duration;

use gpui::RenderImage;
use image::{Frame, RgbaImage};
use serde_json::Value;

use crate::backend::{self, Backend, Rasteriser};
use crate::transport::Transport;
use crate::normalize::{self, Normalization};

#[derive(Debug)]
pub enum LottieError {
    Decompress(std::io::Error),
    Json(serde_json::Error),
    Parse(String),
    /// Uses features the renderer cannot draw at all; [`LottieError::advisories`] has the detail.
    Unsupported(Vec<String>),
}

impl LottieError {
    pub fn advisories(&self) -> &[String] {
        match self {
            Self::Unsupported(issues) => issues,
            _ => &[],
        }
    }
}

impl std::fmt::Display for LottieError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Decompress(error) => write!(f, "gzip decode failed: {error}"),
            Self::Json(error) => write!(f, "not valid JSON: {error}"),
            Self::Parse(detail) => write!(f, "not a valid Lottie animation: {detail}"),
            Self::Unsupported(issues) => {
                write!(f, "animation uses {} unsupported feature(s)", issues.len())?;
                if let Some(first) = issues.first() {
                    write!(f, "; first: {first}")?;
                }
                Ok(())
            }
        }
    }
}

impl std::error::Error for LottieError {}

#[derive(Debug, Clone, PartialEq)]
pub struct LottieMeta {
    pub version: String,
    pub frame_rate: f32,
    pub in_point: f32,
    pub out_point: f32,
    pub total_frames: f32,
    pub duration: Duration,
    /// Intrinsic canvas width in px, the `width` a `<lottie>` gets with no CSS.
    pub width: u32,
    pub height: u32,
    pub telegram_sticker: bool,
}

/// How playback behaves at the end of the active segment; mirrors lottie-web's `loop` (`true`,
/// `false` or a number).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum LoopMode {
    None,
    /// `loop: true`: wrap forever; never completes.
    #[default]
    Loop,
    /// `loop: <n>`: wrap `n` times then complete, so the animation plays `n + 1` times, as in lottie-web.
    Count(u32),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Direction {
    #[default]
    Forward,
    Reverse,
}

impl Direction {
    pub(crate) fn sign(self) -> f32 {
        match self {
            Self::Forward => 1.0,
            Self::Reverse => -1.0,
        }
    }

    pub(crate) fn flipped(self) -> Self {
        match self {
            Self::Forward => Self::Reverse,
            Self::Reverse => Self::Forward,
        }
    }
}

/// How a loop boundary is crossed, mirroring `<lottie-player mode>`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PlayMode {
    /// `mode="normal"` — jump back to the start of the segment.
    #[default]
    Normal,
    /// `mode="bounce"` — reverse direction instead of jumping (ping-pong).
    Bounce,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum LottieEvent {
    /// `enterFrame` — the playhead moved this tick.
    EnterFrame {
        frame: f32,
        elapsed: f32,
    },
    /// `loopComplete` — one pass over the active segment finished and playback
    /// wrapped (or bounced).
    LoopComplete {
        count: u32,
    },
    /// `complete` — playback finished and will not resume on its own.
    Complete,
}

/// The outcome of one [`LottieState::advance`] tick. Flat and `Copy`: the frame pump calls it on
/// every animating node every frame, so it must not allocate.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Advance {
    /// The playhead moved, so the cached frame is stale and the host must
    /// repaint. Also true on the tick a paused animation completes.
    pub needs_repaint: bool,
    pub entered_frame: Option<f32>,
    /// How many segment passes finished this tick (`loopComplete`). Normally 0
    /// or 1; larger only if the tick was long enough to lap the segment.
    pub loops_completed: u32,
    pub completed: bool,
    pub loop_count: u32,
    pub elapsed_frames: f32,
}

impl Advance {
    /// The events this tick produced, in lottie-web's firing order:
    /// `enterFrame`, then `loopComplete`, then `complete`.
    pub fn events(&self) -> impl Iterator<Item = LottieEvent> + '_ {
        let enter = self.entered_frame.map(|frame| LottieEvent::EnterFrame {
            frame,
            elapsed: self.elapsed_frames,
        });
        // `loops_completed` counts boundaries crossed this tick; report the
        // running total each one landed on.
        let first_loop = self.loop_count - self.loops_completed;
        let loops = (0..self.loops_completed).map(move |index| LottieEvent::LoopComplete {
            count: first_loop + index + 1,
        });
        let complete = self.completed.then_some(LottieEvent::Complete);
        enter.into_iter().chain(loops).chain(complete)
    }
}

struct CachedFrame {
    /// `f32::to_bits` of the frame the raster was produced at. Bit equality is
    /// the right test: a paused playhead does not move at all, so this is exact.
    frame_bits: u32,
    width: u32,
    height: u32,
    image: Arc<RenderImage>,
}

/// Persistent playback state for one `<lottie>` node; the host keeps one per node in a side map.
pub struct LottieState {
    renderer: Rasteriser,
    advisories: Vec<String>,
    transport: Transport,

    cached: Option<CachedFrame>,
    /// Held one extra frame before `Window::drop_image`: a texture retired the frame it was last
    /// painted could still be referenced by the scene being presented.
    previous: Option<Arc<RenderImage>>,
    retired: Vec<Arc<RenderImage>>,
}

impl LottieState {
    /// Parses `bytes` (plain Lottie JSON or a gzipped `.tgs`, sniffed by the `1f 8b` magic) and
    /// prepares it for rendering. The document goes through [`crate::normalize`] first; what is
    /// still unrenderable comes back as [`LottieError::Unsupported`] with one advisory per feature.
    pub fn load(bytes: &[u8]) -> Result<Self, LottieError> {
        Self::load_with(Backend::preferred(), bytes)
    }

    /// [`Self::load`] with the rasteriser named. Asking for a backend that was not compiled in is
    /// an error, not a silent downgrade; check [`Backend::is_available`].
    pub fn load_with(backend: Backend, bytes: &[u8]) -> Result<Self, LottieError> {
        let Prepared {
            renderer,
            meta,
            advisories,
            ..
        } = prepare(backend, bytes)?;

        Ok(Self {
            renderer,
            advisories,
            transport: Transport::new(meta),
            cached: None,
            previous: None,
            retired: Vec::new(),
        })
    }

    pub fn meta(&self) -> &LottieMeta {
        self.transport.meta()
    }

    pub fn transport(&self) -> &Transport {
        &self.transport
    }

    pub fn backend(&self) -> Backend {
        self.renderer.backend()
    }

    /// Non-fatal notes about what the compatibility pass had to rewrite, for the
    /// host's `[vue-native compat]` channel. Empty for a clean file.
    pub fn advisories(&self) -> &[String] {
        &self.advisories
    }

    pub fn play(&mut self) {
        self.transport.play()
    }

    pub fn pause(&mut self) {
        self.transport.pause()
    }

    pub fn stop(&mut self) {
        self.transport.stop()
    }

    pub fn seek_frame(&mut self, frame: f32) {
        self.transport.seek_frame(frame)
    }

    pub fn seek_progress(&mut self, progress: f32) {
        self.transport.seek_progress(progress)
    }

    pub fn frame(&self) -> f32 {
        self.transport.frame()
    }

    pub fn progress(&self) -> f32 {
        self.transport.progress()
    }

    pub fn set_speed(&mut self, speed: f32) {
        self.transport.set_speed(speed)
    }

    pub fn speed(&self) -> f32 {
        self.transport.speed()
    }

    pub fn set_direction(&mut self, direction: Direction) {
        self.transport.set_direction(direction)
    }

    pub fn direction(&self) -> Direction {
        self.transport.direction()
    }

    pub fn set_loop_mode(&mut self, loop_mode: LoopMode) {
        self.transport.set_loop_mode(loop_mode)
    }

    pub fn loop_mode(&self) -> LoopMode {
        self.transport.loop_mode()
    }

    pub fn set_play_mode(&mut self, play_mode: PlayMode) {
        self.transport.set_play_mode(play_mode)
    }

    pub fn set_segment(&mut self, segment: Option<(f32, f32)>) {
        self.transport.set_segment(segment)
    }

    pub fn active_segment(&self) -> (f32, f32) {
        self.transport.active_segment()
    }

    pub fn needs_frame(&self) -> bool {
        self.transport.needs_frame()
    }

    pub fn is_finished(&self) -> bool {
        self.transport.is_finished()
    }

    pub fn is_playing(&self) -> bool {
        self.transport.is_playing()
    }

    pub fn advance(&mut self, dt: Duration) -> Advance {
        self.transport.advance(dt)
    }

    /// Rasterises the current frame for a CSS box of `size` logical pixels on a display of `scale`
    /// device pixels per logical pixel, at physical size (`size * scale`, rounded) so it stays
    /// crisp. Scaled by `min(width_px / w, height_px / h)`: one texel per physical pixel under
    /// `contain`. Memoised on `(frame, width_px, height_px)`: a paused animation reuses its raster
    /// and a resize invalidates it. `None` for a degenerate (zero-area) box.
    pub fn render_frame(&mut self, size: (f32, f32), scale: f32) -> Option<Arc<RenderImage>> {
        let width_px = (size.0 * scale).round().max(0.0) as u32;
        let height_px = (size.1 * scale).round().max(0.0) as u32;
        if width_px == 0 || height_px == 0 {
            return None;
        }

        let frame = self.transport.frame();
        let frame_bits = frame.to_bits();
        if let Some(cached) = &self.cached {
            if cached.frame_bits == frame_bits
                && cached.width == width_px
                && cached.height == height_px
            {
                return Some(cached.image.clone());
            }
        }

        let meta = self.transport.meta();
        let intrinsic = (meta.width.max(1), meta.height.max(1));
        let raster_scale = (width_px as f32 / intrinsic.0 as f32)
            .min(height_px as f32 / intrinsic.1 as f32);
        let (raster_width, raster_height) = backend::raster_size(intrinsic, raster_scale);
        let raster = self
            .renderer
            .render(frame, raster_scale, raster_width, raster_height)?;

        // `RgbaImage` is a byte container here; the bytes are already BGRA, as gpui's atlas uploads.
        let buffer = RgbaImage::from_raw(raster.width, raster.height, raster.bgra)?;
        let image = Arc::new(RenderImage::new(vec![Frame::new(buffer)]).dynamic());

        // Retire the frame before last: the scene that painted it has been presented.
        if let Some(previous) = self.previous.take() {
            if !self
                .cached
                .as_ref()
                .is_some_and(|current| current.image.id == previous.id)
            {
                self.retired.push(previous);
            }
        }
        self.previous = self.cached.take().map(|cached| cached.image);
        self.cached = Some(CachedFrame {
            frame_bits,
            width: width_px,
            height: height_px,
            image: image.clone(),
        });
        Some(image)
    }

    /// Rasters no longer painted whose atlas tiles the caller must release with
    /// `Window::drop_image`. The host must drain this every frame it builds a `<lottie>` element,
    /// or a 60fps animation leaks one texture per frame.
    #[must_use = "the atlas tiles leak unless these are passed to Window::drop_image"]
    pub fn take_retired(&mut self) -> Vec<Arc<RenderImage>> {
        std::mem::take(&mut self.retired)
    }

    /// Every raster still held, for teardown; the state is unusable afterwards.
    #[must_use = "the atlas tiles leak unless these are passed to Window::drop_image"]
    pub fn take_all_images(&mut self) -> Vec<Arc<RenderImage>> {
        let mut images = std::mem::take(&mut self.retired);
        images.extend(self.previous.take());
        images.extend(self.cached.take().map(|cached| cached.image));
        images
    }
}

/// The raster [`LottieState::render_frame`] draws for a box of `size` logical px on a `scale`
/// display: one texel per device px under `contain`, sized as both backends size it. `None` for a
/// box with no area. A host that draws elsewhere ([`crate::Animation::render_at`]) asks this for
/// the same pixels.
pub fn raster_size_for(meta: &LottieMeta, size: (f32, f32), scale: f32) -> Option<(u32, u32)> {
    let width_px = (size.0 * scale).round().max(0.0) as u32;
    let height_px = (size.1 * scale).round().max(0.0) as u32;
    if width_px == 0 || height_px == 0 {
        return None;
    }
    let intrinsic = (meta.width.max(1), meta.height.max(1));
    let raster_scale = (width_px as f32 / intrinsic.0 as f32).min(height_px as f32 / intrinsic.1 as f32);
    Some(backend::raster_size(intrinsic, raster_scale))
}

/// A loaded document, before any transport: what [`LottieState`] and
/// [`crate::Animation`] are both built from.
pub(crate) struct Prepared {
    pub renderer: Rasteriser,
    pub meta: LottieMeta,
    pub advisories: Vec<String>,
    pub json_bytes: usize,
}

pub(crate) fn prepare(backend: Backend, bytes: &[u8]) -> Result<Prepared, LottieError> {
    let json = if bytes.starts_with(&[0x1f, 0x8b]) {
        use std::io::Read as _;
        let mut text = String::new();
        flate2::read::GzDecoder::new(bytes)
            .read_to_string(&mut text)
            .map_err(LottieError::Decompress)?;
        text
    } else {
        String::from_utf8_lossy(bytes).into_owned()
    };

    let mut document: Value = serde_json::from_str(&json).map_err(LottieError::Json)?;
    let json_bytes = json.len();
    drop(json);
    let telegram_sticker = document.get("tgs").is_some();
    let normalization = normalize::normalize(&mut document);

    let meta = read_meta(&document, telegram_sticker)?;
    let renderer = Rasteriser::load(backend, document, &meta)?;
    Ok(Prepared {
        renderer,
        meta,
        advisories: advisories_for(normalization),
        json_bytes,
    })
}

/// Reads [`LottieMeta`] straight out of the Bodymovin document, not from a backend: the engines
/// disagree on the timeline (ThorVG's frame count is 0-based and its duration already divided by
/// the frame rate) and `meta()` must not depend on which is compiled in. Required fields mirror
/// `rasterlottie::Animation`'s, so a non-Lottie fails with [`LottieError::Parse`].
fn read_meta(document: &Value, telegram_sticker: bool) -> Result<LottieMeta, LottieError> {
    fn number(document: &Value, key: &str) -> Result<f32, LottieError> {
        document
            .get(key)
            .and_then(Value::as_f64)
            .map(|value| value as f32)
            .ok_or_else(|| {
                LottieError::Parse(format!("missing or non-numeric required field {key:?}"))
            })
    }

    let frame_rate = number(document, "fr")?;
    let in_point = number(document, "ip")?;
    let out_point = number(document, "op")?;
    let width = number(document, "w")?.max(0.0) as u32;
    let height = number(document, "h")?.max(0.0) as u32;
    let total_frames = out_point - in_point;

    Ok(LottieMeta {
        // `"v"` is optional in the wild, so it defaults rather than failing the load.
        version: document
            .get("v")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned(),
        frame_rate,
        in_point,
        out_point,
        total_frames,
        duration: Duration::from_secs_f32(if frame_rate > 0.0 {
            (total_frames / frame_rate).max(0.0)
        } else {
            0.0
        }),
        width,
        height,
        telegram_sticker,
    })
}

fn advisories_for(normalization: Normalization) -> Vec<String> {
    let mut advisories = Vec::new();
    // Worded without naming a backend: `rasterlottie` needs the pass, ThorVG reads both idioms natively.
    if normalization.terminator_keyframes > 0 {
        advisories.push(format!(
            "filled {} Bodymovin terminator keyframe(s) ({{\"t\": n}} with no \"s\") \
             with the value the previous keyframe implies",
            normalization.terminator_keyframes
        ));
    }
    if normalization.solid_layers > 0 {
        advisories.push(format!(
            "rewrote {} solid layer(s) into the equivalent filled rectangles",
            normalization.solid_layers
        ));
    }
    advisories
}
