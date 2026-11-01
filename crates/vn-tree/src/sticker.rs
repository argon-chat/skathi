//! `<vn-sticker>`: Argon's stickers and animated emoji, Lottie (`.tgs`) and
//! VP9-alpha WebM, many on screen at once. The `vn-sticker` crate draws the
//! frames off the UI thread; this module is the element: the box, the fetch,
//! visibility, the [`LiveTile`]s and the events.
//!
//! vue-native only. The browser never sees the tag: the client renders it
//! under `if (isVn)` and keeps its own players for the browser.
//!
//! ```html
//! <vn-sticker src="https://…/files/{id}" kind="auto|lottie|video|image"
//!             tint play="auto|on|off" once max-fps="30" />
//! ```
//!
//! * `src`: the file. http(s) is fetched on gpui's HTTP client off the UI
//!   thread; `data:`, `blob:`, packaged and file paths read as `<lottie src>`
//!   does.
//! * `kind`: `image` paints a still picture the way `<img>` does (a tinted
//!   static emoji). Anything else is sniffed from the bytes: Lottie JSON, a
//!   gzipped `.tgs`, or a WebM.
//! * `tint`: paints the frame's alpha in the element's `color`, as the
//!   browser's `source-atop` fill does.
//! * `play`: `on` plays while a copy can be seen, `off` shows the first frame,
//!   and `auto` (the default) is `on` unless the OS asks for reduced motion.
//!   The client applies its own policy (groups, power saving) through `on` and
//!   `off`.
//! * `once`: one loop from when the copy is first seen, then the last frame
//!   holds and `ended` fires. Without it a sticker loops on its source's clock,
//!   which every copy of that source shares.
//! * `max-fps`: a cap below the file's own rate. Copies at 64 device px or
//!   less play at 30 fps at most anyway (the owner's budget).
//! * The box: CSS sizes it; an axis CSS leaves `auto` takes the file's
//!   intrinsic size, through its aspect ratio when the other axis is set.
//!   `object-fit` places the picture: `contain` (the default), `cover` or
//!   `fill`. `none` and `scale-down` draw as `contain`, with an advisory.
//! * Events: `load` when this copy first shows a frame (`a`/`b` = the file's
//!   intrinsic size, `key` = the src), `error` when the file cannot be fetched
//!   or played (`key` = the src, the reason goes to the compat log), and
//!   `ended` after a `once` loop. No per-frame events.
//!
//! # From a file to the screen
//!
//! 1. The build ([`build`]) records the node's attributes and paints the tile
//!    the node shows, or nothing yet. It never writes a frame.
//! 2. The tick ([`tick`], from the pump, before the draw) asks the engine where
//!    each source is, fetches the ones that need bytes, and works out each
//!    node's target: its source at its box's size in device px, rounded up to
//!    a 4, 8 or 16 px step so a box that moves by a pixel keeps its target.
//!    Every copy at one size shares a target, and every target in one window
//!    shares a tile.
//! 3. A node is visible when [`live::visible`] says so: painted in the last
//!    drawn frame, inside the viewport and not clipped away by an ancestor.
//!    Targets with a visible copy play; the others hold their frame. A target
//!    nothing has shown for [`TILE_IDLE`] is no longer asked for, and its
//!    tiles go back to their atlases.
//! 4. A frame with a new sequence number is written into its tile. Written
//!    where the last drawn scene samples it, it costs a present and no build
//!    (`crate::live`). A tile's first frame, a frame that had to move in the
//!    atlas, and a node switching tiles need a draw, and touch the node, so a
//!    cache boundary over it renders again instead of replaying a sprite that
//!    points at the old place. That is why `<vn-sticker>` is not one of
//!    `crate::cache`'s exclusions: a message with animated emoji stays
//!    cacheable.
//! 5. A node keeps painting the tile it shows until the tile of the target it
//!    wants has a frame, so a resize never flashes empty.
//!
//! A pop-out's tiles live in its own atlas and are written by its own draw
//! ([`window_tick`], from `crate::window_frame_tick`); the pump's tick only
//! asks for that draw.

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use gpui::prelude::*;
use gpui::{div, AnyElement, App, Context, ImageSource, RenderImage, Window, WindowControlArea};
use vn_sticker::{Frame, Kind, SourceInfo, SourceStatus, TargetKey};

use super::{
    apply_computed_style, bind_events, focus_handle_for, is_sized, pointer_mask, presentational_size, tab_index_of,
    warn_compat, watch_focus, with_control_area,
};
use crate::live::{self, LiveTile, Shown};
use crate::tree::{self, Tree};
use crate::windows::{self, WinKey};
use crate::EventPayload;

/// A tile no visible copy has painted for this long goes back to its atlas,
/// and its target stops being asked for (the owner's budget).
pub(crate) const TILE_IDLE: Duration = Duration::from_secs(3);

/// The longest side a Lottie is drawn at, in device px. A video is never
/// decoded above its own size; the GPU scales both.
const MAX_LOTTIE_PX: u32 = 1024;

// ---------------------------------------------------------------------------
// attributes
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Play {
    Auto,
    On,
    Off,
}

/// `object-fit`, as far as a sticker takes it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Fit {
    Contain,
    Cover,
    Fill,
}

impl Fit {
    fn gpui(self) -> gpui::ObjectFit {
        match self {
            Fit::Contain => gpui::ObjectFit::Contain,
            Fit::Cover => gpui::ObjectFit::Cover,
            Fit::Fill => gpui::ObjectFit::Fill,
        }
    }
}

/// What the build read off the node.
#[derive(Clone, Debug, PartialEq)]
struct Props {
    src: Arc<str>,
    /// `kind="image"`: a still picture, painted as `<img>` paints it.
    image: bool,
    tint: bool,
    play: Play,
    once: bool,
    max_fps: Option<u16>,
    fit: Fit,
}

/// The browser's boolean-attribute rule: present is true unless it spells
/// `"false"` (what `:attr="false"` serialises to).
fn bool_attr(raw: Option<&str>) -> bool {
    raw.is_some_and(|v| v.trim() != "false")
}

fn play_from(raw: Option<&str>) -> Result<Play, String> {
    match raw.map(|p| p.trim().to_ascii_lowercase()).as_deref() {
        None | Some("") | Some("auto") => Ok(Play::Auto),
        Some("on") | Some("true") => Ok(Play::On),
        Some("off") | Some("false") => Ok(Play::Off),
        Some(other) => Err(other.to_string()),
    }
}

/// `max-fps`: a positive number of frames a second; anything else is no cap.
fn max_fps_from(raw: Option<&str>) -> Option<u16> {
    let fps = raw?.trim().parse::<f32>().ok()?;
    (fps >= 1.0).then(|| fps.min(1000.0) as u16)
}

fn props_of(t: &Tree, id: u32, node: &crate::tree::Node, prev: Option<&Arc<str>>) -> Props {
    let src = t.prop(id, "src").map(str::trim).unwrap_or("");
    // One allocation per src, not per build.
    let src = match prev {
        Some(p) if &**p == src => p.clone(),
        _ => Arc::from(src),
    };
    let kind = t.prop(id, "kind").map(|k| k.trim().to_ascii_lowercase());
    if let Some(k) = kind.as_deref().filter(|k| !matches!(*k, "" | "auto" | "lottie" | "video" | "image")) {
        warn_compat(
            &format!("vn-sticker-kind:{k}"),
            &format!("<vn-sticker kind=\"{k}\"> is not a kind: use auto, lottie, video or image. It plays as auto"),
        );
    }
    let play = play_from(t.prop(id, "play")).unwrap_or_else(|other| {
        warn_compat(
            &format!("vn-sticker-play:{other}"),
            &format!("<vn-sticker play=\"{other}\"> is not a play mode: use on, off or auto. It plays as auto"),
        );
        Play::Auto
    });
    let max_fps = max_fps_from(t.prop(id, "max-fps").or_else(|| t.prop(id, "maxFps")).or_else(|| t.prop(id, "maxfps")));
    let fit = match node.computed.as_ref().and_then(|cs| cs.extras.object_fit) {
        Some(vn_css::ObjectFit::Cover) => Fit::Cover,
        Some(vn_css::ObjectFit::Fill) => Fit::Fill,
        Some(other @ (vn_css::ObjectFit::None | vn_css::ObjectFit::ScaleDown)) => {
            warn_compat(
                "vn-sticker-object-fit",
                &format!(
                    "<vn-sticker> draws `object-fit: {}` as `contain`: a sticker is drawn at the size it \
                     is shown, so it has no natural pixel size to keep",
                    if other == vn_css::ObjectFit::None { "none" } else { "scale-down" }
                ),
            );
            Fit::Contain
        }
        _ => Fit::Contain,
    };
    if t.prop(id, "tone").is_some() {
        warn_compat(
            "vn-sticker-tone",
            "<vn-sticker tone> is not supported yet: the sticker plays in its own colours",
        );
    }
    Props {
        src,
        image: kind.as_deref() == Some("image"),
        tint: bool_attr(t.prop(id, "tint")),
        play,
        once: bool_attr(t.prop(id, "once")),
        max_fps,
        fit,
    }
}

// ---------------------------------------------------------------------------
// targets
// ---------------------------------------------------------------------------

/// The size a source is drawn at for a box of `boxed` device px: the picture's
/// size under `fit`, its longest side rounded up to a 4 px step (8 above 128,
/// 16 above 256), never above a video's own size or [`MAX_LOTTIE_PX`]. `None`
/// for an empty box or source.
fn target_size(intrinsic: (u32, u32), boxed: (f32, f32), fit: Fit, kind: Kind) -> Option<(u32, u32)> {
    let (iw, ih) = (intrinsic.0 as f32, intrinsic.1 as f32);
    let (bw, bh) = boxed;
    if iw < 1.0 || ih < 1.0 || !(bw >= 0.5 && bh >= 0.5) {
        return None;
    }
    let (sx, sy) = (bw / iw, bh / ih);
    // `fill` stretches the picture; drawn at the larger of the two scales,
    // neither axis is magnified.
    let scale = match fit {
        Fit::Contain => sx.min(sy),
        Fit::Cover | Fit::Fill => sx.max(sy),
    };
    let own = iw.max(ih);
    let cap = match kind {
        Kind::Video => own as u32,
        Kind::Lottie => MAX_LOTTIE_PX,
    };
    let long = ((own * scale).ceil() as u32).max(1);
    let step = match long {
        0..=128 => 4,
        129..=256 => 8,
        _ => 16,
    };
    let long = long.div_ceil(step).saturating_mul(step).min(cap).max(1);
    let short = ((iw.min(ih) / own) * long as f32).round().max(1.0) as u32;
    Some(if iw >= ih { (long, short) } else { (short, long) })
}

/// Node `id`'s target: its source drawn for a box of `boxed` device px.
fn key_for(id: u32, props: &Props, info: &SourceInfo, boxed: (f32, f32), reduce_motion: bool) -> Option<TargetKey> {
    let (width, height) = target_size((info.width, info.height), boxed, props.fit, info.kind)?;
    let mut key = TargetKey::new(props.src.clone(), width, height);
    // A tinted video needs only its alpha; a Lottie is tinted when painted.
    key.alpha_only = props.tint && info.kind == Kind::Video;
    key.max_fps = props.max_fps;
    key.still = info.frames <= 1
        || match props.play {
            Play::On => false,
            Play::Off => true,
            Play::Auto => reduce_motion,
        };
    key.once = (props.once && !key.still).then_some(id);
    Some(key)
}

// ---------------------------------------------------------------------------
// state
// ---------------------------------------------------------------------------

/// One `<vn-sticker>` node.
struct Entry {
    props: Props,
    /// The window that built the node last.
    win: WinKey,
    /// Both box axes come from CSS. When one does not, the box takes the
    /// source's size, and the build runs again once that is known.
    sized: bool,
    info: Option<SourceInfo>,
    /// What the node asks for now.
    want: Option<TargetKey>,
    /// The tile the build paints: `want`'s, once that has a frame.
    shows: Option<(TargetKey, WinKey)>,
    /// A place of it can be seen now.
    visible: bool,
    /// The last tick it could be seen.
    seen: Option<Instant>,
    loaded: bool,
    failed: bool,
    ended: bool,
}

impl Entry {
    fn new(props: Props, win: WinKey) -> Entry {
        Entry {
            props,
            win,
            sized: false,
            info: None,
            want: None,
            shows: None,
            visible: false,
            seen: None,
            loaded: false,
            failed: false,
            ended: false,
        }
    }

    /// Seen within [`TILE_IDLE`]: its target is asked for, its tiles kept.
    fn recent(&self, now: Instant) -> bool {
        self.seen.is_some_and(|s| now.duration_since(s) < TILE_IDLE)
    }
}

/// One target's picture in one window's atlas.
struct Tile {
    live: LiveTile,
    /// The frame written last; 0 before the first.
    seq: u64,
}

thread_local! {
    static NODES: RefCell<HashMap<u32, Entry>> = RefCell::new(HashMap::new());
    static TILES: RefCell<HashMap<(TargetKey, WinKey), Tile>> = RefCell::new(HashMap::new());
}

/// Sources being fetched, so a source is asked for once. Shared with the
/// fetch tasks, which take theirs out when they are done.
fn fetching() -> &'static Mutex<HashSet<Arc<str>>> {
    static FETCHING: OnceLock<Mutex<HashSet<Arc<str>>>> = OnceLock::new();
    FETCHING.get_or_init(|| Mutex::new(HashSet::new()))
}

/// `(nodes, tiles, tile bytes)` for devtools `mem`. A tile's bytes count
/// twice: the atlas copy and the CPU copy of the last frame `LiveTile`
/// keeps.
pub(crate) fn mem_stats() -> (usize, usize, usize) {
    let nodes = NODES.with(|m| m.borrow().len());
    TILES.with(|m| {
        let m = m.borrow();
        let bytes = m.keys().map(|(k, _)| k.width as usize * k.height as usize * 4 * 2).sum();
        (nodes, m.len(), bytes)
    })
}

/// Forgets nodes that no longer exist. Their tiles go back on the next tick.
pub(crate) fn gc(live: impl Fn(u32) -> bool) {
    NODES.with(|m| m.borrow_mut().retain(|&id, _| live(id)));
}

/// A window closed: its atlas went with it, so its tiles are simply dropped.
pub(crate) fn forget_window(key: WinKey) {
    TILES.with(|m| m.borrow_mut().retain(|(_, w), _| *w != key));
    NODES.with(|m| {
        for e in m.borrow_mut().values_mut() {
            if e.shows.as_ref().is_some_and(|(_, w)| *w == key) {
                e.shows = None;
            }
        }
    });
}

// ---------------------------------------------------------------------------
// the build
// ---------------------------------------------------------------------------

/// What a node paints.
enum Picture {
    Nothing,
    Tile(Arc<RenderImage>),
    Image(ImageSource),
}

/// Records what the build read, and answers what to paint and, for a source
/// already known, its intrinsic size.
fn note_build(id: u32, win: WinKey, t: &Tree, node: &crate::tree::Node, sized: bool) -> (Picture, Option<(u32, u32)>, Fit) {
    NODES.with(|m| {
        let mut m = m.borrow_mut();
        let prev = m.get(&id).map(|e| e.props.src.clone());
        let props = props_of(t, id, node, prev.as_ref());
        let e = m.entry(id).or_insert_with(|| Entry::new(props.clone(), win));
        if e.props.src != props.src || e.props.image != props.image {
            // Another picture: nothing shown until it has a frame, and its own
            // `load` / `error`.
            *e = Entry::new(props.clone(), win);
        }
        e.props = props;
        e.win = win;
        e.sized = sized;
        let fit = e.props.fit;
        if e.props.image {
            let picture = crate::img::source_for(id, &e.props.src).map_or(Picture::Nothing, Picture::Image);
            return (picture, None, fit);
        }
        let picture = e
            .shows
            .as_ref()
            .filter(|(_, w)| *w == win)
            .and_then(|k| TILES.with(|t| t.borrow().get(k).filter(|t| t.seq != 0).map(|t| t.live.image())))
            .map_or(Picture::Nothing, Picture::Tile);
        (picture, e.info.as_ref().map(|i| (i.width, i.height)), fit)
    })
}

/// gpui `Img`'s sizing rule for a box holding a picture of `iw` × `ih`: an
/// `auto` axis takes the intrinsic size, or follows the other axis's absolute
/// length through the aspect ratio.
fn size_as_replaced(style: &mut gpui::StyleRefinement, iw: f32, ih: f32, rem: gpui::Pixels) {
    use gpui::{DefiniteLength, Length};
    let auto = |l: Option<Length>| matches!(l, None | Some(Length::Auto));
    let absolute = |l: Option<Length>| match l {
        Some(Length::Definite(DefiniteLength::Absolute(a))) => Some(f32::from(a.to_pixels(rem))),
        _ => None,
    };
    if iw <= 0.0 || ih <= 0.0 {
        return;
    }
    if style.aspect_ratio.is_none() {
        style.aspect_ratio = Some(iw / ih);
    }
    if auto(style.size.width) {
        let w = absolute(style.size.height).map_or(iw, |h| iw * h / ih);
        style.size.width = Some(gpui::px(w).into());
    }
    if auto(style.size.height) {
        let h = absolute(style.size.width).map_or(ih, |w| ih * w / iw);
        style.size.height = Some(gpui::px(h).into());
    }
}

/// `<vn-sticker>`: a styled box, as a `<div>` is, with the tile the node shows
/// painted over its content box. Not focusable without `tabindex`, as `<img>`.
pub(super) fn build<V: 'static>(
    t: &Tree,
    id: u32,
    node: &crate::tree::Node,
    window: &mut Window,
    cx: &mut Context<V>,
    now: Instant,
    control_area: Option<WindowControlArea>,
) -> AnyElement {
    let win = windows::key(window);
    let mut d = with_control_area(div().id(id as usize), id, control_area, win);
    d = crate::aria::apply(d, t, id, node);
    if let Some(cs) = &node.computed {
        d = apply_computed_style(d, cs, id, now);
    }
    presentational_size(t, id, d.style());
    let sized = {
        let size = &d.style().size;
        is_sized(size.width) && is_sized(size.height)
    };
    let (picture, intrinsic, fit) = note_build(id, win, t, node, sized);
    if let Some((iw, ih)) = intrinsic {
        size_as_replaced(d.style(), iw as f32, ih as f32, window.rem_size());
    }
    let picture = match picture {
        Picture::Nothing => None,
        Picture::Tile(image) => Some(gpui::img(ImageSource::Render(image))),
        Picture::Image(source) => Some(gpui::img(source)),
    };
    if let Some(picture) = picture {
        let picture = picture.object_fit(fit.gpui()).size_full().into_any_element();
        let picture = super::clip_replaced(picture, d.style(), window.rem_size());
        d = d.child(picture);
    }

    let mask = pointer_mask(node);
    let bit = |ev: u32| mask & (1u64 << ev) != 0;
    let tab_index = tab_index_of(t, id);
    let focusable =
        tab_index.is_some() || bit(tree::EV_KEYDOWN) || bit(tree::EV_KEYUP) || bit(tree::EV_FOCUS) || bit(tree::EV_BLUR);
    let styled_focus = node.computed.as_ref().is_some_and(|cs| cs.needs_focus_handle());
    let handle = (focusable || styled_focus).then(|| focus_handle_for(id, tab_index, cx));
    if let Some(h) = &handle {
        d = d.track_focus(h);
        if focusable {
            watch_focus(id, h, window, cx);
        }
    }
    let click_focus = handle.filter(|_| focusable);
    bind_events(d, id, mask, click_focus, focusable, true, cx).into_any_element()
}

// ---------------------------------------------------------------------------
// the tick
// ---------------------------------------------------------------------------

enum Event {
    Load { id: u32, src: Arc<str>, size: (f64, f64) },
    Error { id: u32, src: Arc<str>, why: Option<Arc<str>> },
    Ended { id: u32 },
}

/// The fetch of `src`, once: http(s) on gpui's HTTP client on the background
/// executor, anything else read here (data:, blob:, the package, a file).
fn fetch(src: &Arc<str>, cx: &mut App) {
    if !fetching().lock().unwrap_or_else(|p| p.into_inner()).insert(src.clone()) {
        return;
    }
    let stickers = vn_sticker::global();
    let scheme = src.split_once(':').map(|(s, _)| s.to_ascii_lowercase()).unwrap_or_default();
    if matches!(scheme.as_str(), "http" | "https") {
        let client = cx.http_client();
        let src = src.clone();
        cx.background_executor()
            .spawn(async move {
                match crate::fonts::http_bytes(client, &src).await {
                    Ok(bytes) => stickers.provide(&src, bytes.into()),
                    Err(why) => stickers.fail(&src, &format!("could not be fetched: {why}")),
                }
                fetching().lock().unwrap_or_else(|p| p.into_inner()).remove(&src);
            })
            .detach();
        return;
    }
    match crate::img::read_bytes("vn-sticker", src) {
        Ok(bytes) => stickers.provide(src, bytes.into()),
        // The element and src are put in front when it is reported.
        Err(why) => {
            let prefix = format!("<vn-sticker src=\"{src}\">: ");
            stickers.fail(src, why.strip_prefix(&prefix).unwrap_or(&why));
        }
    }
    fetching().lock().unwrap_or_else(|p| p.into_inner()).remove(src);
}

/// Every `<vn-sticker>`, once a frame from the pump, before the draw: sources
/// fetched, targets asked for, this window's tiles written, idle tiles
/// retired, events dispatched. `true` when a pop-out has to draw (its tiles
/// are written by its own draw) or a box waits for its source's size.
pub(crate) fn tick(window: &mut Window, cx: &mut App) -> bool {
    if NODES.with(|m| m.borrow().is_empty()) && TILES.with(|m| m.borrow().is_empty()) {
        return false;
    }
    let here = windows::key(window);
    let now = Instant::now();
    let page_hidden = live::page_hidden();
    let here_shown = window.is_visible();
    let stickers = vn_sticker::global();
    let mut draw = false;
    let mut touch: Vec<u32> = Vec::new();
    let mut events: Vec<Event> = Vec::new();
    let mut fetches: Vec<Arc<str>> = Vec::new();
    let mut images: Vec<(u32, Arc<str>)> = Vec::new();

    // Each node: where its source is, whether it can be seen, what it wants.
    let mut statuses: HashMap<Arc<str>, SourceStatus> = HashMap::new();
    NODES.with(|m| {
        for (&id, e) in m.borrow_mut().iter_mut() {
            e.visible = !page_hidden && (e.win != here || here_shown) && live::visible(id, e.win);
            if e.visible {
                e.seen = Some(now);
            }
            if e.props.src.is_empty() {
                continue;
            }
            if e.props.image {
                if !e.loaded && !e.failed {
                    images.push((id, e.props.src.clone()));
                }
                continue;
            }
            let status = statuses.entry(e.props.src.clone()).or_insert_with_key(|src| stickers.status(src));
            match status {
                SourceStatus::NeedsBytes => fetches.push(e.props.src.clone()),
                SourceStatus::Loading => {}
                SourceStatus::Failed(why) => {
                    if !e.failed {
                        e.failed = true;
                        events.push(Event::Error { id, src: e.props.src.clone(), why: Some(why.clone()) });
                    }
                }
                SourceStatus::Ready(info) => {
                    if e.info.is_none() && !e.sized {
                        // The box takes the source's size: build it again.
                        touch.push(id);
                        draw = true;
                    }
                    e.info = Some(info.clone());
                }
            }
            if let Some(key) = e.info.as_ref().and_then(|info| want_of(id, e, info)) {
                e.want = Some(key);
            }
        }
    });
    fetches.sort();
    fetches.dedup();
    for src in &fetches {
        fetch(src, cx);
    }
    for (id, src) in images {
        let event = match crate::img::probe(id, &src, cx) {
            crate::img::Probe::Pending => continue,
            crate::img::Probe::Loaded(w, h) => Event::Load { id, src, size: (w, h) },
            // The resolver or the loader has said why.
            crate::img::Probe::Failed => Event::Error { id, src, why: None },
        };
        NODES.with(|m| {
            if let Some(e) = m.borrow_mut().get_mut(&id) {
                e.loaded = matches!(event, Event::Load { .. });
                e.failed = !e.loaded;
            }
        });
        events.push(event);
    }

    // The targets: asked for while a copy was seen lately, played while one
    // can be seen now.
    let demand: HashMap<TargetKey, bool> = NODES.with(|m| {
        let mut demand: HashMap<TargetKey, bool> = HashMap::new();
        for e in m.borrow().values().filter(|e| e.recent(now)) {
            if let Some(key) = &e.want {
                *demand.entry(key.clone()).or_default() |= e.visible;
            }
        }
        demand
    });
    for (key, visible) in &demand {
        stickers.demand(key, *visible);
    }
    let frames: HashMap<TargetKey, Frame> =
        demand.keys().filter_map(|k| stickers.frame(k).map(|f| (k.clone(), f))).collect();

    // This window's tiles.
    let shown = write(window, here, &frames, &mut touch);
    live::note(shown);

    // Another window's tiles are written by its own draw: ask for one when a
    // copy there can be seen and has a newer frame.
    draw |= NODES.with(|m| {
        m.borrow().values().any(|e| {
            e.win != here
                && e.visible
                && e.want.as_ref().is_some_and(|k| {
                    let written = TILES.with(|t| t.borrow().get(&(k.clone(), e.win)).map_or(0, |t| t.seq));
                    frames.get(k).is_some_and(|f| f.seq != written)
                })
        })
    });

    retire(now, &mut touch);

    // `load` once a copy shows a frame, `ended` once a `once` loop is done.
    NODES.with(|m| {
        for (&id, e) in m.borrow_mut().iter_mut() {
            if e.props.image {
                continue;
            }
            if !e.loaded && e.shows.is_some() {
                e.loaded = true;
                let size = e.info.as_ref().map_or((0.0, 0.0), |i| (i.width as f64, i.height as f64));
                events.push(Event::Load { id, src: e.props.src.clone(), size });
            }
            let ended = e.shows.as_ref().is_some_and(|(k, _)| k.once.is_some() && frames.get(k).is_some_and(|f| f.ended));
            if ended && !e.ended {
                e.ended = true;
                events.push(Event::Ended { id });
            }
        }
    });

    if !touch.is_empty() {
        tree::with_tree(|t| {
            for id in touch {
                t.touch(id);
            }
        });
    }
    dispatch(events, window, cx);
    draw
}

/// A pop-out's half of [`tick`], from its own draw: its tiles written with
/// the frames due now. What the pump's tick decided (visibility, targets)
/// stands; the draw that follows shows the frames.
pub(crate) fn window_tick(window: &mut Window) {
    let here = windows::key(window);
    let keys: HashSet<TargetKey> = NODES.with(|m| {
        m.borrow().values().filter(|e| e.win == here && e.visible).filter_map(|e| e.want.clone()).collect()
    });
    if keys.is_empty() && TILES.with(|m| m.borrow().keys().all(|(_, w)| *w != here)) {
        return;
    }
    let stickers = vn_sticker::global();
    let frames: HashMap<TargetKey, Frame> =
        keys.into_iter().filter_map(|k| stickers.frame(&k).map(|f| (k, f))).collect();
    let mut touch = Vec::new();
    write(window, here, &frames, &mut touch);
    if !touch.is_empty() {
        tree::with_tree(|t| {
            for id in touch {
                t.touch(id);
            }
        });
    }
}

/// Node `id`'s target now, from the box it was painted at last.
fn want_of(id: u32, e: &Entry, info: &SourceInfo) -> Option<TargetKey> {
    let (_, _, w, h) = windows::peek(e.win, |w| w.bounds.get(&id).copied()).flatten()?;
    let env = windows::env_of(e.win);
    let scale = env.map(|e| e.scale_factor).filter(|s| *s > 0.0).unwrap_or(1.0);
    let reduce_motion = env.is_some_and(|e| e.reduce_motion);
    key_for(id, &e.props, info, (w * scale, h * scale), reduce_motion)
}

/// Writes the frames due into window `here`'s tiles and moves nodes onto the
/// tile they want once it has a frame. The answer is what the writes need.
fn write(window: &mut Window, here: WinKey, frames: &HashMap<TargetKey, Frame>, touch: &mut Vec<u32>) -> Shown {
    // The targets this window paints, and whether a copy of each can be seen.
    let needed: HashMap<TargetKey, bool> = NODES.with(|m| {
        let mut needed: HashMap<TargetKey, bool> = HashMap::new();
        for e in m.borrow().values().filter(|e| e.win == here) {
            if let Some(key) = &e.want {
                *needed.entry(key.clone()).or_default() |= e.visible;
            }
        }
        needed
    });
    let mut shown = Shown::Nowhere;
    let mut moved: Vec<TargetKey> = Vec::new();
    TILES.with(|tiles| {
        let mut tiles = tiles.borrow_mut();
        for (key, &visible) in &needed {
            let Some(frame) = frames.get(key) else { continue };
            if frame.width != key.width || frame.height != key.height {
                continue;
            }
            let slot = (key.clone(), here);
            if !tiles.contains_key(&slot) {
                // A tile is made for a copy that can be seen, not ahead of it.
                if !visible {
                    continue;
                }
                tiles.insert(slot.clone(), Tile { live: LiveTile::new(here, key.width, key.height), seq: 0 });
            }
            let tile = tiles.get_mut(&slot).expect("just made");
            if tile.seq == frame.seq {
                continue;
            }
            // Asked as if visible, to tell a write in place from one that
            // moved the tile.
            let wrote = tile.live.write_bgra(window, frame.bytes.to_vec(), true);
            tile.seq = frame.seq;
            if wrote == Shown::Draw {
                moved.push(key.clone());
            }
            shown = shown.max(if visible { wrote } else { Shown::Nowhere });
        }
    });
    NODES.with(|m| {
        for (&id, e) in m.borrow_mut().iter_mut().filter(|(_, e)| e.win == here) {
            // A tile that moved: a cache boundary over the node would replay
            // a sprite pointing at the old place.
            if e.shows.as_ref().is_some_and(|(k, w)| *w == here && moved.contains(k)) {
                touch.push(id);
            }
            let Some(want) = &e.want else { continue };
            if e.shows.as_ref().is_some_and(|(k, w)| k == want && *w == here) {
                continue;
            }
            let ready = TILES.with(|t| t.borrow().get(&(want.clone(), here)).is_some_and(|t| t.seq != 0));
            if ready {
                e.shows = Some((want.clone(), here));
                touch.push(id);
                if e.visible {
                    shown = Shown::Draw;
                }
            }
        }
    });
    shown
}

/// Hands back every tile no copy seen lately paints or wants. A node whose
/// tile goes stops painting it (and is touched, so a cache boundary renders
/// it again); it gets a tile back when it is seen again.
fn retire(now: Instant, touch: &mut Vec<u32>) {
    let kept: HashSet<(TargetKey, WinKey)> = NODES.with(|m| {
        let mut kept = HashSet::new();
        for e in m.borrow().values().filter(|e| e.recent(now)) {
            if let Some(key) = &e.want {
                kept.insert((key.clone(), e.win));
            }
            if let Some(shows) = &e.shows {
                kept.insert(shows.clone());
            }
        }
        kept
    });
    let gone: Vec<(TargetKey, WinKey)> = TILES.with(|m| {
        let mut m = m.borrow_mut();
        let gone: Vec<_> = m.keys().filter(|k| !kept.contains(*k)).cloned().collect();
        for slot in &gone {
            if let Some(tile) = m.remove(slot) {
                tile.live.retire();
            }
        }
        gone
    });
    if gone.is_empty() {
        return;
    }
    NODES.with(|m| {
        for (&id, e) in m.borrow_mut().iter_mut() {
            if e.shows.as_ref().is_some_and(|s| gone.contains(s)) {
                e.shows = None;
                touch.push(id);
            }
        }
    });
}

fn dispatch(events: Vec<Event>, window: &mut Window, cx: &mut App) {
    for event in events {
        match event {
            Event::Load { id, src, size } => {
                let payload = EventPayload { a: size.0, b: size.1, key: Some(&src), ..Default::default() };
                crate::render::js_dispatch_app(id, tree::EV_LOAD, &payload, window, cx);
            }
            Event::Error { id, src, why } => {
                if let Some(why) = why {
                    warn_compat(&format!("vn-sticker:{src}"), &format!("<vn-sticker src=\"{src}\">: {why}"));
                }
                let payload = EventPayload { key: Some(&src), ..Default::default() };
                crate::render::js_dispatch_app(id, tree::EV_ERROR, &payload, window, cx);
            }
            Event::Ended { id } => {
                crate::render::js_dispatch_app(id, tree::EV_ENDED, &EventPayload::default(), window, cx);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn props(src: &str) -> Props {
        Props {
            src: Arc::from(src),
            image: false,
            tint: false,
            play: Play::Auto,
            once: false,
            max_fps: None,
            fit: Fit::Contain,
        }
    }

    fn info(kind: Kind, width: u32, height: u32, frames: u32) -> SourceInfo {
        SourceInfo { kind, width, height, frames, fps: 60.0 }
    }

    #[test]
    fn a_target_is_the_picture_under_object_fit_rounded_up_to_a_step() {
        // A 512 sticker in a 200 × 200 box at 1.5: 300 device px, a 16 step.
        assert_eq!(target_size((512, 512), (300.0, 300.0), Fit::Contain, Kind::Lottie), Some((304, 304)));
        // Inline emoji: 24 css px at 1.5 = 36, a 4 step.
        assert_eq!(target_size((100, 100), (36.0, 36.0), Fit::Contain, Kind::Video), Some((36, 36)));
        assert_eq!(target_size((100, 100), (35.2, 35.2), Fit::Contain, Kind::Video), Some((36, 36)));
        // Contain fits the long side, cover the short one.
        assert_eq!(target_size((200, 100), (100.0, 100.0), Fit::Contain, Kind::Lottie), Some((100, 50)));
        assert_eq!(target_size((200, 100), (100.0, 100.0), Fit::Cover, Kind::Lottie), Some((200, 100)));
        // Fill stretches: drawn at the larger scale, so no axis is magnified.
        assert_eq!(target_size((200, 100), (100.0, 100.0), Fit::Fill, Kind::Lottie), Some((200, 100)));
        // A step between 128 and 256 is 8.
        assert_eq!(target_size((512, 512), (130.0, 130.0), Fit::Contain, Kind::Lottie), Some((136, 136)));
    }

    #[test]
    fn a_video_is_never_drawn_above_its_size_and_a_lottie_not_above_the_cap() {
        assert_eq!(target_size((100, 100), (540.0, 540.0), Fit::Contain, Kind::Video), Some((100, 100)));
        // The step does not round past the video's own size either.
        assert_eq!(target_size((98, 98), (97.0, 97.0), Fit::Contain, Kind::Video), Some((98, 98)));
        assert_eq!(target_size((512, 512), (4000.0, 4000.0), Fit::Contain, Kind::Lottie), Some((1024, 1024)));
        assert_eq!(target_size((512, 256), (4000.0, 4000.0), Fit::Contain, Kind::Lottie), Some((1024, 512)));
    }

    #[test]
    fn an_empty_box_or_source_has_no_target() {
        assert_eq!(target_size((512, 512), (0.0, 300.0), Fit::Contain, Kind::Lottie), None);
        assert_eq!(target_size((0, 512), (300.0, 300.0), Fit::Contain, Kind::Lottie), None);
        assert_eq!(target_size((512, 512), (f32::NAN, 300.0), Fit::Contain, Kind::Lottie), None);
    }

    #[test]
    fn a_box_moving_by_a_pixel_keeps_its_target() {
        let a = target_size((512, 512), (290.0, 290.0), Fit::Contain, Kind::Lottie);
        let b = target_size((512, 512), (291.0, 291.0), Fit::Contain, Kind::Lottie);
        assert_eq!(a, b);
    }

    #[test]
    fn play_decides_still_and_once_is_a_copy_of_its_own() {
        let lottie = info(Kind::Lottie, 512, 512, 180);
        let key = key_for(7, &props("s"), &lottie, (300.0, 300.0), false).unwrap();
        assert!(!key.still && key.once.is_none() && !key.alpha_only);

        // auto follows reduced motion; on beats it; off is the first frame.
        assert!(key_for(7, &props("s"), &lottie, (300.0, 300.0), true).unwrap().still);
        let on = Props { play: Play::On, ..props("s") };
        assert!(!key_for(7, &on, &lottie, (300.0, 300.0), true).unwrap().still);
        let off = Props { play: Play::Off, ..props("s") };
        assert!(key_for(7, &off, &lottie, (300.0, 300.0), false).unwrap().still);

        let once = Props { once: true, ..props("s") };
        assert_eq!(key_for(7, &once, &lottie, (300.0, 300.0), false).unwrap().once, Some(7));
        let once_off = Props { once: true, play: Play::Off, ..props("s") };
        assert_eq!(key_for(7, &once_off, &lottie, (300.0, 300.0), false).unwrap().once, None, "a still never ends");

        // One frame is a still whatever play says.
        let single = info(Kind::Lottie, 512, 512, 1);
        assert!(key_for(7, &on, &single, (300.0, 300.0), false).unwrap().still);
    }

    #[test]
    fn a_tinted_video_decodes_its_alpha_and_a_tinted_lottie_does_not() {
        let tint = Props { tint: true, max_fps: Some(24), ..props("s") };
        let video = key_for(1, &tint, &info(Kind::Video, 100, 100, 90), (48.0, 48.0), false).unwrap();
        assert!(video.alpha_only);
        assert_eq!(video.max_fps, Some(24));
        let lottie = key_for(1, &tint, &info(Kind::Lottie, 100, 100, 90), (48.0, 48.0), false).unwrap();
        assert!(!lottie.alpha_only, "a Lottie is tinted when painted");
    }

    #[test]
    fn two_copies_at_one_size_share_a_target_and_a_tint_does_not_split_a_lottie() {
        let lottie = info(Kind::Lottie, 100, 100, 90);
        let a = key_for(1, &props("s"), &lottie, (48.0, 48.0), false);
        let b = key_for(2, &Props { tint: true, ..props("s") }, &lottie, (47.0, 47.0), false);
        assert_eq!(a, b);
    }

    #[test]
    fn attributes_read_as_documented() {
        assert_eq!(play_from(None), Ok(Play::Auto));
        assert_eq!(play_from(Some("auto")), Ok(Play::Auto));
        assert_eq!(play_from(Some(" ON ")), Ok(Play::On));
        assert_eq!(play_from(Some("off")), Ok(Play::Off));
        assert_eq!(play_from(Some("false")), Ok(Play::Off));
        assert_eq!(play_from(Some("sometimes")), Err("sometimes".into()));
        assert_eq!(max_fps_from(Some("30")), Some(30));
        assert_eq!(max_fps_from(Some(" 24.5 ")), Some(24));
        assert_eq!(max_fps_from(Some("0")), None);
        assert_eq!(max_fps_from(Some("fast")), None);
        assert_eq!(max_fps_from(None), None);
        assert!(bool_attr(Some("")));
        assert!(!bool_attr(Some("false")));
        assert!(!bool_attr(None));
    }

    #[test]
    fn an_entry_is_recent_for_the_tile_idle_time() {
        let now = Instant::now();
        let mut e = Entry::new(props("s"), windows::UNBOUND);
        assert!(!e.recent(now), "never seen");
        e.seen = Some(now);
        assert!(e.recent(now + TILE_IDLE - Duration::from_millis(1)));
        assert!(!e.recent(now + TILE_IDLE));
    }
}
