//! `<video>` — the element layer over the `sk-video` crate: the per-node `VideoState` side map, the
//! `src` → path resolve, the prop→transport sync, the once-per-frame tick and the event dispatch. The
//! gpui element is built in [`crate::render`] (`build_video`).
//!
//! - `autoplay` is edge-triggered off the prop, as on `<lottie>`.
//! - `currentTime` is written through a write counter, not the value: `el.currentTime = 0` on a video
//!   already at 0 must re-seek, and a prop channel that dedupes on the string cannot see it.
//! - Removing a playing element pauses and suspends it. An audio source can also be played explicitly
//!   while detached.

use std::cell::RefCell;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Instant;

use gpui::{App, RenderImage, Window};
use sk_video::{FrameRequest, MediaError, MediaEvent, MediaSource, VideoOptions, VideoState};

use crate::tree::{self, Tree};
use crate::EventPayload;

#[cfg(feature = "audio")]
pub(crate) mod graph_audio;

struct Entry {
    src: String,
    /// `None` when no backend could be built or the `src` could not be resolved.
    state: Option<VideoState>,
    autoplay: bool,
    props: Props,
    seek_write: Option<u32>,
    frame_box: (u32, u32),
    recent: Vec<&'static str>,
    warned_error: bool,
    resume: Option<(String, f64, bool)>,
    /// Per-window gpui resources ride on it: the atlas a retired frame's tile returns to, and the
    /// atlas a zero-copy GPU frame is blitted into.
    window: crate::windows::WinKey,
    scale: f32,
}

thread_local! {
    /// Audio elements and routed video elements. The flag records an explicit play() while detached.
    static AUDIO_ELEMENTS: RefCell<HashMap<u32, bool>> = RefCell::default();
    static VIDEO_STATES: RefCell<HashMap<u32, Entry>> = RefCell::new(HashMap::new());
    /// Script-set state by node, apart from [`VIDEO_STATES`] because it outlives the decoder ([`Api`]).
    static API: RefCell<HashMap<u32, Api>> = RefCell::new(HashMap::new());
}

pub(crate) fn track_audio(id: u32) {
    AUDIO_ELEMENTS.with(|ids| {
        ids.borrow_mut().entry(id).or_default();
    });
}

fn sync_audio_element(t: &Tree, id: u32) -> Result<(), MediaError> {
    let scale = VIDEO_STATES.with(|m| m.borrow().get(&id).map_or(1.0, |e| e.scale));
    sync(t, id, crate::windows::window_of_node_in(t, id), scale).map(|_| ())
}

fn sync_audio_playback(id: u32, playing: bool) -> Result<(), MediaError> {
    tree::read_tree(|t| {
        let tracked = AUDIO_ELEMENTS.with(|ids| {
            let mut ids = ids.borrow_mut();
            let Some(play_detached) = ids.get_mut(&id) else {
                return false;
            };
            *play_detached = playing && !t.connected(id);
            true
        });
        if playing && tracked {
            sync_audio_element(t, id)?;
        }
        Ok(())
    })
}

pub(crate) fn sync_audio_elements() {
    AUDIO_ELEMENTS.with(|ids| {
        tree::read_tree(|t| {
            for (&id, play_detached) in ids.borrow_mut().iter_mut() {
                if t.connected(id) {
                    *play_detached = false;
                } else if !*play_detached {
                    continue;
                }
                let _ = sync_audio_element(t, id);
            }
        })
    });
}

#[derive(Default, Clone, Copy)]
struct Api {
    // Element state, not resource state: `muted` set in `onMounted` before `src` must hold for the
    // load that follows, and the decoder is rebuilt on a `src` change or a lost GPU device.
    muted: Option<bool>,
    volume: Option<f64>,
    rate: Option<f64>,
    looping: Option<bool>,
    playing: Option<bool>,
}

fn note_api(id: u32, verb: u32, arg: f64, live: bool) {
    API.with(|m| {
        let mut m = m.borrow_mut();
        let api = m.entry(id).or_default();
        match verb {
            0 | 1 if !live => api.playing = Some(verb == 0),
            3 if arg.is_finite() => api.volume = Some(arg.clamp(0.0, 1.0)),
            4 => api.muted = Some(arg != 0.0),
            5 if arg.is_finite() && arg > 0.0 => api.rate = Some(arg),
            6 => api.looping = Some(arg != 0.0),
            _ => {}
        }
    });
}

fn api_of(id: u32, take_playing: bool) -> Api {
    API.with(|m| {
        let mut m = m.borrow_mut();
        let Some(api) = m.get_mut(&id) else {
            return Api::default();
        };
        let out = *api;
        if take_playing {
            api.playing = None;
        }
        out
    })
}

/// An attribute moved after the script's write: the attribute is the newer word, so the script's
/// stops overriding it on the next decoder.
fn attribute_wins(id: u32, f: impl FnOnce(&mut Api)) {
    API.with(|m| {
        if let Some(api) = m.borrow_mut().get_mut(&id) {
            f(api);
        }
    });
}

pub(crate) fn count() -> usize {
    VIDEO_STATES.with(|m| m.borrow().len())
}

pub(crate) fn audible_count() -> usize {
    VIDEO_STATES.with(|m| {
        m.borrow()
            .values()
            .filter_map(|e| e.state.as_ref())
            .filter(|s| s.audible())
            .count()
    })
}

fn queue_drops(win: crate::windows::WinKey, images: impl IntoIterator<Item = Arc<RenderImage>>) {
    crate::windows::queue_drops(win, images);
}

const RING: usize = 32;

/// The browser's boolean-attribute rule: present is true unless it spells `"false"` (`:attr="false"`).
fn bool_attr(raw: Option<&str>) -> bool {
    raw.is_some_and(|v| v.trim() != "false")
}

#[derive(PartialEq)]
struct Props {
    looping: bool,
    muted: bool,
    volume: f64,
    rate: f64,
}

impl Default for Props {
    fn default() -> Self {
        Self {
            looping: false,
            muted: false,
            volume: 1.0,
            rate: 1.0,
        }
    }
}

fn number_attr(raw: Option<&str>) -> Option<f64> {
    raw.and_then(|v| v.trim().parse::<f64>().ok())
}

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
    ("autopictureinpicture", "there is no picture-in-picture"),
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
    // `playsinline` is accepted in silence: we are never fullscreen by default, so the attribute
    // already describes what happens.
}

pub(crate) fn sync(
    t: &Tree,
    id: u32,
    win: crate::windows::WinKey,
    scale: f32,
) -> Result<Option<(u32, u32)>, MediaError> {
    warn_unsupported_props(t, id);
    let src = t.prop(id, "src").unwrap_or("").trim();
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
                    src: src.to_owned(),
                    state: load(id, src)?,
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

        let entry = map.get_mut(&id).expect("entry just synchronized");
        // `scale` goes too: `frame_box` is in physical pixels and the windows can differ in DPI.
        entry.window = win;
        entry.scale = scale;
        let Some(state) = entry.state.as_mut() else {
            return Ok(None);
        };
        state.resume();

        let props = Props {
            looping: bool_attr(t.prop(id, "loop")),
            muted: bool_attr(t.prop(id, "muted")),
            volume: number_attr(t.prop(id, "volume")).unwrap_or(1.0),
            rate: number_attr(t.prop(id, "playbackrate"))
                .or_else(|| number_attr(t.prop(id, "playback-rate")))
                .unwrap_or(1.0),
        };
        if fresh || props.looping != entry.props.looping {
            state.set_looping(props.looping);
            if !fresh {
                attribute_wins(id, |a| a.looping = None);
            }
        }
        if fresh || props.muted != entry.props.muted {
            state.set_muted(props.muted);
            if !fresh {
                attribute_wins(id, |a| a.muted = None);
            }
        }
        if fresh || props.volume != entry.props.volume {
            state.set_volume(props.volume);
            if !fresh {
                attribute_wins(id, |a| a.volume = None);
            }
        }
        if fresh || props.rate != entry.props.rate {
            state.set_rate(props.rate);
            if !fresh {
                attribute_wins(id, |a| a.rate = None);
            }
        }
        entry.props = props;
        // A new decoder starts from what scripts set on the element (`Api`): after the attributes,
        // which are only its starting values, and before `autoplay`, so a muted preview is muted
        // before it plays.
        let api = if fresh {
            api_of(id, true)
        } else {
            Api::default()
        };
        if let Some(looping) = api.looping {
            state.set_looping(looping);
        }
        if let Some(muted) = api.muted {
            state.set_muted(muted);
        }
        if let Some(volume) = api.volume {
            state.set_volume(volume);
        }
        if let Some(rate) = api.rate {
            state.set_rate(rate);
        }

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

        if fresh || autoplay != entry.autoplay {
            state.set_playing(autoplay);
        }
        entry.autoplay = autoplay;
        if let Some(playing) = api.playing {
            state.set_playing(playing);
        }

        Ok(state.native_size())
    })
}

fn load(_id: u32, src: &str) -> Result<Option<VideoState>, MediaError> {
    if src.is_empty() {
        // An absent `src` is not worth an error (a `v-if`'d binding hits it on its first frame).
        return Ok(None);
    }
    let (backend, source) = if src.starts_with(sk_video::stream::PREFIX) {
        (
            sk_video::stream::backend(src),
            MediaSource::Uri(src.to_owned()),
        )
    } else {
        let Some(source) = resolve_source(src) else {
            return Ok(None);
        };
        (sk_video::backend_for_this_platform(), source)
    };
    let Ok(backend) = backend.inspect_err(|error| report(src, error)) else {
        return Ok(None);
    };
    let mut state = VideoState::new(backend);
    // Loading may open the system audio device. Restore the graph before the decoder opens.
    #[cfg(feature = "audio")]
    graph_audio::restore(_id, &mut state).inspect_err(|error| report(src, error))?;
    Ok(state
        .set_source(source)
        .inspect_err(|error| report(src, error))
        .ok()
        .map(|_| state))
}

fn resolve_source(src: &str) -> Option<MediaSource> {
    let source = match crate::img::resolve_media("video", src) {
        Ok(source) => source,
        Err(msg) => {
            crate::render::warn_compat(&format!("video-src:{src}"), &msg);
            return None;
        }
    };
    // A missing URL is not: the server's status is the useful half there, and only the network
    // source knows it.
    if matches!(&source, MediaSource::Path(p) if !p.exists()) {
        let MediaSource::Path(path) = &source else {
            unreachable!()
        };
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

    Some(source)
}

fn report(src: &str, error: &MediaError) {
    let text = match &error.advisory {
        Some(advisory) => advisory.clone(),
        None => format!("<video src=\"{src}\">: {}", error.message),
    };
    crate::render::warn_compat(&format!("video-error:{src}:{:?}", error.code), &text);
}

pub(crate) fn with_state<R>(id: u32, f: impl FnOnce(&mut VideoState) -> R) -> Option<R> {
    VIDEO_STATES.with(|m| m.borrow_mut().get_mut(&id)?.state.as_mut().map(f))
}

pub(crate) fn note_frame_box(id: u32, options: &VideoOptions) {
    let want = options.max_frame_size();
    VIDEO_STATES.with(|m| {
        if let Some(entry) = m.borrow_mut().get_mut(&id) {
            entry.frame_box = want;
        }
    });
}

pub(crate) fn gc(live: impl Fn(u32) -> bool) {
    AUDIO_ELEMENTS.with(|ids| ids.borrow_mut().retain(|&id, _| live(id)));
    #[cfg(feature = "audio")]
    graph_audio::gc(&live);
    API.with(|m| m.borrow_mut().retain(|&id, _| live(id)));
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

pub fn device_changed() -> usize {
    VIDEO_STATES.with(|m| {
        let mut map = m.borrow_mut();
        let mut rebuilt = 0;
        for entry in map.values_mut() {
            let Some(state) = entry.state.as_mut() else {
                continue;
            };
            entry.resume = Some((
                entry.src.clone(),
                state.current_time(),
                state.wants_playing(),
            ));
            let _dead_on_the_old_device = state.take_all_images();
            entry.state = None;
            entry.src.clear();
            rebuilt += 1;
        }
        rebuilt
    })
}

pub fn any_playing() -> bool {
    VIDEO_STATES.with(|m| {
        m.borrow()
            .values()
            .filter_map(|e| e.state.as_ref())
            .any(|s| s.needs_frames())
    })
}

fn on_screen(id: u32, win: crate::windows::WinKey) -> bool {
    crate::live::visible(id, win)
}

pub fn tick(window: &mut Window, cx: &mut App) -> bool {
    drain_pending_drops(window, cx);

    let now = Instant::now();

    // Collect first, dispatch after: a JS handler can mutate the tree (and `flush_changes`
    // re-enters `gc`), so the map must not be borrowed while any of this reaches JavaScript.
    let mut moved = false;
    // The zero-copy path leaves the frame in VRAM as a BORROWED texture; it becomes an atlas tile
    // here, uploaded outside the map borrow (`&mut Window`) in the window that PAINTS the element.
    let mut uploads: Vec<(u32, Arc<RenderImage>, sk_video::GpuFrame, bool)> = Vec::new();
    let mut elsewhere: Vec<(crate::windows::WinKey, crate::windows::PendingUpload)> = Vec::new();
    let here = crate::windows::key(window);
    let ticks: Vec<(u32, Vec<MediaEvent>)> = VIDEO_STATES.with(|m| {
        let mut map = m.borrow_mut();
        let mut out = Vec::new();
        for (&id, entry) in map.iter_mut() {
            let attached = tree::read_tree(|t| t.connected(id));
            let Some(state) = entry.state.as_mut() else {
                continue;
            };
            // Detached (unmounted, or parked in a never-inserted `<KeepAlive>` container): stop decoding.
            if !attached && !AUDIO_ELEMENTS.with(|m| m.borrow().get(&id).copied().unwrap_or(false))
            {
                if state.wants_playing() && !state.suspended() {
                    state.pause();
                    let removed = [MediaEvent::TimeUpdate, MediaEvent::Pause];
                    for event in removed {
                        if entry.recent.len() >= RING {
                            entry.recent.remove(0);
                        }
                        entry.recent.push(event.name());
                    }
                    out.push((id, removed.to_vec()));
                }
                state.suspend();
                continue;
            }
            let scale = entry.scale;
            let max_size = if entry.frame_box == (0, 0) {
                state
                    .native_size()
                    .map(|(w, h)| ((w as f32 * scale) as u32, (h as f32 * scale) as u32))
                    .unwrap_or((0, 0))
            } else {
                entry.frame_box
            };
            let advance = state.advance(now, &FrameRequest { max_size });
            let show = advance.new_frame && on_screen(id, entry.window);
            match state.take_gpu_upload() {
                Some((image, gpu)) if entry.window == here => uploads.push((id, image, gpu, show)),
                Some((image, gpu)) => {
                    moved |= show;
                    elsewhere.push((
                        entry.window,
                        crate::windows::PendingUpload {
                            node: id,
                            image,
                            gpu: sk_video::HeldGpuFrame::new(gpu),
                        },
                    ));
                }
                None => moved |= show,
            }
            // The seek waits for `loadedmetadata`, when a browser's `currentTime` becomes settable too.
            if entry.resume.is_some() && advance.events.contains(&MediaEvent::LoadedMetadata) {
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
        // At most one frame per element per tick; a queue that survived a draw is stale (the texture
        // is reused), so the newest wins.
        crate::windows::of(win, |w| {
            w.pending_uploads.retain(|u| u.node != upload.node);
            w.pending_uploads.push(upload);
        });
    }

    // Before the events, because a JS handler could unmount the node and take its `VideoState` —
    // and therefore the texture — with it.
    for (id, image, gpu, show) in uploads {
        let write = window
            .write_image_tile(
                &image,
                0,
                gpui::TileSource::Gpu(gpui::GpuTextureHandle(gpu.texture)),
            )
            .unwrap_or(gpui::TileWrite::Refused);
        crate::live::note(crate::live::shown(write, show));
        if write == gpui::TileWrite::Refused {
            crate::render::warn_compat(
                "video-gpu-upload",
                "<video>: the renderer refused the zero-copy GPU blit for a decoded frame — \
                 falling back to the CPU readback path, which costs ~1.2 ms per 800x450 frame. \
                 SK_LOG=info names the reason; SK_VIDEO_DEVICE=private is the usual one.",
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

pub(crate) fn drain_pending_uploads(window: &mut Window) {
    let queued = crate::windows::here(window, |w| std::mem::take(&mut w.pending_uploads));
    for u in queued {
        let alive =
            VIDEO_STATES.with(|m| m.borrow().get(&u.node).is_some_and(|e| e.state.is_some()));
        if !alive {
            continue;
        }
        let ok = window
            .upload_image_from_gpu(&u.image, 0, gpui::GpuTextureHandle(u.gpu.frame().texture))
            .unwrap_or(false);
        if ok {
            continue;
        }
        crate::render::warn_compat(
            "video-gpu-upload-popout",
            "<video> in a pop-out: the renderer refused the zero-copy GPU blit into that \
             window's atlas — falling back to the CPU readback path.",
        );
        VIDEO_STATES.with(|m| {
            if let Some(state) = m
                .borrow_mut()
                .get_mut(&u.node)
                .and_then(|e| e.state.as_mut())
            {
                state.gpu_upload_failed();
            }
        });
    }
}

/// [`drain_pending_drops`] for a window other than the pump's (a pop-out draining its own atlas).
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

/// `HTMLMediaElement`'s readable properties as `name=value` lines, the shape `dom::computed_style` uses.
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
        // Not a DOM property: whether the platform plays this element's sound.
        put("audible", s.audible().to_string());
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

pub(crate) fn media_prop(id: u32, code: u32) -> Option<f64> {
    let flag = |b: bool| if b { 1.0 } else { 0.0 };
    if with_state(id, |_| ()).is_none() {
        let api = api_of(id, false);
        let attr = |name: &str| tree::read_tree(|t| t.prop(id, name).map(|v| bool_attr(Some(v))));
        return match code {
            2 => api.playing.map(|p| flag(!p)),
            7 => api.volume,
            8 => api.muted.or_else(|| attr("muted")).map(flag),
            9 => api.rate,
            10 => api.looping.or_else(|| attr("loop")).map(flag),
            _ => None,
        };
    }
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

pub(crate) fn media_call(id: u32, verb: u32, arg: f64) -> bool {
    if matches!(verb, 0 | 1) && sync_audio_playback(id, verb == 0).is_err() {
        return false;
    }
    let live = VIDEO_STATES.with(|m| {
        m.borrow().get(&id).is_some_and(|entry| {
            entry.state.is_some()
                && tree::read_tree(|t| t.prop(id, "src").unwrap_or("").trim() == entry.src)
        })
    });
    // The element's own state whether or not a decoder exists yet (`Api`).
    note_api(id, verb, arg, live);
    if !live {
        return matches!(verb, 0 | 1 | 3 | 4 | 5 | 6);
    }
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

/// `canPlayType(type)`, from a live probe of this machine, never a static table.
pub(crate) fn can_play_type(id: u32, mime_type: &str) -> String {
    with_state(id, |s| s.can_play_type(mime_type).as_str().to_string()).unwrap_or_default()
}

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
    fn element_api_writes_before_the_decoder_are_kept_for_it() {
        let id = 0x7fff_0001;
        assert!(
            media_call(id, 4, 1.0),
            "muted = true is the element's, decoder or not"
        );
        assert!(media_call(id, 3, 0.25));
        assert!(
            media_call(id, 0, 0.0),
            "play() before there is anything to play"
        );
        assert!(!media_call(id, 2, 5.0), "a seek has nothing to seek yet");
        let api = api_of(id, false);
        assert_eq!(api.muted, Some(true));
        assert_eq!(api.volume, Some(0.25));
        assert_eq!(api.playing, Some(true));
        assert_eq!(media_prop(id, 8), Some(1.0));
        assert_eq!(media_prop(id, 7), Some(0.25));
        assert_eq!(media_prop(id, 2), Some(0.0), "paused is false after play()");
        let taken = api_of(id, true);
        assert_eq!(taken.playing, Some(true));
        assert_eq!(api_of(id, false).playing, None);
        assert_eq!(api_of(id, false).muted, Some(true));
        attribute_wins(id, |a| a.muted = None);
        assert_eq!(api_of(id, false).muted, None);
        gc(|n| n != id);
        assert!(API.with(|m| !m.borrow().contains_key(&id)));
    }

    #[test]
    fn a_video_that_changes_window_is_re_stamped() {
        let id = tree::with_tree(|t| {
            let root = t.create_root();
            let video = t.intern("video");
            let node = t.create_element(video);
            t.insert(node, root, 0);
            node
        });
        let here = |k, scale| {
            tree::read_tree(|t| {
                sync(t, id, k, scale).unwrap();
            })
        };
        let recorded = || {
            VIDEO_STATES.with(|m| {
                m.borrow()
                    .get(&id)
                    .map(|e| (e.window, e.scale))
                    .expect("the entry exists")
            })
        };

        here(7u64, 1.0);
        assert_eq!(recorded(), (7u64, 1.0), "the window that created it");

        here(9u64, 2.0);
        assert_eq!(
            recorded(),
            (9u64, 2.0),
            "the window that paints it now, and ITS scale — `frame_box` is physical px"
        );

        here(7u64, 1.0);
        assert_eq!(recorded(), (7u64, 1.0), "and back again");

        VIDEO_STATES.with(|m| {
            m.borrow_mut().remove(&id);
        });
    }

    /// Every `MediaEvent` needs a unique code: a duplicate would deliver one event under another's name.
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
        assert!(codes.iter().all(|&c| c < u64::BITS));
    }
}
