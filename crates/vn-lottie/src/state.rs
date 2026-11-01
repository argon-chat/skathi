//! The persistent playback state behind one `<lottie>` node: the prepared
//! animation, the transport (play/pause/seek/speed/direction/loop) and the
//! rendered-frame cache.
//!
//! Everything here is deliberately free of `Window`, so the transport and the
//! event semantics are testable without opening one. The only gpui type that
//! leaks in is [`RenderImage`], which is what the element builder in
//! [`crate::element`] paints.

use std::sync::Arc;
use std::time::Duration;

use gpui::RenderImage;
use image::{Frame, RgbaImage};
use serde_json::Value;

use crate::backend::{self, Backend, Rasteriser};
use crate::transport::Transport;
use crate::normalize::{self, Normalization};

/// Why a Lottie could not be loaded.
#[derive(Debug)]
pub enum LottieError {
    /// The bytes were neither Lottie JSON nor a gzipped (`.tgs`) wrapper of it.
    Decompress(std::io::Error),
    /// The payload was not valid JSON.
    Json(serde_json::Error),
    /// The document parsed as JSON but not as a Lottie animation.
    Parse(String),
    /// The animation parsed but uses features the renderer cannot draw at all.
    /// [`LottieError::advisories`] carries the per-feature detail.
    Unsupported(Vec<String>),
}

impl LottieError {
    /// One advisory line per unsupported feature, for the host's compat channel.
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

/// What the animation says about itself, read once at load.
#[derive(Debug, Clone, PartialEq)]
pub struct LottieMeta {
    /// Bodymovin schema version string (`"v"`), e.g. `"5.11.0"`.
    pub version: String,
    /// Authored frames per second.
    pub frame_rate: f32,
    /// First frame of the animation (`"ip"`), usually 0.
    pub in_point: f32,
    /// One past the last frame (`"op"`).
    pub out_point: f32,
    /// `out_point - in_point`, the playable length in frames.
    pub total_frames: f32,
    /// Wall-clock length at `speed = 1.0`.
    pub duration: Duration,
    /// Intrinsic canvas width in px, the `width` a `<lottie>` gets with no CSS.
    pub width: u32,
    /// Intrinsic canvas height in px.
    pub height: u32,
    /// True when the source carried Telegram's `"tgs": 1` marker.
    pub telegram_sticker: bool,
}

/// How playback behaves when it reaches the end of the active segment.
///
/// Mirrors lottie-web's `loop` config value, which is `true`, `false` or a
/// number.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum LoopMode {
    /// `loop: false` — stop at the end and emit [`LottieEvent::Complete`].
    None,
    /// `loop: true` — wrap forever, emitting [`LottieEvent::LoopComplete`] each
    /// time round. Never completes.
    #[default]
    Loop,
    /// `loop: <n>` — wrap `n` times, then complete. As in lottie-web, the
    /// animation therefore plays `n + 1` times in total.
    Count(u32),
}

/// Playback direction, mirroring lottie-web's `setDirection(1 | -1)`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Direction {
    /// `1` — in_point → out_point.
    #[default]
    Forward,
    /// `-1` — out_point → in_point.
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

/// One playback event, named after its lottie-web counterpart.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum LottieEvent {
    /// `enterFrame` — the playhead moved this tick.
    EnterFrame {
        /// The new playhead position, in source frames.
        frame: f32,
        /// Frames elapsed since the start of the active segment.
        elapsed: f32,
    },
    /// `loopComplete` — one pass over the active segment finished and playback
    /// wrapped (or bounced).
    LoopComplete {
        /// How many passes have finished since load.
        count: u32,
    },
    /// `complete` — playback finished and will not resume on its own.
    Complete,
}

/// The outcome of one [`LottieState::advance`] tick.
///
/// Deliberately flat and `Copy`: the frame pump calls this on every animating
/// node every frame, so it must not allocate.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Advance {
    /// The playhead moved, so the cached frame is stale and the host must
    /// repaint. Also true on the tick a paused animation completes.
    pub needs_repaint: bool,
    /// The new playhead position when it moved (`enterFrame`).
    pub entered_frame: Option<f32>,
    /// How many segment passes finished this tick (`loopComplete`). Normally 0
    /// or 1; larger only if the tick was long enough to lap the segment.
    pub loops_completed: u32,
    /// Playback finished this tick (`complete`).
    pub completed: bool,
    /// Total passes finished since load, for the `loopComplete` payload.
    pub loop_count: u32,
    /// Frames elapsed within the active segment, for the `enterFrame` payload.
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

/// The last raster we handed gpui, keyed by everything that would change it.
struct CachedFrame {
    /// `f32::to_bits` of the frame the raster was produced at. Bit equality is
    /// the right test: a paused playhead does not move at all, so this is exact.
    frame_bits: u32,
    width: u32,
    height: u32,
    image: Arc<RenderImage>,
}

/// Persistent playback state for one `<lottie>` node.
///
/// The host keeps one of these per node in a side map keyed by node id, exactly
/// as it keeps `TextInputState` for `<input>` — see `CONTRACT.md`.
pub struct LottieState {
    renderer: Rasteriser,
    advisories: Vec<String>,
    transport: Transport,

    cached: Option<CachedFrame>,
    /// Held one extra frame before being handed back for `Window::drop_image`.
    /// gpui keys atlas tiles by `RenderImage::id`, so a texture retired in the
    /// same frame it was last painted could still be referenced by the scene
    /// being presented. This is the discipline gpui's own per-frame video view
    /// uses (`livekit_client::RemoteVideoTrackView`).
    previous: Option<Arc<RenderImage>>,
    retired: Vec<Arc<RenderImage>>,
}

impl LottieState {
    /// Parses `bytes` — plain Lottie JSON, or a gzipped `.tgs` sticker — and
    /// prepares it for rendering.
    ///
    /// The gzip magic (`1f 8b`) is sniffed, so callers never have to care which
    /// they hold; a `.tgs` is just gzipped Bodymovin JSON with a `"tgs": 1`
    /// marker, which lands in [`LottieMeta::telegram_sticker`].
    ///
    /// Before the backend sees it the document goes through
    /// [`crate::normalize`], without which most real After Effects exports are
    /// rejected outright. Whatever survives that and is still unrenderable
    /// comes back as [`LottieError::Unsupported`] carrying one advisory line
    /// per feature.
    pub fn load(bytes: &[u8]) -> Result<Self, LottieError> {
        Self::load_with(Backend::preferred(), bytes)
    }

    /// [`Self::load`], with the rasteriser named explicitly.
    ///
    /// Everything outside the rasterisation itself — the gunzip, the
    /// [`crate::normalize`] pass, [`LottieMeta`], the transport, the events —
    /// is backend-independent by construction, so the only thing this changes
    /// is which engine draws and therefore which files draw *correctly*.
    ///
    /// Asking for a backend that was not compiled in is an error rather than a
    /// silent downgrade; use [`Backend::is_available`] to check first.
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

    /// Everything the file said about itself.
    pub fn meta(&self) -> &LottieMeta {
        self.transport.meta()
    }

    /// The transport alone.
    pub fn transport(&self) -> &Transport {
        &self.transport
    }

    /// Which rasteriser is drawing this animation.
    pub fn backend(&self) -> Backend {
        self.renderer.backend()
    }

    /// Non-fatal notes about what the compatibility pass had to rewrite, for the
    /// host's `[vue-native compat]` channel. Empty for a clean file.
    pub fn advisories(&self) -> &[String] {
        &self.advisories
    }

    // ---- transport: [`Transport`]'s, see there -----------------------

    /// [`Transport::play`].
    pub fn play(&mut self) {
        self.transport.play()
    }

    /// [`Transport::pause`].
    pub fn pause(&mut self) {
        self.transport.pause()
    }

    /// [`Transport::stop`].
    pub fn stop(&mut self) {
        self.transport.stop()
    }

    /// [`Transport::seek_frame`].
    pub fn seek_frame(&mut self, frame: f32) {
        self.transport.seek_frame(frame)
    }

    /// [`Transport::seek_progress`].
    pub fn seek_progress(&mut self, progress: f32) {
        self.transport.seek_progress(progress)
    }

    /// [`Transport::frame`].
    pub fn frame(&self) -> f32 {
        self.transport.frame()
    }

    /// [`Transport::progress`].
    pub fn progress(&self) -> f32 {
        self.transport.progress()
    }

    /// [`Transport::set_speed`].
    pub fn set_speed(&mut self, speed: f32) {
        self.transport.set_speed(speed)
    }

    /// [`Transport::speed`].
    pub fn speed(&self) -> f32 {
        self.transport.speed()
    }

    /// [`Transport::set_direction`].
    pub fn set_direction(&mut self, direction: Direction) {
        self.transport.set_direction(direction)
    }

    /// [`Transport::direction`].
    pub fn direction(&self) -> Direction {
        self.transport.direction()
    }

    /// [`Transport::set_loop_mode`].
    pub fn set_loop_mode(&mut self, loop_mode: LoopMode) {
        self.transport.set_loop_mode(loop_mode)
    }

    /// [`Transport::loop_mode`].
    pub fn loop_mode(&self) -> LoopMode {
        self.transport.loop_mode()
    }

    /// [`Transport::set_play_mode`].
    pub fn set_play_mode(&mut self, play_mode: PlayMode) {
        self.transport.set_play_mode(play_mode)
    }

    /// [`Transport::set_segment`].
    pub fn set_segment(&mut self, segment: Option<(f32, f32)>) {
        self.transport.set_segment(segment)
    }

    /// [`Transport::active_segment`].
    pub fn active_segment(&self) -> (f32, f32) {
        self.transport.active_segment()
    }

    /// [`Transport::needs_frame`].
    pub fn needs_frame(&self) -> bool {
        self.transport.needs_frame()
    }

    /// [`Transport::is_finished`].
    pub fn is_finished(&self) -> bool {
        self.transport.is_finished()
    }

    /// [`Transport::is_playing`].
    pub fn is_playing(&self) -> bool {
        self.transport.is_playing()
    }

    /// [`Transport::advance`].
    pub fn advance(&mut self, dt: Duration) -> Advance {
        self.transport.advance(dt)
    }

    // ---- rasterisation --------------------------------------------------

    /// Rasterises the current frame for a CSS box of `size` logical pixels on a
    /// display of `scale` device pixels per logical pixel.
    ///
    /// # DPI
    ///
    /// The raster is produced at **physical** pixels — `size * scale`, rounded —
    /// so a 96×96 CSS box on a 150% display rasterises at 144×144 and stays
    /// crisp. Never rasterise at logical size and let the GPU upscale.
    ///
    /// The animation is scaled by `min(width_px / w, height_px / h)`, which is
    /// exactly 1 texel per physical pixel under `object-fit: contain` (the
    /// default). Under `cover` the raster is upscaled by the overflow ratio.
    ///
    /// # Caching
    ///
    /// The result is memoised on `(frame, width_px, height_px)`. A paused
    /// animation re-uses its raster forever — the playhead does not move, so the
    /// bit-exact frame comparison holds — and a resize is what invalidates it.
    /// At the sizes stickers actually run (0.1–2 ms/frame at 256px, measured)
    /// re-rendering on resize needs no debounce.
    ///
    /// Returns `None` for a degenerate (zero-area) box.
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

        // `RgbaImage` is a byte container here, not a colour-space claim: the
        // bytes are already BGRA, which is what gpui's atlas uploads.
        let buffer = RgbaImage::from_raw(raster.width, raster.height, raster.bgra)?;
        let image = Arc::new(RenderImage::new(vec![Frame::new(buffer)]).dynamic());

        // Retire the frame *before* last: the scene that painted it has been
        // presented by now, so its atlas tile is safe to release.
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

    /// Rasters that are no longer painted and whose atlas tiles the caller must
    /// release with `Window::drop_image`.
    ///
    /// **The host must drain this every frame it builds a `<lottie>` element.**
    /// gpui keys atlas tiles by `RenderImage::id` and every rasterised frame
    /// mints a fresh id, so a 60fps animation leaks one texture per frame if
    /// these are dropped on the floor.
    #[must_use = "the atlas tiles leak unless these are passed to Window::drop_image"]
    pub fn take_retired(&mut self) -> Vec<Arc<RenderImage>> {
        std::mem::take(&mut self.retired)
    }

    /// Every raster still held, for teardown when the node is removed from the
    /// tree. The state is unusable afterwards and should be dropped.
    #[must_use = "the atlas tiles leak unless these are passed to Window::drop_image"]
    pub fn take_all_images(&mut self) -> Vec<Arc<RenderImage>> {
        let mut images = std::mem::take(&mut self.retired);
        images.extend(self.previous.take());
        images.extend(self.cached.take().map(|cached| cached.image));
        images
    }
}

/// The raster [`LottieState::render_frame`] draws for a box of `size` logical
/// px on a `scale` display: the animation at one texel per device px under
/// `contain`, sized as both backends size it. `None` for a box with no area.
/// A host that draws elsewhere ([`crate::Animation::render_at`]) asks this for
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
    /// The (gunzipped) JSON's length in bytes.
    pub json_bytes: usize,
}

/// The gunzip (sniffed by the `1f 8b` magic), the JSON parse, the
/// [`crate::normalize`] pass, [`LottieMeta`] and the backend's load.
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

/// Reads [`LottieMeta`] straight out of the Bodymovin document.
///
/// Deliberately **not** delegated to a backend. Both engines expose their own
/// idea of the timeline — `rasterlottie` through its `Animation` model,
/// ThorVG through `tvg_animation_get_total_frame`/`_duration`, which report a
/// 0-based frame count and a duration already divided by the frame rate — and
/// routing metadata through either would make `meta()` depend on which one is
/// compiled in. The document is the single source of truth instead.
///
/// The required fields mirror `rasterlottie::Animation`'s non-`default`ed
/// ones, so a document that is not a Lottie fails here with the same
/// [`LottieError::Parse`] it always did.
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
        // `"v"` is optional in the wild — hand-authored files often omit it —
        // so it defaults rather than failing the load.
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
    // Worded without naming a backend: the pass runs for both, and while
    // `rasterlottie` *needs* it (it rejects the file outright), ThorVG reads
    // both idioms natively and merely gets a document that already says the
    // same thing more explicitly.
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
