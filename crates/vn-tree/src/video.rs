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
    /// The last value each transport PROP had. Every one of them is pushed
    /// only when it CHANGES — see the note on `sync`.
    props: Props,
    /// Last `currentTime` write counter seen (decision 2).
    seek_write: Option<u32>,
    /// The box the element was last built at, in physical pixels — what the
    /// next tick asks the decoder for. Written by the build, read by the tick.
    frame_box: (u32, u32),
    /// The events emitted since JS last drained them, newest last. A ring
    /// rather than a queue: this is a *debug and bridge* surface
    /// (`__vn_media_events`), and an app that never drains it must not grow.
    recent: Vec<&'static str>,
    /// Has this element's error already been turned into an advisory?
    warned_error: bool,
    /// Where to pick up after a forced rebuild: `(src, currentTime, playing)`.
    ///
    /// Set only by [`device_changed`] — the renderer recreating its D3D11
    /// device is the one event that invalidates a `VideoState` for a reason
    /// that has nothing to do with the element. Carried across the rebuild and
    /// applied at `loadedmetadata`, which is the first moment a seek means
    /// anything. The `src` is part of it so an app that also changes `src` in
    /// the same frame gets the browser's answer (start at 0) rather than ours.
    resume: Option<(String, f64, bool)>,
    /// **The window that painted this node last** — recorded at creation, not
    /// at drop (docs/MULTIWINDOW.md §5.1). Two things ride on it and both are
    /// per-window gpui resources: the atlas a retired frame's tile goes back
    /// to, and the atlas a zero-copy GPU frame has to be blitted INTO. Getting
    /// the second one wrong is why a `<video>` in a pop-out painted a
    /// transparent box: the tile went to the primary window's atlas and the
    /// pop-out's had nothing to draw.
    window: crate::windows::WinKey,
    /// The scale factor of that window, from its last painted frame. Two
    /// windows can sit on displays of different DPI, and `frame_box` is in
    /// PHYSICAL pixels — so the decoder request has to use the scale of the
    /// window that paints the element, not of whichever one ran the pump.
    scale: f32,
}

thread_local! {
    static VIDEO_STATES: RefCell<HashMap<u32, Entry>> = RefCell::new(HashMap::new());
}

/// Live `<video>` elements (devtools `mem`).
pub(crate) fn count() -> usize {
    VIDEO_STATES.with(|m| m.borrow().len())
}

/// Frames whose node was GC'd, queued against the window whose atlas holds
/// their tiles — see `lottie::queue_drops` for why the key is the window.
fn queue_drops(win: crate::windows::WinKey, images: impl IntoIterator<Item = Arc<RenderImage>>) {
    crate::windows::queue_drops(win, images);
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

/// The transport props, as last read off the tree. Compared to decide whether
/// a prop MOVED — see `sync`.
#[derive(PartialEq)]
struct Props {
    looping: bool,
    muted: bool,
    volume: f64,
    rate: f64,
}

impl Default for Props {
    fn default() -> Self {
        Self { looping: false, muted: false, volume: 1.0, rate: 1.0 }
    }
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
pub(crate) fn sync(t: &Tree, id: u32, win: crate::windows::WinKey, scale: f32) -> Option<(u32, u32)> {
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
            let mut resume = None;
            if let Some(mut old) = map.remove(&id) {
                resume = old.resume.take().filter(|(want, _, _)| *want == src);
                if let Some(state) = old.state.as_mut() {
                    queue_drops(old.window, state.take_all_images());
                }
            }
            map.insert(
                id,
                Entry {
                    src: src.clone(),
                    state: load(&src),
                    autoplay,
                    props: Props::default(),
                    seek_write: None,
                    frame_box: (0, 0),
                    recent: Vec::new(),
                    warned_error: false,
                    resume,
                    window: win,
                    scale,
                },
            );
        }

        let entry = map.get_mut(&id)?;
        // **Re-stamped every frame**, exactly as `lottie::sync` does and for
        // the same reason: a node can move between windows over its life — a
        // `<Teleport>` into a pop-out and back is the demo's own shape — and
        // both of these are facts about the window that is painting it NOW.
        //
        // Recorded only at creation, the zero-copy upload path kept routing
        // decoded frames into the `pending_uploads` of the window that no
        // longer paints the element: the tile was blitted into an atlas nothing
        // would look in, and the `<video>` painted transparent in the window it
        // had moved to. That is the seventh hazard (§7.4) one teleport later.
        // `scale` goes with it because `frame_box` is in PHYSICAL pixels and
        // the two windows can sit on displays of different DPI.
        //
        // The atlas tiles already handed out do NOT need re-homing: a drop is
        // drained from the new window's own `window_frame_tick`, so
        // `App::drop_image(image, Some(new))` can reach every other window
        // through `App::windows` and sweeps the old one's atlas as well.
        entry.window = win;
        entry.scale = scale;
        let state = entry.state.as_mut()?;
        // Being BUILT means being in the painted tree, so this is where a
        // `<KeepAlive>` page coming back out of the cache puts its decoder
        // back (the tick is what took it away — see `attached`).
        state.resume();

        // --- transport: pushed only where the PROP CHANGED ---------------
        //
        // Not "pushed every frame". The tree hands us every prop on every
        // build, so re-applying them unconditionally would overwrite anything
        // set through the element's own API a frame earlier — `el.muted = false`
        // would flip back to `muted` sixty times a second because the ATTRIBUTE
        // still says `muted`. That is not what a browser does: `patchProp` only
        // writes when Vue's diff says the binding moved, and an attribute is a
        // starting value, not a per-frame assertion.
        let props = Props {
            looping: bool_attr(t.prop(id, "loop")),
            muted: bool_attr(t.prop(id, "muted")),
            volume: number_attr(t.prop(id, "volume")).unwrap_or(1.0),
            // The DOM property is `playbackRate`; Vue hyphenates/lowercases an
            // attribute, so both spellings arrive here and both are accepted.
            rate: number_attr(t.prop(id, "playbackrate"))
                .or_else(|| number_attr(t.prop(id, "playback-rate")))
                .unwrap_or(1.0),
        };
        if fresh || props.looping != entry.props.looping {
            state.set_looping(props.looping);
        }
        if fresh || props.muted != entry.props.muted {
            state.set_muted(props.muted);
        }
        if fresh || props.volume != entry.props.volume {
            state.set_volume(props.volume);
        }
        if fresh || props.rate != entry.props.rate {
            state.set_rate(props.rate);
        }
        entry.props = props;

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
    let source = match crate::img::resolve_media("video", src) {
        Ok(source) => source,
        Err(msg) => {
            crate::render::warn_compat(&format!("video-src:{src}"), &msg);
            return None;
        }
    };
    // A missing FILE is worth its own message, because the platform's answer to
    // it ("the source is not supported") names neither the resolved path nor the
    // base it was resolved against. A missing URL is not: the server's status is
    // the useful half there, and only the network source knows it.
    if matches!(&source, MediaSource::Path(p) if !p.exists()) {
        let MediaSource::Path(path) = &source else { unreachable!() };
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
    match state.set_source(source) {
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
                queue_drops(entry.window, state.take_all_images());
            }
            false
        })
    });
}

// ---------------------------------------------------------------------------
// the frame pump
// ---------------------------------------------------------------------------

/// The renderer recreated its D3D11 device (device-lost recovery), so every
/// `<video>` is decoding onto a device that no longer exists. Returns how many
/// elements were rebuilt.
///
/// Called from the host's frame pump when
/// `gpui_windows::renderer_d3d11_device()` reports a new generation. The
/// element does NOT restart: the position and the transport state are carried
/// across and re-applied at `loadedmetadata` (see [`Entry::resume`]), so a
/// TDR while a video plays costs a stutter rather than a rewind.
///
/// The old frames are dropped WITHOUT `Window::drop_image`. That looks like a
/// leak and is the opposite: `DirectXAtlas::handle_device_lost` has already
/// cleared `tiles_by_key`, so the tiles are gone and asking the atlas to free
/// them again would only remove keys that a rebuilt element may already have
/// re-inserted.
pub fn device_changed() -> usize {
    VIDEO_STATES.with(|m| {
        let mut map = m.borrow_mut();
        let mut rebuilt = 0;
        for entry in map.values_mut() {
            let Some(state) = entry.state.as_mut() else { continue };
            entry.resume = Some((
                entry.src.clone(),
                state.current_time(),
                state.wants_playing(),
            ));
            let _dead_on_the_old_device = state.take_all_images();
            entry.state = None;
            // `sync` reloads when the src it built from differs from the live
            // prop; clearing it is what makes the next build rebuild.
            entry.src.clear();
            rebuilt += 1;
        }
        rebuilt
    })
}

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

    // Collect first, dispatch after: a JS handler can mutate the tree (and
    // `flush_changes` re-enters `gc`), so the map must not be borrowed while
    // any of this reaches JavaScript.
    let mut moved = false;
    // The zero-copy present path leaves the decoded frame in VRAM and hands us
    // a BORROWED texture; this is where it becomes an atlas tile. Collected
    // inside the map borrow and uploaded outside it, for the same reason the
    // events are collected and dispatched after — `upload_image_from_gpu` takes
    // `&mut Window`, and the map must not be borrowed while it is held.
    //
    // **Routed to the window that PAINTS the element.** The pump runs with the
    // primary window; blitting a pop-out's frame into the primary window's
    // atlas puts the tile where nothing will look for it and the `<video>`
    // paints transparent. Frames for another window go into that window's
    // `pending_uploads` and are drained by its own draw
    // (`crate::window_frame_tick`), which happens later in the same effect
    // cycle — inside the tick the texture is valid for, as
    // `VideoState::take_gpu_upload` requires.
    let mut uploads: Vec<(u32, Arc<RenderImage>, vn_video::GpuFrame)> = Vec::new();
    let mut elsewhere: Vec<(crate::windows::WinKey, crate::windows::PendingUpload)> = Vec::new();
    let here = crate::windows::key(window);
    let ticks: Vec<(u32, Vec<MediaEvent>)> = VIDEO_STATES.with(|m| {
        let mut map = m.borrow_mut();
        let mut out = Vec::new();
        for (&id, entry) in map.iter_mut() {
            let attached = attached(id);
            let Some(state) = entry.state.as_mut() else { continue };
            // Detached — unmounted, or parked in a `<KeepAlive>` cache
            // container that is never inserted — means stop decoding
            // (decision 3 in the module docs).
            if !attached {
                state.suspend();
                continue;
            }
            // Before the first build the box is unknown; the video's own size
            // is the honest first guess and one frame later the real box wins.
            // The scale is the PAINTING window's, recorded by its own build:
            // two windows can be on displays of different DPI and `frame_box`
            // is in physical pixels (docs/MULTIWINDOW.md §5.3).
            let scale = entry.scale;
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
            if let Some((image, gpu)) = state.take_gpu_upload() {
                if entry.window == here {
                    uploads.push((id, image, gpu));
                } else {
                    elsewhere.push((
                        entry.window,
                        crate::windows::PendingUpload { node: id, image, gpu },
                    ));
                }
            }
            // A rebuild forced by device-lost picks up where it left off. The
            // seek can only land once the new pipeline knows the duration, so
            // it waits for `loadedmetadata` — the same moment a browser's own
            // `currentTime` becomes settable.
            if entry.resume.is_some()
                && advance.events.contains(&MediaEvent::LoadedMetadata)
            {
                if let Some((_, at, playing)) = entry.resume.take() {
                    state.seek(at);
                    state.set_playing(playing);
                    moved = true;
                }
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

    for (win, upload) in elsewhere {
        // At most one frame per element per tick, and a queue that survived a
        // draw is stale by definition (the texture is reused by the next
        // `present`) — so the newest wins rather than piling up.
        crate::windows::of(win, |w| {
            w.pending_uploads.retain(|u| u.node != upload.node);
            w.pending_uploads.push(upload);
        });
    }

    // Before the events, because a JS handler could unmount the node and take
    // its `VideoState` — and therefore the texture — with it.
    for (id, image, gpu) in uploads {
        let ok = window
            .upload_image_from_gpu(&image, 0, gpui::GpuTextureHandle(gpu.texture))
            .unwrap_or(false);
        if !ok {
            // The atlas refused it: a decode on a private device
            // (`VN_VIDEO_DEVICE=private`), a device generation that moved under
            // us, or a renderer with no GPU path. Fall back to the readback,
            // which always works, rather than painting a transparent box.
            crate::render::warn_compat(
                "video-gpu-upload",
                "<video>: the renderer refused the zero-copy GPU blit for a decoded frame — \
                 falling back to the CPU readback path, which costs ~1.2 ms per 800x450 frame. \
                 VN_LOG=info names the reason; VN_VIDEO_DEVICE=private is the usual one.",
            );
            VIDEO_STATES.with(|m| {
                if let Some(state) = m.borrow_mut().get_mut(&id).and_then(|e| e.state.as_mut()) {
                    state.gpu_upload_failed();
                }
            });
        }
    }

    for (id, events) in ticks {
        dispatch_events(id, &events, window, cx);
    }
    moved
}

/// Is this node still part of the painted document?
///
/// **Not "was it built last frame".** gpui only rebuilds the element tree when
/// the host notifies, and the host only notifies when something moved — so a
/// paused or still-loading `<video>` legitimately goes many frames without
/// being built, and inferring "detached" from that suspends the element and
/// tears the pipeline down before it ever produced a frame. (It did. The
/// symptom was `networkState` walking 3 → 0 and no event ever arriving.)
///
/// The tree answers it without a draw: `<KeepAlive>` parks a deactivated page
/// in a container it creates and never inserts, so a parked subtree's parent
/// chain simply does not reach the root.
///
/// `display: none` deliberately does NOT suspend — a browser keeps decoding a
/// hidden video, and so do we.
fn attached(id: u32) -> bool {
    tree::read_tree(|t| {
        let root = t.root;
        let mut cursor = id;
        // Bounded, so a corrupt parent chain cannot hang the pump.
        for _ in 0..4096 {
            if cursor == 0 {
                return false;
            }
            if cursor == root {
                return true;
            }
            match t.node(cursor) {
                Some(n) => cursor = n.parent,
                None => return false,
            }
        }
        false
    })
}

/// Blits every zero-copy frame queued for THIS window into its sprite atlas.
///
/// Called from the pop-out's own draw (`crate::window_frame_tick`), which is
/// the only place that holds the window whose atlas the tile has to land in.
/// A refusal falls back to the CPU readback exactly as the primary window's
/// path does — the element paints a real picture either way.
pub(crate) fn drain_pending_uploads(window: &mut Window) {
    let queued = crate::windows::here(window, |w| std::mem::take(&mut w.pending_uploads));
    for u in queued {
        let ok = window
            .upload_image_from_gpu(&u.image, 0, gpui::GpuTextureHandle(u.gpu.texture))
            .unwrap_or(false);
        if ok {
            continue;
        }
        crate::render::warn_compat(
            "video-gpu-upload-popout",
            "<video> in a pop-out: the renderer refused the zero-copy GPU blit into that \
             window's atlas — falling back to the CPU readback path (docs/MULTIWINDOW.md §7.4).",
        );
        VIDEO_STATES.with(|m| {
            if let Some(state) = m.borrow_mut().get_mut(&u.node).and_then(|e| e.state.as_mut()) {
                state.gpu_upload_failed();
            }
        });
    }
}

/// [`drain_pending_drops`] for a window that is not the one the pump runs
/// with — a pop-out draining its own atlas from its own draw.
pub(crate) fn drain_pending_drops_for(window: &mut Window, cx: &mut App) {
    drain_pending_drops(window, cx);
}

fn drain_pending_drops(window: &mut Window, cx: &mut App) {
    let images = crate::windows::here(window, |w| std::mem::take(&mut w.pending_drops));
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

/// ONE scalar property of [`media_state`], by code — `None` for a node that is
/// not a live media element. Booleans are `0`/`1`.
///
/// The runtime reads every scalar property through this. [`media_state`]
/// answers all of them at once, and on Media Foundation that is about sixteen
/// engine calls, two of them building `TimeRanges` — ~0.2 ms a read, measured.
/// Argon's video custom emoji read `readyState` for each of eight canvases on
/// every animation frame, and those reads alone were most of the client's JS
/// while the window sat idle. One property is one engine call.
///
/// Codes (packages/vue-native/src/protocol.d.ts `VnMediaProp`): 0 currentTime,
/// 1 duration, 2 paused, 3 ended, 4 seeking, 5 readyState, 6 networkState,
/// 7 volume, 8 muted, 9 playbackRate, 10 loop, 11 videoWidth, 12 videoHeight.
pub(crate) fn media_prop(id: u32, code: u32) -> Option<f64> {
    let flag = |b: bool| if b { 1.0 } else { 0.0 };
    with_state(id, |s| match code {
        0 => Some(s.current_time()),
        1 => Some(s.duration()),
        2 => Some(flag(s.paused())),
        3 => Some(flag(s.ended())),
        4 => Some(flag(s.seeking())),
        5 => Some(s.ready_state() as u8 as f64),
        6 => Some(s.network_state() as u8 as f64),
        7 => Some(s.volume()),
        8 => Some(flag(s.muted())),
        9 => Some(s.playback_rate()),
        10 => Some(flag(s.looping())),
        11 => Some(s.native_size().map_or(0.0, |(w, _)| w as f64)),
        12 => Some(s.native_size().map_or(0.0, |(_, h)| h as f64)),
        _ => None,
    })
    .flatten()
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

    /// **A `<video>` that moves between windows is re-stamped with the window
    /// that paints it NOW** (docs/MULTIWINDOW.md §7.4).
    ///
    /// The window and the DPI used to be recorded only when the entry was
    /// created, so a `<Teleport>` into a pop-out and back left the element
    /// answering the window it was born in. The zero-copy upload path then
    /// routed every decoded frame into the `pending_uploads` of a window that
    /// no longer painted it — the tile was blitted into an atlas nothing would
    /// look in, and the `<video>` painted transparent in the window it had
    /// moved to. That is the seventh hazard one teleport later.
    ///
    /// `lottie::sync` has always re-stamped; this is the same line.
    #[test]
    fn a_video_that_changes_window_is_re_stamped() {
        let id = tree::with_tree(|t| {
            let root = t.create_root();
            let video = t.intern("video");
            let node = t.create_element(video);
            t.insert(node, root, 0);
            node
        });
        // An empty `src` loads nothing, which is exactly what this wants: the
        // entry and its bookkeeping exist, the decoder does not.
        let here = |k, scale| tree::read_tree(|t| {
            sync(t, id, k, scale);
        });
        let recorded = || VIDEO_STATES.with(|m| {
            m.borrow().get(&id).map(|e| (e.window, e.scale)).expect("the entry exists")
        });

        here(7u64, 1.0);
        assert_eq!(recorded(), (7u64, 1.0), "the window that created it");

        // Popped out: a different window, on a display of a different DPI.
        here(9u64, 2.0);
        assert_eq!(
            recorded(),
            (9u64, 2.0),
            "the window that paints it now, and ITS scale — `frame_box` is physical px"
        );

        // …and back.
        here(7u64, 1.0);
        assert_eq!(recorded(), (7u64, 1.0), "and back again");

        VIDEO_STATES.with(|m| {
            m.borrow_mut().remove(&id);
        });
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
