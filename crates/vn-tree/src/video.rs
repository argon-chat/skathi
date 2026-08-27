//! `<video>` — the element layer over the `vn-video` crate.
//!
//! This module owns everything that outlives a frame: the per-node
//! [`VideoState`] side map, the `src` → path resolve, the prop→transport sync,
//! the once-per-frame tick and the event dispatch. The gpui element itself is
//! built in [`crate::render`] (`build_video`), which is where the cascade, the
//! focus handles and the listener wiring already live.
//!
//! `crates/vn-video/CONTRACT.md` is the spec this implements. It is deliberately
//! the same five entry points `lottie.rs` exposes — `sync`, `gc`, `any_playing`,
//! `tick`, event dispatch — because the host wiring should not have to learn a
//! second shape per media element.
//!
//! # The three element-level decisions
//!
//! 1. **`autoplay` defaults to `false`,** as it does on `<lottie>` and unlike a
//!    browser's `<video autoplay>`… which is the same thing, because a browser
//!    only honours `autoplay` when the video is muted and the page has user
//!    activation. Here the attribute is also EDGE-TRIGGERED, exactly as
//!    `<lottie autoplay>` is: `:autoplay="isPlaying"` is a transport control.
//!    A browser reads `autoplay` once and expects `play()`/`pause()` for the
//!    rest, which needs `HTMLMediaElement` methods on the JS mirror — see §6 of
//!    the CONTRACT for the one-file change that adds them. Until then this is a
//!    documented superset, never a silent divergence.
//! 2. **`currentTime` is written through the WRITE COUNTER,** not the value.
//!    `el.currentTime = 0` on a video already at 0 must re-seek (that is how a
//!    replay button works), and a prop channel that dedupes on the string alone
//!    cannot see it. The `<input value>` path solved this once already; this
//!    reuses it.
//! 3. **A detached `<video>` is SUSPENDED, unlike a detached `<lottie>`.**
//!    A parked `<KeepAlive>` page keeps its Lottie clock running because the
//!    cost is arithmetic. A video costs ~28 MiB of RAM and ~68 MiB of VRAM per
//!    1080p stream, and a browser does not keep decoding one in a detached
//!    subtree either.

use std::cell::RefCell;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Instant;

use gpui::{App, RenderImage, Window};
use vn_video::{
    FrameRequest, MediaError, MediaEvent, MediaSource, VideoOptions, VideoState,
};

use crate::tree::{self, Tree};
use crate::EventPayload;

/// One `<video>` node's persistent state.
struct Entry {
    /// The raw `src` string the state was built from — the (re)load key.
    src: String,
    /// `None` when no backend could be built or the `src` could not be
    /// resolved. Remembered rather than retried, so a broken `src` costs one
    /// advisory, not one per frame.
    state: Option<VideoState>,
    /// Last `autoplay` value seen, so play/pause is edge-triggered off the
    /// *prop* (decision 1 in the module docs).
    autoplay: bool,
    /// Last `currentTime` write counter seen (decision 2).
    seek_write: Option<u32>,
    /// The box the element was last built at, in physical pixels — what the
    /// next tick asks the decoder for. Written by the build, read by the tick.
    frame_box: (u32, u32),
    /// Was this node built in the frame just past? A node that stops being
    /// built has been detached or `display: none`d, and must stop decoding.
    built: bool,
    /// The events emitted since JS last drained them, newest last. A ring
    /// rather than a queue: this is a *debug and bridge* surface
    /// (`__vn_media_events`), and an app that never drains it must not grow.
    recent: Vec<&'static str>,
    /// Has this element's error already been turned into an advisory?
    warned_error: bool,
}

thread_local! {
    static VIDEO_STATES: RefCell<HashMap<u32, Entry>> = RefCell::new(HashMap::new());
    /// Frames whose node was GC'd. `gc_node_state` has no `Window`, and
    /// dropping a `RenderImage` without `Window::drop_image` leaks its atlas
    /// tiles — so they wait here for the next frame that does.
    static PENDING_DROPS: RefCell<Vec<Arc<RenderImage>>> = const { RefCell::new(Vec::new()) };
}

const RING: usize = 32;

// ---------------------------------------------------------------------------
// props
// ---------------------------------------------------------------------------

/// The browser's boolean-attribute rule: present is true unless it spells
/// `"false"` (which is what `:attr="false"` serialises to through `patchProp`).
fn bool_attr(raw: Option<&str>) -> bool {
    raw.is_some_and(|v| v.trim() != "false")
}

fn number_attr(raw: Option<&str>) -> Option<f64> {
    raw.and_then(|v| v.trim().parse::<f64>().ok())
}

/// Props `<video>` has in a browser that this renderer does not implement.
/// Compat-first: say what was dropped and what to do instead, once.
const UNSUPPORTED_PROPS: &[(&str, &str)] = &[
    (
        "controls",
        "there is no UA transport chrome — build one in Vue and drive it through the element's \
         properties (the MediaPage in apps/hello is a worked example)",
    ),
    (
        "crossorigin",
        "sources are local files; there is no CORS to configure",
    ),
    (
        "preload",
        "the OS media pipeline decides its own read-ahead; the attribute is accepted and ignored",
    ),
    (
        "disablepictureinpicture",
        "there is no picture-in-picture to disable",
    ),
    (
        "disableremoteplayback",
        "there is no remote playback to disable",
    ),
    (
        "autopictureinpicture",
        "there is no picture-in-picture",
    ),
    (
        "srcobject",
        "MediaStream sources are not supported — this build exposes no getUserMedia and no \
         MediaStream. Use a file src.",
    ),
];

fn warn_unsupported_props(t: &Tree, id: u32) {
    for (name, alternative) in UNSUPPORTED_PROPS {
        if t.prop(id, name).is_some() {
            crate::render::warn_compat(
                &format!("video-prop:{name}"),
                &format!("<video {name}=…> is not supported — {alternative}"),
            );
        }
    }
    // `playsinline` is accepted in silence: we are never fullscreen by default,
    // so the attribute already describes what happens.
}

// ---------------------------------------------------------------------------
// lifecycle
// ---------------------------------------------------------------------------

/// Creates the state if `src` changed (or there is none yet), then pushes every
/// prop into the transport. Returns the video's intrinsic size, which is what
/// the box falls back to when CSS declares none.
///
/// Called from the render walk, once per node per frame. Everything expensive
/// (backend creation, the load) is behind the `src` comparison; the rest is a
/// handful of setters the crate dedupes.
pub(crate) fn sync(t: &Tree, id: u32) -> Option<(u32, u32)> {
    warn_unsupported_props(t, id);
    let src = t.prop(id, "src").unwrap_or("").trim().to_string();
    let autoplay = bool_attr(t.prop(id, "autoplay"));

    VIDEO_STATES.with(|m| {
        let mut map = m.borrow_mut();
        let fresh = match map.get(&id) {
            Some(entry) => entry.src != src,
            None => true,
        };
        if fresh {
            if let Some(mut old) = map.remove(&id) {
                if let Some(state) = old.state.as_mut() {
                    PENDING_DROPS.with(|p| p.borrow_mut().extend(state.take_all_images()));
                }
            }
            map.insert(
                id,
                Entry {
                    src: src.clone(),
                    state: load(&src),
                    autoplay,
                    seek_write: None,
                    frame_box: (0, 0),
                    built: true,
                    recent: Vec::new(),
                    warned_error: false,
                },
            );
        }

        let entry = map.get_mut(&id)?;
        entry.built = true;
        let state = entry.state.as_mut()?;

        // --- transport, every frame (the crate no-ops on unchanged values) ---
        state.set_looping(bool_attr(t.prop(id, "loop")));
        state.set_muted(bool_attr(t.prop(id, "muted")));
        state.set_volume(number_attr(t.prop(id, "volume")).unwrap_or(1.0));
        // The DOM property is `playbackRate`; Vue hyphenates/lowercases an
        // attribute, so both spellings arrive here and both are accepted.
        state.set_rate(
            number_attr(t.prop(id, "playbackrate"))
                .or_else(|| number_attr(t.prop(id, "playback-rate")))
                .unwrap_or(1.0),
        );

        // `currentTime`: the WRITE COUNTER decides, not the value (decision 2).
        if let Some((value, write)) = t
            .prop_write(id, "currenttime")
            .or_else(|| t.prop_write(id, "current-time"))
        {
            if entry.seek_write != Some(write) {
                entry.seek_write = Some(write);
                if let Ok(seconds) = value.trim().parse::<f64>() {
                    state.seek(seconds);
                }
            }
        }

        // `autoplay`, EDGE-triggered off the prop (decision 1).
        if fresh || autoplay != entry.autoplay {
            state.set_playing(autoplay);
        }
        entry.autoplay = autoplay;

        state.native_size()
    })
}

/// Builds one node's state. `None` means "nothing to paint" — a broken
/// `<video>` behaves like a broken `<img>`: an advisory, an empty box (or the
/// poster), never a panic.
fn load(src: &str) -> Option<VideoState> {
    if src.is_empty() {
        // An absent `src` is not an error worth shouting about (a `v-if`'d or
        // not-yet-resolved binding hits this on its first frame).
        return None;
    }
    let path = match crate::img::resolve_path("video", src) {
        Ok(path) => path,
        Err(msg) => {
            crate::render::warn_compat(&format!("video-src:{src}"), &msg);
            return None;
        }
    };
    if !path.exists() {
        crate::render::warn_compat(
            &format!("video-src:{src}"),
            &format!(
                "<video src=\"{src}\"> could not be loaded — {} does not exist. Relative paths \
                 resolve against the app root (apps/<app>/), the same base <img src> uses.",
                path.display()
            ),
        );
        return None;
    }

    let backend = match vn_video::backend_for_this_platform() {
        Ok(backend) => backend,
        Err(error) => {
            report(src, &error);
            return None;
        }
    };
    let mut state = VideoState::new(backend);
    match state.set_source(MediaSource::Path(path)) {
        Ok(()) => Some(state),
        Err(error) => {
            report(src, &error);
            None
        }
    }
}

/// One media error → one advisory line, in the shape the rest of the renderer
/// uses. The advisory is the part that matters: it names the missing decoder
/// and the fix (see `vn_video::MediaError`).
fn report(src: &str, error: &MediaError) {
    let text = match &error.advisory {
        Some(advisory) => advisory.clone(),
        None => format!("<video src=\"{src}\">: {}", error.message),
    };
    crate::render::warn_compat(&format!("video-error:{src}:{:?}", error.code), &text);
}

/// Runs `f` with one node's live state, if it has one.
pub(crate) fn with_state<R>(id: u32, f: impl FnOnce(&mut VideoState) -> R) -> Option<R> {
    VIDEO_STATES.with(|m| m.borrow_mut().get_mut(&id)?.state.as_mut().map(f))
}

/// Records the box this node was built at, so the next tick asks the decoder
/// for a frame the size we are actually going to paint — the single biggest
/// lever on the present cost (`vn_video::FrameRequest`).
pub(crate) fn note_frame_box(id: u32, options: &VideoOptions) {
    let want = options.max_frame_size();
    VIDEO_STATES.with(|m| {
        if let Some(entry) = m.borrow_mut().get_mut(&id) {
            entry.frame_box = want;
        }
    });
}

/// Drops states for nodes that no longer exist, handing their frames back
/// through [`PENDING_DROPS`].
pub(crate) fn gc(live: impl Fn(u32) -> bool) {
    VIDEO_STATES.with(|m| {
        m.borrow_mut().retain(|&id, entry| {
            if live(id) {
                return true;
            }
            if let Some(state) = entry.state.as_mut() {
                PENDING_DROPS.with(|p| p.borrow_mut().extend(state.take_all_images()));
            }
            false
        })
    });
}

// ---------------------------------------------------------------------------
// the frame pump
// ---------------------------------------------------------------------------

/// True while any `<video>` still needs frames — folded into the host's
/// `animating()` OR-chain next to `<lottie>`'s.
pub fn any_playing() -> bool {
    VIDEO_STATES.with(|m| {
        m.borrow()
            .values()
            .filter_map(|e| e.state.as_ref())
            .any(|s| s.needs_frames())
    })
}

/// Pulls a frame for every live `<video>` and dispatches the events that
/// produced. Returns `true` when something moved and the window must repaint.
///
/// Runs from the host's frame pump, **before** the draw that builds the
/// elements, so the frame a node paints is the one this tick pulled.
pub fn tick(window: &mut Window, cx: &mut App) -> bool {
    drain_pending_drops(window, cx);

    let now = Instant::now();
    let scale = window.scale_factor();

    // Collect first, dispatch after: a JS handler can mutate the tree (and
    // `flush_changes` re-enters `gc`), so the map must not be borrowed while
    // any of this reaches JavaScript.
    let mut moved = false;
    let ticks: Vec<(u32, Vec<MediaEvent>)> = VIDEO_STATES.with(|m| {
        let mut map = m.borrow_mut();
        let mut out = Vec::new();
        for (&id, entry) in map.iter_mut() {
            // A node that was not built last frame is detached, `display:
            // none`, or parked in a `<KeepAlive>` cache — stop decoding
            // (decision 3 in the module docs).
            let built = std::mem::take(&mut entry.built);
            let Some(state) = entry.state.as_mut() else { continue };
            if !built {
                state.suspend();
                continue;
            }
            // Before the first build the box is unknown; the video's own size
            // is the honest first guess and one frame later the real box wins.
            let max_size = if entry.frame_box == (0, 0) {
                state
                    .native_size()
                    .map(|(w, h)| {
                        ((w as f32 * scale) as u32, (h as f32 * scale) as u32)
                    })
                    .unwrap_or((0, 0))
            } else {
                entry.frame_box
            };
            if max_size == (0, 0) {
                continue;
            }
            let advance = state.advance(now, &FrameRequest { max_size });
            if advance.new_frame {
                moved = true;
            }
            if !advance.events.is_empty() {
                for event in &advance.events {
                    if entry.recent.len() >= RING {
                        entry.recent.remove(0);
                    }
                    entry.recent.push(event.name());
                }
                out.push((id, advance.events));
            }
        }
        out
    });

    for (id, events) in ticks {
        dispatch_events(id, &events, window, cx);
    }
    moved
}

fn drain_pending_drops(window: &mut Window, cx: &mut App) {
    let images = PENDING_DROPS.with(|p| std::mem::take(&mut *p.borrow_mut()));
    for image in images {
        cx.drop_image(image, Some(window));
    }
}

fn event_code(event: MediaEvent) -> u32 {
    match event {
        MediaEvent::LoadStart => tree::EV_LOADSTART,
        MediaEvent::LoadedMetadata => tree::EV_LOADEDMETADATA,
        MediaEvent::LoadedData => tree::EV_LOADEDDATA,
        MediaEvent::CanPlay => tree::EV_CANPLAY,
        MediaEvent::CanPlayThrough => tree::EV_CANPLAYTHROUGH,
        MediaEvent::Play => tree::EV_PLAY,
        MediaEvent::Playing => tree::EV_PLAYING,
        MediaEvent::Pause => tree::EV_PAUSE,
        MediaEvent::Seeking => tree::EV_SEEKING,
        MediaEvent::Seeked => tree::EV_SEEKED,
        MediaEvent::TimeUpdate => tree::EV_TIMEUPDATE,
        MediaEvent::DurationChange => tree::EV_DURATIONCHANGE,
        MediaEvent::RateChange => tree::EV_RATECHANGE,
        MediaEvent::VolumeChange => tree::EV_VOLUMECHANGE,
        MediaEvent::Waiting => tree::EV_WAITING,
        MediaEvent::Ended => tree::EV_ENDED,
        MediaEvent::Error => tree::EV_ERROR,
        MediaEvent::Emptied => tree::EV_EMPTIED,
    }
}

/// One tick's events, gated on the listener bits.
///
/// An `error` is ALSO routed through the compat channel, once, whether or not
/// anyone is listening: a `<video>` that silently shows nothing is the exact
/// failure mode the advisory policy exists to prevent, and a developer who has
/// not written an `@error` handler is precisely the one who needs to be told
/// which codec is missing.
fn dispatch_events(id: u32, events: &[MediaEvent], window: &mut Window, cx: &mut App) {
    for &event in events {
        if event == MediaEvent::Error {
            let already = VIDEO_STATES.with(|m| {
                m.borrow_mut()
                    .get_mut(&id)
                    .map(|e| std::mem::replace(&mut e.warned_error, true))
                    .unwrap_or(true)
            });
            if !already {
                let src = VIDEO_STATES
                    .with(|m| m.borrow().get(&id).map(|e| e.src.clone()))
                    .unwrap_or_default();
                if let Some(error) = with_state(id, |s| s.error()).flatten() {
                    report(&src, &error);
                }
            }
        }
    }

    let listeners = tree::read_tree(|t| t.node(id).map(|n| n.listeners)).unwrap_or(0);
    if listeners == 0 {
        return;
    }
    for &event in events {
        let code = event_code(event);
        if listeners & (1u64 << code) == 0 {
            continue;
        }
        // `message` has to outlive the payload that borrows it, so it is bound
        // here rather than inside the branch.
        let error = (event == MediaEvent::Error)
            .then(|| with_state(id, |s| s.error()).flatten())
            .flatten();
        let message = error
            .as_ref()
            .map(|e| e.advisory.clone().unwrap_or_else(|| e.message.clone()))
            .unwrap_or_default();
        let payload = if event == MediaEvent::Error {
            EventPayload {
                a: error.as_ref().map(|e| e.code as u8 as f64).unwrap_or(4.0),
                b: with_state(id, |s| s.current_time()).unwrap_or(0.0),
                c: 0.0,
                d: 0.0,
                key: Some(&message),
            }
        } else {
            let (time, duration, ready, rate) = with_state(id, |s| {
                (
                    s.current_time(),
                    s.duration(),
                    s.ready_state() as u8 as f64,
                    s.playback_rate(),
                )
            })
            .unwrap_or((0.0, 0.0, 0.0, 1.0));
            EventPayload {
                // NaN crosses into JS as NaN, which is exactly what
                // `video.duration` reads before metadata in a browser.
                a: time,
                b: duration,
                c: ready,
                d: rate,
                key: None,
            }
        };
        crate::render::js_dispatch_app(id, code, &payload, window, cx);
    }
}

// ---------------------------------------------------------------------------
// the DOM surface (see crates/vn-video/CONTRACT.md §6)
// ---------------------------------------------------------------------------

/// `HTMLMediaElement`'s readable properties, as `name=value` lines — the same
/// shape `dom::computed_style` uses, and for the same reason: the payload is
/// numbers and the caller is three lines of JS.
///
/// This is the PROPERTY half of the media surface. The JS runtime will read it
/// to answer `el.currentTime`, `el.duration`, `el.paused`… ; until it grows
/// those accessors an app can call `__vn_media_state(id)` directly, which is
/// what the MediaPage demo does.
pub(crate) fn media_state(id: u32) -> Option<String> {
    with_state(id, |s| {
        let ranges = |list: Vec<(f64, f64)>| {
            list.iter()
                .map(|(a, b)| format!("{a}:{b}"))
                .collect::<Vec<_>>()
                .join(",")
        };
        let mut out = String::new();
        let mut put = |k: &str, v: String| {
            out.push_str(k);
            out.push('=');
            out.push_str(&v);
            out.push('\n');
        };
        put("currentTime", s.current_time().to_string());
        put("duration", s.duration().to_string());
        put("paused", s.paused().to_string());
        put("ended", s.ended().to_string());
        put("seeking", s.seeking().to_string());
        put("readyState", (s.ready_state() as u8).to_string());
        put("networkState", (s.network_state() as u8).to_string());
        put("volume", s.volume().to_string());
        put("muted", s.muted().to_string());
        put("playbackRate", s.playback_rate().to_string());
        put("loop", s.looping().to_string());
        let (w, h) = s.native_size().unwrap_or((0, 0));
        put("videoWidth", w.to_string());
        put("videoHeight", h.to_string());
        put("buffered", ranges(s.buffered()));
        put("seekable", ranges(s.seekable()));
        if let Some(error) = s.error() {
            put("error", (error.code as u8).to_string());
            put(
                "errorMessage",
                error.advisory.unwrap_or(error.message).replace('\n', " "),
            );
        }
        out
    })
}

/// The METHOD half: `play()` / `pause()` / `currentTime =` / `volume =` … .
/// Verbs rather than names because this is a two-argument host call and the
/// JS side is a switch either way.
pub(crate) fn media_call(id: u32, verb: u32, arg: f64) -> bool {
    with_state(id, |s| match verb {
        0 => {
            s.play();
            true
        }
        1 => {
            s.pause();
            true
        }
        2 => {
            s.seek(arg);
            true
        }
        3 => {
            s.set_volume(arg);
            true
        }
        4 => {
            s.set_muted(arg != 0.0);
            true
        }
        5 => {
            s.set_rate(arg);
            true
        }
        6 => {
            s.set_looping(arg != 0.0);
            true
        }
        _ => false,
    })
    .unwrap_or(false)
}

/// `canPlayType(type)` — `""` / `"maybe"` / `"probably"`, from a LIVE probe of
/// this machine (see the owner constraint in `vn_video`'s crate docs).
pub(crate) fn can_play_type(id: u32, mime_type: &str) -> String {
    with_state(id, |s| s.can_play_type(mime_type).as_str().to_string()).unwrap_or_default()
}

/// Drains the per-node event ring, comma-joined.
///
/// A bridge and a test surface, not the event channel: media events are
/// dispatched through the ordinary `EV_*` path above. This exists because the
/// JS runtime does not yet know the media event codes (CONTRACT.md §6), so an
/// app that wants `@ended` today polls this and re-dispatches — and because a
/// devtools assertion like "did `ended` actually fire" should not have to
/// install a listener.
pub(crate) fn drain_recent(id: u32) -> String {
    VIDEO_STATES.with(|m| {
        m.borrow_mut()
            .get_mut(&id)
            .map(|e| std::mem::take(&mut e.recent).join(","))
            .unwrap_or_default()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn boolean_attributes_follow_the_html_rule() {
        assert!(!bool_attr(None));
        assert!(bool_attr(Some("")));
        assert!(bool_attr(Some("true")));
        // What `:muted="false"` serialises to through patchProp.
        assert!(!bool_attr(Some("false")));
    }

    #[test]
    fn numeric_attributes_reject_nonsense_rather_than_guessing() {
        assert_eq!(number_attr(Some("0.5")), Some(0.5));
        assert_eq!(number_attr(Some(" 2 ")), Some(2.0));
        assert_eq!(number_attr(Some("fast")), None);
        assert_eq!(number_attr(None), None);
    }

    /// Every `MediaEvent` must have a code, and no two may share one — a
    /// duplicate would silently deliver one event under another's name.
    #[test]
    fn every_media_event_has_its_own_code() {
        let all = [
            MediaEvent::LoadStart,
            MediaEvent::LoadedMetadata,
            MediaEvent::LoadedData,
            MediaEvent::CanPlay,
            MediaEvent::CanPlayThrough,
            MediaEvent::Play,
            MediaEvent::Playing,
            MediaEvent::Pause,
            MediaEvent::Seeking,
            MediaEvent::Seeked,
            MediaEvent::TimeUpdate,
            MediaEvent::DurationChange,
            MediaEvent::RateChange,
            MediaEvent::VolumeChange,
            MediaEvent::Waiting,
            MediaEvent::Ended,
            MediaEvent::Error,
            MediaEvent::Emptied,
        ];
        let mut codes: Vec<u32> = all.iter().map(|&e| event_code(e)).collect();
        codes.sort_unstable();
        let unique = {
            let mut c = codes.clone();
            c.dedup();
            c
        };
        assert_eq!(codes, unique, "two media events share a host code");
        // …and every one of them is in the table devtools and the JS mirror
        // read, so a listing can never lag the codes.
        for &e in &all {
            let code = event_code(e);
            let named = tree::EVENT_NAMES.iter().find(|(c, _)| *c == code);
            assert_eq!(
                named.map(|(_, n)| *n),
                Some(e.name()),
                "code {code} ({}) is missing from tree::EVENT_NAMES",
                e.name()
            );
        }
        // The whole point of the u64 widening.
        assert!(codes.iter().all(|&c| c < u64::BITS));
    }
}
