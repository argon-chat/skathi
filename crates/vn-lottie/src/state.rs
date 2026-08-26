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
use rasterlottie::{Animation, PreparedAnimation, RenderConfig, Renderer, Rgba8};

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
    fn sign(self) -> f32 {
        match self {
            Self::Forward => 1.0,
            Self::Reverse => -1.0,
        }
    }

    fn flipped(self) -> Self {
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
    prepared: PreparedAnimation,
    meta: LottieMeta,
    advisories: Vec<String>,

    frame: f32,
    playing: bool,
    finished: bool,
    speed: f32,
    direction: Direction,
    /// The direction `play()` resets to; `Bounce` flips `direction` but must not
    /// lose the authored one.
    base_direction: Direction,
    loop_mode: LoopMode,
    play_mode: PlayMode,
    loops_done: u32,
    segment: Option<(f32, f32)>,

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

        let mut document: serde_json::Value =
            serde_json::from_str(&json).map_err(LottieError::Json)?;
        let telegram_sticker = document.get("tgs").is_some();
        let normalization = normalize::normalize(&mut document);

        let animation: Animation = serde_json::from_value(document)
            .map_err(|error| LottieError::Parse(error.to_string()))?;

        let report = rasterlottie::analyze_animation(&animation);
        if !report.is_supported() {
            return Err(LottieError::Unsupported(
                report
                    .issues
                    .iter()
                    .map(|issue| format!("{} ({:?}): {}", issue.path, issue.kind, issue.detail))
                    .collect(),
            ));
        }

        let meta = LottieMeta {
            version: animation.version.clone(),
            frame_rate: animation.frame_rate,
            in_point: animation.in_point,
            out_point: animation.out_point,
            total_frames: animation.duration_frames(),
            duration: Duration::from_secs_f32(animation.duration_seconds().max(0.0)),
            width: animation.width,
            height: animation.height,
            telegram_sticker,
        };

        let prepared = Renderer::default()
            .prepare(&animation)
            .map_err(|error| LottieError::Parse(error.to_string()))?;

        Ok(Self {
            prepared,
            frame: meta.in_point,
            meta,
            advisories: advisories_for(normalization),
            playing: true,
            finished: false,
            speed: 1.0,
            direction: Direction::Forward,
            base_direction: Direction::Forward,
            loop_mode: LoopMode::default(),
            play_mode: PlayMode::default(),
            loops_done: 0,
            segment: None,
            cached: None,
            previous: None,
            retired: Vec::new(),
        })
    }

    /// Everything the file said about itself.
    pub fn meta(&self) -> &LottieMeta {
        &self.meta
    }

    /// Non-fatal notes about what the compatibility pass had to rewrite, for the
    /// host's `[vue-native compat]` channel. Empty for a clean file.
    pub fn advisories(&self) -> &[String] {
        &self.advisories
    }

    // ---- transport, mirroring lottie-web's `AnimationItem` --------------

    /// `play()` — resume from the current playhead. Replaying a completed
    /// animation rewinds it, as lottie-web does.
    pub fn play(&mut self) {
        if self.finished {
            self.finished = false;
            self.loops_done = 0;
            self.direction = self.base_direction;
            self.frame = self.segment_entry();
        }
        self.playing = true;
    }

    /// `pause()` — freeze the playhead where it is.
    pub fn pause(&mut self) {
        self.playing = false;
    }

    /// `stop()` — pause and rewind to the start of the active segment.
    pub fn stop(&mut self) {
        self.playing = false;
        self.finished = false;
        self.loops_done = 0;
        self.direction = self.base_direction;
        self.frame = self.segment_entry();
    }

    /// `goToAndStop(frame, true)` — jump to an absolute source frame, clamped
    /// into the active segment.
    pub fn seek_frame(&mut self, frame: f32) {
        let (start, end) = self.active_segment();
        self.frame = frame.clamp(start, end);
        self.finished = false;
    }

    /// Jump to a normalised position in `0.0..=1.0` across the active segment.
    pub fn seek_progress(&mut self, progress: f32) {
        let (start, end) = self.active_segment();
        self.frame = start + (end - start) * progress.clamp(0.0, 1.0);
        self.finished = false;
    }

    /// The playhead, in absolute source frames.
    pub fn frame(&self) -> f32 {
        self.frame
    }

    /// The playhead as `0.0..=1.0` across the active segment.
    pub fn progress(&self) -> f32 {
        let (start, end) = self.active_segment();
        if end <= start {
            return 0.0;
        }
        ((self.frame - start) / (end - start)).clamp(0.0, 1.0)
    }

    /// `setSpeed()` — a multiplier on wall-clock time. Clamped non-negative;
    /// play *backwards* with [`Self::set_direction`], as lottie-web does.
    pub fn set_speed(&mut self, speed: f32) {
        self.speed = speed.max(0.0);
    }

    /// The current speed multiplier.
    pub fn speed(&self) -> f32 {
        self.speed
    }

    /// `setDirection()`. Under [`PlayMode::Bounce`] this also becomes the
    /// direction a `stop()`/replay returns to.
    pub fn set_direction(&mut self, direction: Direction) {
        self.direction = direction;
        self.base_direction = direction;
    }

    /// The direction the playhead is currently travelling — under
    /// [`PlayMode::Bounce`] this flips at every boundary.
    pub fn direction(&self) -> Direction {
        self.direction
    }

    /// `setLoop()`. Changing the mode clears the finished flag so a completed
    /// animation can be made to loop without an explicit `play()`.
    pub fn set_loop_mode(&mut self, loop_mode: LoopMode) {
        self.loop_mode = loop_mode;
        if loop_mode != LoopMode::None {
            self.finished = false;
        }
    }

    /// The current loop mode.
    pub fn loop_mode(&self) -> LoopMode {
        self.loop_mode
    }

    /// `mode="normal" | "bounce"`.
    pub fn set_play_mode(&mut self, play_mode: PlayMode) {
        self.play_mode = play_mode;
    }

    /// `playSegments([in, out])` — restrict playback to a frame range. `None`
    /// (`resetSegments()`) restores the full `in_point..out_point`.
    ///
    /// The playhead is pulled into the new range, so a segment change never
    /// leaves it stranded outside the window it is allowed to play.
    pub fn set_segment(&mut self, segment: Option<(f32, f32)>) {
        self.segment = segment.map(|(start, end)| {
            let (start, end) = if start <= end {
                (start, end)
            } else {
                (end, start)
            };
            (
                start.clamp(self.meta.in_point, self.meta.out_point),
                end.clamp(self.meta.in_point, self.meta.out_point),
            )
        });
        let (start, end) = self.active_segment();
        self.frame = self.frame.clamp(start, end);
    }

    /// The frame range playback is currently confined to.
    pub fn active_segment(&self) -> (f32, f32) {
        self.segment
            .unwrap_or((self.meta.in_point, self.meta.out_point))
    }

    /// True while the animation is playing and has not completed — i.e. the
    /// host must keep pumping frames for this node.
    ///
    /// The host's `animating()` OR-chain includes this, the same way it already
    /// includes the CSS transition animator. See `CONTRACT.md`.
    pub fn needs_frame(&self) -> bool {
        self.playing && !self.finished && self.speed > 0.0
    }

    /// True once playback has run to its end and stopped on its own.
    pub fn is_finished(&self) -> bool {
        self.finished
    }

    /// True while the transport is running (independent of completion).
    pub fn is_playing(&self) -> bool {
        self.playing
    }

    // ---- the tick -------------------------------------------------------

    /// Moves the playhead by `dt` of wall-clock time and reports what happened.
    ///
    /// A paused, finished or zero-speed animation returns an all-false
    /// [`Advance`] without touching anything, so calling this unconditionally on
    /// every node every frame is free.
    pub fn advance(&mut self, dt: Duration) -> Advance {
        let mut result = Advance {
            loop_count: self.loops_done,
            ..Advance::default()
        };
        if !self.needs_frame() {
            return result;
        }

        let (start, end) = self.active_segment();
        let span = end - start;
        let before = self.frame;

        // A degenerate segment has nowhere to go: treat reaching it as an
        // immediate completion rather than spinning on a zero-length loop.
        if span <= 0.0 {
            self.frame = start;
            self.finish(&mut result);
            return result;
        }

        let mut remaining = dt.as_secs_f32() * self.meta.frame_rate * self.speed;
        // Bounded because each iteration consumes at least the distance to the
        // near boundary, and a bounce/wrap resets to the far one.
        let mut guard = 0;
        while remaining > 0.0 && guard < 64 {
            guard += 1;
            let to_boundary = match self.direction {
                Direction::Forward => end - self.frame,
                Direction::Reverse => self.frame - start,
            };

            if remaining < to_boundary {
                self.frame += remaining * self.direction.sign();
                break;
            }

            // Landed on (or past) the boundary.
            remaining -= to_boundary;
            self.frame = match self.direction {
                Direction::Forward => end,
                Direction::Reverse => start,
            };
            self.loops_done += 1;
            result.loops_completed += 1;
            result.loop_count = self.loops_done;

            let keep_going = match self.loop_mode {
                LoopMode::None => false,
                LoopMode::Loop => true,
                // lottie-web's numeric `loop` is a count of *repeats*, so the
                // animation plays n + 1 times before completing.
                LoopMode::Count(count) => self.loops_done < count,
            };
            if !keep_going {
                self.finish(&mut result);
                break;
            }

            match self.play_mode {
                PlayMode::Normal => {
                    self.frame = match self.direction {
                        Direction::Forward => start,
                        Direction::Reverse => end,
                    }
                }
                PlayMode::Bounce => self.direction = self.direction.flipped(),
            }
        }

        self.frame = self.frame.clamp(start, end);
        if self.frame != before {
            result.entered_frame = Some(self.frame);
            result.needs_repaint = true;
        }
        result.elapsed_frames = self.frame - start;
        result.needs_repaint |= result.completed || result.loops_completed > 0;
        result
    }

    fn finish(&mut self, result: &mut Advance) {
        self.playing = false;
        self.finished = true;
        result.completed = true;
    }

    /// Where a fresh pass over the segment starts, given the direction.
    fn segment_entry(&self) -> f32 {
        let (start, end) = self.active_segment();
        match self.base_direction {
            Direction::Forward => start,
            Direction::Reverse => end,
        }
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

        let frame_bits = self.frame.to_bits();
        if let Some(cached) = &self.cached {
            if cached.frame_bits == frame_bits
                && cached.width == width_px
                && cached.height == height_px
            {
                return Some(cached.image.clone());
            }
        }

        let raster_scale = (width_px as f32 / self.meta.width.max(1) as f32)
            .min(height_px as f32 / self.meta.height.max(1) as f32);
        let raster = self
            .prepared
            .render_frame(
                self.frame,
                RenderConfig::new(Rgba8::TRANSPARENT, raster_scale),
            )
            .ok()?;

        // rasterlottie hands back RGBA; gpui's atlas wants BGRA.
        let mut pixels = raster.pixels;
        for pixel in pixels.chunks_exact_mut(4) {
            pixel.swap(0, 2);
        }
        let buffer = RgbaImage::from_raw(raster.width, raster.height, pixels)?;
        let image = Arc::new(RenderImage::new(vec![Frame::new(buffer)]));

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

fn advisories_for(normalization: Normalization) -> Vec<String> {
    let mut advisories = Vec::new();
    if normalization.terminator_keyframes > 0 {
        advisories.push(format!(
            "filled {} Bodymovin terminator keyframe(s) ({{\"t\": n}} with no \"s\"); \
             the renderer cannot read them but their value is implied by the previous keyframe",
            normalization.terminator_keyframes
        ));
    }
    if normalization.solid_layers > 0 {
        advisories.push(format!(
            "rewrote {} solid layer(s) into equivalent filled rectangles; \
             the renderer has no solid-layer support",
            normalization.solid_layers
        ));
    }
    advisories
}
