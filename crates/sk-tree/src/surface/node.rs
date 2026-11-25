//! The element side of `<sk-surface>`: per node, the [`LiveTile`] the surface's frame is written
//! into, the painted size reported to the producer and to JS, and the events.

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use gpui::{App, RenderImage, Window};

use super::{display, feedback, Frame, SurfaceDisplay, SurfaceFormat, SurfaceHandle, SurfaceInfo};
use crate::live::{self, LiveTile, Shown};
use crate::tree;
use crate::windows::{self, WinKey};
use crate::EventPayload;

struct Node {
    surface: SurfaceHandle,
    window: WinKey,
    tile: Option<LiveTile>,
    #[cfg(windows)]
    latched: Option<super::d3d12::Ready>,
    /// The surface's `seq` the tile holds (or that cleared it).
    seen: u64,
    /// The id of the frame the tile holds.
    frame: u64,
    /// The content box, physical px, and the scale, from the last paint.
    painted: (u32, u32, f32),
    reported: (u32, u32),
    visible: bool,
    ready: bool,
    /// The display last reported to JS.
    display: Option<SurfaceDisplay>,
    /// The brightest the frame may be painted: `dynamic-range-limit` applied to the display.
    headroom: f32,
    /// The tile holds the frame's half-float values, painted up to `headroom`.
    hdr: bool,
    attached: bool,
    /// Why the last GPU frame was not shown, once logged.
    #[cfg_attr(not(any(windows, target_os = "macos")), allow(dead_code))]
    refused: Option<String>,
    /// An `<sk-layer>`'s picture: not drawn into the page, shown only while `read`.
    proxy: bool,
    /// A backdrop of its window sampled it in the frame last drawn there.
    read: bool,
}

thread_local! {
    static NODES: RefCell<HashMap<u32, Node>> = RefCell::new(HashMap::new());
    /// Events a pop-out's draw raised, for the next primary tick to dispatch.
    static DEFERRED: RefCell<Vec<(u32, Event)>> = const { RefCell::new(Vec::new()) };
    /// Each window's native handle, as present feedback names it.
    static PRESENT_KEYS: RefCell<HashMap<WinKey, u64>> = RefCell::new(HashMap::new());
    /// Windows other than the ticking one with a new proxy frame a backdrop reads there.
    static PROXY_REDRAWS: RefCell<Vec<WinKey>> = const { RefCell::new(Vec::new()) };
}

fn new_node(surface: SurfaceHandle, win: WinKey, proxy: bool) -> Node {
    Node {
        surface,
        window: win,
        tile: None,
        #[cfg(windows)]
        latched: None,
        seen: 0,
        frame: 0,
        painted: (0, 0, 1.0),
        reported: (0, 0),
        visible: false,
        ready: false,
        display: None,
        headroom: 1.0,
        hdr: false,
        attached: false,
        refused: None,
        proxy,
        read: false,
    }
}

#[cfg(windows)]
enum Gpu {
    D3D11(super::d3d11::Texture),
    D3D12(super::d3d12::Ready),
}

#[cfg(target_os = "macos")]
enum Gpu {
    Metal(super::metal::Ready),
}

enum Event {
    Resize(u32, u32, f32),
    Ready(u32, u32),
    Lost(&'static str),
    Display(SurfaceDisplay),
}

/// Called from the build: binds node `id` to the surface `name` and returns the picture to paint.
pub(crate) fn note_build(id: u32, win: WinKey, name: &str) -> Option<Arc<RenderImage>> {
    NODES.with(|m| {
        let mut m = m.borrow_mut();
        if m.get(&id).is_some_and(|n| n.surface.name() != name) {
            if let Some(old) = m.remove(&id) {
                forget(old);
            }
        }
        if name.is_empty() {
            return None;
        }
        let node = m
            .entry(id)
            .or_insert_with(|| new_node(super::surface(name), win, false));
        node.window = win;
        node.tile
            .as_ref()
            .filter(|t| t.window() == win && node.ready)
            .map(LiveTile::image)
    })
}

/// The picture an `<sk-layer>` shows under the page.
pub(crate) struct ProxyPicture {
    pub(crate) image: Option<Arc<RenderImage>>,
    /// The frame's size in pixels, 0 while there is none.
    pub(crate) size: (u32, u32),
    pub(crate) headroom: f32,
}

/// Called from the paint of `<sk-layer>` `id`, whose box is `width` x `height` device pixels in
/// `win`: binds the node to `surface`, the layer's proxy, and returns the picture it holds.
pub(crate) fn note_proxy(
    id: u32,
    win: WinKey,
    surface: &SurfaceHandle,
    width: f32,
    height: f32,
    scale: f32,
) -> ProxyPicture {
    NODES.with(|m| {
        let mut m = m.borrow_mut();
        let element = m
            .values()
            .any(|n| !n.proxy && Arc::ptr_eq(n.surface.shared(), surface.shared()));
        surface.lock_state().proxy = !element;
        if m.get(&id)
            .is_some_and(|n| !Arc::ptr_eq(n.surface.shared(), surface.shared()))
        {
            if let Some(old) = m.remove(&id) {
                forget(old);
            }
        }
        let node = m
            .entry(id)
            .or_insert_with(|| new_node(surface.clone(), win, true));
        node.window = win;
        node.painted = (
            width.round().max(0.0) as u32,
            height.round().max(0.0) as u32,
            scale,
        );
        let tile = node
            .tile
            .as_ref()
            .filter(|t| t.window() == win && node.ready);
        ProxyPicture {
            image: tile.map(LiveTile::image),
            size: tile.map_or((0, 0), LiveTile::size),
            headroom: if node.hdr { node.headroom } else { 1.0 },
        }
    })
}

/// Takes which proxies a backdrop of `win` sampled in its frame last drawn.
pub(crate) fn note_reads(win: WinKey, read: &[u64]) {
    NODES.with(|m| {
        for (&id, node) in m.borrow_mut().iter_mut() {
            if node.proxy && node.window == win {
                node.read = read.contains(&u64::from(id));
            }
        }
    });
}

/// The windows besides the ticking one that must draw to show a new proxy frame.
pub(crate) fn take_proxy_redraws() -> Vec<WinKey> {
    PROXY_REDRAWS.with(|r| std::mem::take(&mut *r.borrow_mut()))
}

fn paint_headroom(id: u32) -> f32 {
    NODES.with(|m| {
        m.borrow()
            .get(&id)
            .filter(|n| n.hdr)
            .map_or(1.0, |n| n.headroom)
    })
}

/// Node `id`'s picture, painted with the headroom its frame may reach.
pub(crate) struct HdrPicture(pub(crate) u32, pub(crate) gpui::AnyElement);

impl gpui::Element for HdrPicture {
    type RequestLayoutState = ();
    type PrepaintState = ();

    fn id(&self) -> Option<gpui::ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static std::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _id: Option<&gpui::GlobalElementId>,
        _inspector_id: Option<&gpui::InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (gpui::LayoutId, ()) {
        (self.1.request_layout(window, cx), ())
    }

    fn prepaint(
        &mut self,
        _id: Option<&gpui::GlobalElementId>,
        _inspector_id: Option<&gpui::InspectorElementId>,
        _bounds: gpui::Bounds<gpui::Pixels>,
        _request_layout: &mut (),
        window: &mut Window,
        cx: &mut App,
    ) {
        self.1.prepaint(window, cx);
    }

    fn paint(
        &mut self,
        _id: Option<&gpui::GlobalElementId>,
        _inspector_id: Option<&gpui::InspectorElementId>,
        _bounds: gpui::Bounds<gpui::Pixels>,
        _request_layout: &mut (),
        _prepaint: &mut (),
        window: &mut Window,
        cx: &mut App,
    ) {
        let headroom = paint_headroom(self.0);
        window.with_signed_image_headroom(headroom, |window| self.1.paint(window, cx));
    }
}

/// Called from the paint with the content box in window pixels.
pub(crate) fn note_painted(
    id: u32,
    width: f32,
    height: f32,
    scale: f32,
    #[cfg(target_os = "macos")] waker: Option<gpui::SurfaceFrameWaker>,
) {
    NODES.with(|m| {
        if let Some(node) = m.borrow_mut().get_mut(&id) {
            node.painted = (
                (width * scale).round().max(0.0) as u32,
                (height * scale).round().max(0.0) as u32,
                scale,
            );
            #[cfg(target_os = "macos")]
            if let Some(waker) = waker {
                if width > 0.0 && height > 0.0 && live::visible(id, node.window) {
                    node.surface
                        .lock_state()
                        .pacing_windows
                        .entry(node.window)
                        .or_insert(waker);
                }
            }
        }
    });
}

fn attached(id: u32) -> bool {
    tree::read_tree(|t| t.connected(id))
}

fn forget(node: Node) {
    if let Some(tile) = node.tile {
        tile.retire();
    }
}

pub(crate) fn gc(live: impl Fn(u32) -> bool) {
    let gone: Vec<Node> = NODES.with(|m| {
        let mut m = m.borrow_mut();
        let ids: Vec<u32> = m.keys().copied().filter(|&id| !live(id)).collect();
        ids.into_iter().filter_map(|id| m.remove(&id)).collect()
    });
    for node in gone {
        forget(node);
    }
}

pub(crate) fn forget_window(key: WinKey) {
    display::forget(key);
    #[cfg(target_os = "macos")]
    for surface in super::all() {
        surface.lock_state().pacing_windows.remove(&key);
    }
    if let Some(window) = PRESENT_KEYS.with(|k| k.borrow_mut().remove(&key)) {
        feedback::with(|t| t.window_closed(window));
    }
    NODES.with(|m| {
        for node in m.borrow_mut().values_mut() {
            if node.tile.as_ref().is_some_and(|t| t.window() == key) {
                node.tile = None;
                node.seen = 0;
            }
        }
    });
}

/// Per frame, before the draw: writes new frames into visible nodes' tiles, reports sizes and
/// visibility, and fires the element events. Returns `true` when the window must draw.
pub(crate) fn tick(window: &mut Window, cx: &mut App) -> bool {
    if !super::any() {
        return false;
    }
    #[cfg(target_os = "macos")]
    super::metal::watch_device();
    let here = windows::key(window);
    note_reads(here, &window.vn_underlays_read());
    let page_hidden = live::page_hidden();
    let here_shown = window.is_visible();
    let mut events = DEFERRED.with(|d| std::mem::take(&mut *d.borrow_mut()));
    let mut touch = Vec::new();
    let mut draw = false;
    let mut infos: HashMap<*const super::Shared, SurfaceInfo> = HashMap::new();
    let mut shown = Shown::Nowhere;
    let mut refresh_here = false;
    NODES.with(|m| {
        for (&id, node) in m.borrow_mut().iter_mut() {
            let attached = attached(id);
            node.visible = attached
                && !page_hidden
                && (node.window != here || here_shown)
                && node.painted.0 > 0
                && node.painted.1 > 0
                && live::visible(id, node.window)
                && (!node.proxy || node.read);
            if node.proxy {
                let mut moved = Vec::new();
                if attached {
                    follow_headroom(id, node, &mut moved);
                }
                node.attached = attached;
                let fresh = node.surface.lock_state().seq != node.seen;
                if node.visible && !moved.is_empty() && node.window == here {
                    refresh_here = true;
                } else if node.visible && (fresh || !moved.is_empty()) && node.window != here {
                    PROXY_REDRAWS.with(|r| {
                        let mut r = r.borrow_mut();
                        if !r.contains(&node.window) {
                            r.push(node.window);
                        }
                    });
                }
                continue;
            }
            if attached && node.painted.0 > 0 && (node.painted.0, node.painted.1) != node.reported {
                node.reported = (node.painted.0, node.painted.1);
                events.push((
                    id,
                    Event::Resize(node.painted.0, node.painted.1, node.painted.2),
                ));
            }
            if attached {
                follow_headroom(id, node, &mut touch);
            }
            node.attached = attached;
            if node.window != here {
                draw |= node.visible && node.surface.lock_state().seq != node.seen;
            }
        }
    });
    if refresh_here {
        window.refresh();
    }
    // After the switch, so the display reported says whether this window now draws HDR.
    let hdr = update_output(window, here);
    NODES.with(|m| {
        for (&id, node) in m.borrow_mut().iter_mut() {
            if node.attached {
                let on = display::of(node.window);
                if on.known && node.display.is_none_or(|d| on.differs(&d)) {
                    node.display = Some(on);
                    if !node.proxy {
                        events.push((id, Event::Display(on)));
                    }
                }
                let limit = crate::inherit::dynamic_range_limit(id);
                let info = infos.entry(Arc::as_ptr(node.surface.shared())).or_default();
                fold_info(info, node, on, limit);
            }
            if node.window == here {
                let (s, event) = write_node(window, here, id, node, hdr, &mut touch);
                shown = shown.max(s);
                events.extend(event.map(|e| (id, e)));
            }
        }
    });
    live::note(shown);
    display::note_tick(infos.values().any(|info| info.visible));
    finish(touch, &mut draw);
    #[cfg(target_os = "macos")]
    update_pacing(window, here);
    publish_infos(super::all(), &infos);
    carry(window, here);
    dispatch(events, window, cx);
    draw
}

/// Adds a node to its surface's info: the largest visible node sets the size (a hidden one's last
/// size only stands in), and the surface is visible while any node is, element or picture.
fn fold_info(
    info: &mut SurfaceInfo,
    node: &Node,
    on: SurfaceDisplay,
    limit: sk_css::DynamicRangeLimit,
) {
    let rank = |visible: bool, w: u32, h: u32| (visible, u64::from(w) * u64::from(h));
    if rank(node.visible, node.painted.0, node.painted.1)
        > rank(info.visible, info.width, info.height)
    {
        info.width = node.painted.0;
        info.height = node.painted.1;
        info.scale = node.painted.2;
        info.display = on;
        info.dynamic_range_limit = limit;
    } else if !info.display.known {
        info.display = on;
    }
    info.visible |= node.visible;
}

/// Hands each surface its info; only a surface no element shows waits for a backdrop to read it
/// before it wakes the frame loop.
fn publish_infos(surfaces: Vec<SurfaceHandle>, infos: &HashMap<*const super::Shared, SurfaceInfo>) {
    let (pictures, elements) = NODES.with(|m| {
        let m = m.borrow();
        let of = |proxy: bool| -> HashSet<*const super::Shared> {
            m.values()
                .filter(|n| n.proxy == proxy)
                .map(|n| Arc::as_ptr(n.surface.shared()))
                .collect()
        };
        (of(true), of(false))
    });
    for surface in surfaces {
        let key = Arc::as_ptr(surface.shared());
        let info = infos.get(&key).copied().unwrap_or_default();
        surface.lock_state().proxy = pictures.contains(&key) && !elements.contains(&key);
        surface.set_info(info);
        #[cfg(any(windows, target_os = "macos"))]
        surface.lock_state().trim_slots(std::time::Instant::now());
    }
}

#[cfg(windows)]
pub(crate) fn latch_d3d12(window: &Window) {
    if !super::any() {
        return;
    }
    let here = windows::key(window);
    let key = present_key(window, here);
    NODES.with_borrow_mut(|nodes| {
        for node in nodes
            .values_mut()
            .filter(|node| node.window == here && node.visible)
        {
            node.latched = node.surface.lock_state().latch_d3d12(node.seen, key);
        }
    });
}

fn present_key(window: &Window, here: WinKey) -> Option<u64> {
    if !feedback::watching() {
        return None;
    }
    PRESENT_KEYS.with(|k| {
        let mut keys = k.borrow_mut();
        if let Some(&key) = keys.get(&here) {
            return Some(key);
        }
        let key = feedback::window_key(window, here)?;
        keys.insert(here, key);
        Some(key)
    })
}

/// Tells present feedback which frame of each surface `here`'s next present shows.
fn carry(window: &Window, here: WinKey) {
    let Some(key) = present_key(window, here) else {
        return;
    };
    let carrying = NODES.with(|m| {
        m.borrow()
            .values()
            .filter(|n| n.window == here && n.visible && n.ready && n.tile.is_some())
            .map(|n| (feedback::key(n.surface.shared()), n.frame))
            .collect()
    });
    feedback::with(|t| t.carry(key, carrying));
    #[cfg(not(any(windows, target_os = "macos")))]
    feedback::ticked(key, display::of(here).refresh_hz);
}

/// Samples `window`'s display; every tick, so `dynamic-range` answers without a surface.
pub(crate) fn sample_display(window: &Window) {
    display::sample(window, windows::key(window));
}

/// Moves a node to the headroom it may be painted with now, from the display's live reading.
fn follow_headroom(id: u32, node: &mut Node, touch: &mut Vec<u32>) {
    let limit = crate::inherit::dynamic_range_limit(id);
    let output = display::live(node.window).output_headroom;
    note_headroom(id, node, limit.headroom(output), touch);
}

/// A picture shown in HDR is repainted when its headroom moves.
fn note_headroom(id: u32, node: &mut Node, headroom: f32, touch: &mut Vec<u32>) {
    if (headroom - node.headroom).abs() > 1e-3 * headroom {
        node.headroom = headroom;
        if node.hdr && node.ready {
            touch.push(id);
        }
    }
}

/// Puts `here` in HDR while a visible half-float surface in it may be brighter than SDR white,
/// asking for the most headroom such a surface's `dynamic-range-limit` allows on its display.
fn update_output(window: &Window, here: WinKey) -> bool {
    let headroom = wanted_headroom(here, display::reach(here));
    display::update_output(window, here, headroom)
}

/// A surface that holds no frame asks for nothing: an idle window stays 8-bit.
fn wanted_headroom(here: WinKey, reach: f32) -> f32 {
    NODES.with(|m| {
        m.borrow()
            .iter()
            .filter(|(_, n)| {
                n.window == here
                    && !n.proxy
                    && n.visible
                    && n.surface.format() == SurfaceFormat::Rgba16Float
                    && n.surface.lock_state().frame.is_some()
            })
            .map(|(&id, _)| crate::inherit::dynamic_range_limit(id).headroom(reach))
            .fold(1.0, f32::max)
    })
}

/// A pop-out's half of [`tick`], from its own draw: its nodes' tiles live in its atlas.
pub(crate) fn window_tick(window: &mut Window) {
    if !super::any() {
        return;
    }
    let here = windows::key(window);
    note_reads(here, &window.vn_underlays_read());
    #[cfg(target_os = "macos")]
    update_pacing(window, here);
    let hdr = update_output(window, here);
    let mut touch = Vec::new();
    NODES.with(|m| {
        for (&id, node) in m.borrow_mut().iter_mut() {
            if node.window == here {
                if let (_, Some(event)) = write_node(window, here, id, node, hdr, &mut touch) {
                    DEFERRED.with(|d| d.borrow_mut().push((id, event)));
                }
            }
        }
    });
    let mut draw = false;
    finish(touch, &mut draw);
    carry(window, here);
}

#[cfg(target_os = "macos")]
fn update_pacing(window: &Window, here: WinKey) {
    let Some(waker) = window.surface_frame_waker() else {
        return;
    };
    let visible: std::collections::HashSet<_> = NODES.with_borrow(|nodes| {
        nodes
            .values()
            .filter(|n| n.window == here && n.visible)
            .map(|n| Arc::as_ptr(n.surface.shared()))
            .collect()
    });
    for surface in super::all() {
        let mut state = surface.lock_state();
        if visible.contains(&Arc::as_ptr(surface.shared())) {
            state
                .pacing_windows
                .entry(here)
                .or_insert_with(|| waker.clone());
        } else {
            state.pacing_windows.remove(&here);
        }
    }
}

fn finish(touch: Vec<u32>, draw: &mut bool) {
    if touch.is_empty() {
        return;
    }
    *draw = true;
    tree::with_tree(|t| {
        for id in touch {
            t.touch(id);
        }
    });
}

/// [`write`]; a proxy's new picture redraws its own window only, and its element hears no
/// surface events.
fn write_node(
    window: &mut Window,
    here: WinKey,
    id: u32,
    node: &mut Node,
    window_hdr: bool,
    touch: &mut Vec<u32>,
) -> (Shown, Option<Event>) {
    if !node.proxy {
        return write(window, here, id, node, window_hdr, touch);
    }
    let mut own = Vec::new();
    let (shown, _) = write(window, here, id, node, window_hdr, &mut own);
    if shown == Shown::Draw || !own.is_empty() {
        window.refresh();
        return (Shown::Nowhere, None);
    }
    (shown, None)
}

fn write(
    window: &mut Window,
    here: WinKey,
    id: u32,
    node: &mut Node,
    window_hdr: bool,
    touch: &mut Vec<u32>,
) -> (Shown, Option<Event>) {
    // A half-float frame goes to an HDR window as it is, to a standard one mapped to 8-bit: a
    // switch rewrites the frame already shown.
    let hdr = window_hdr && node.surface.format() == SurfaceFormat::Rgba16Float;
    if hdr != node.hdr {
        node.hdr = hdr;
        node.seen = 0;
    }
    #[cfg(windows)]
    let mut unmapped = None;
    // One lock: a present on another thread may resize the surface's texture in between.
    let locked = {
        let mut st = node.surface.lock_state();
        #[cfg(windows)]
        let latched = node.latched.take();
        #[cfg(windows)]
        let texture = match st.frame {
            Some(Frame::D3D12 { .. }) => st.gpu12.for_tick(latched).map(Gpu::D3D12),
            // Only a visible node's tick may draw a half-float frame's mapping.
            Some(Frame::Gpu { .. }) if node.visible && st.seq != node.seen => {
                match st.gpu_texture(hdr) {
                    Ok(texture) => texture.map(Gpu::D3D11),
                    Err(error) => {
                        unmapped = Some(error);
                        None
                    }
                }
            }
            _ => None,
        };
        #[cfg(target_os = "macos")]
        let texture = match st.frame {
            Some(Frame::Metal { .. }) => st.metal.ready().map(Gpu::Metal),
            _ => None,
        };
        #[cfg(not(any(windows, target_os = "macos")))]
        let texture = ();
        let copied = copied(&texture);
        if let Some((done, (_, ready, _))) = copied {
            log::trace!(target: "sk_tree::surface::feedback", "{} tick node={id} latest={} selected={ready} done={done}", feedback::monotonic_ns(), st.frame_id);
        }
        let showable = match copied {
            Some((done, (_, frame, _))) => done.then_some(frame),
            None => st.frame.is_some().then_some(st.frame_id),
        };
        if let Some(frame) = showable {
            st.settle(frame);
            if node.visible && st.seq != node.seen {
                st.pin(frame, present_key(window, here));
            }
        }
        (
            st.seq,
            st.frame_id,
            st.frame.clone(),
            st.lost,
            texture,
            copied,
        )
    };
    let (mut seq, mut frame_id, frame, lost, texture, copied) = locked;
    #[cfg(windows)]
    if let Some(error) = unmapped {
        refused(
            node,
            format!("its half-float frame could not be mapped for the window: {error}"),
        );
    }
    if seq == node.seen {
        return (Shown::Nowhere, None);
    }
    // The copy into the surface's texture waits on the producer's fence on the GPU; handing it to
    // the atlas before it ran would stall the renderer's queue on the producer. While the newest
    // frame copies, the newest copied one is shown.
    let mut size = None;
    if let Some((done, (ready_seq, ready_frame, ready_size))) = copied {
        if node.visible {
            if !done {
                sk_runtime::frames::wake();
                return (Shown::Nowhere, None);
            }
            if ready_seq != seq {
                sk_runtime::frames::wake();
                if ready_seq <= node.seen {
                    return (Shown::Nowhere, None);
                }
                (seq, frame_id) = (ready_seq, ready_frame);
            }
            size = Some(ready_size);
        }
    }
    let Some(frame) = frame else {
        node.seen = seq;
        let was = std::mem::replace(&mut node.ready, false);
        if let Some(tile) = node.tile.take() {
            tile.retire();
            touch.push(id);
        }
        return (Shown::Nowhere, was.then_some(Event::Lost(lost)));
    };
    if !node.visible {
        return (Shown::Nowhere, None);
    }
    let size = size.unwrap_or_else(|| frame.size());
    let fits = node
        .tile
        .as_ref()
        .is_some_and(|t| t.window() == here && t.size() == size);
    if !fits {
        if let Some(old) = node.tile.take() {
            old.retire();
        }
        node.tile = Some(LiveTile::new(here, size.0, size.1));
    }
    let tile = node.tile.as_mut().expect("just made");
    #[cfg(not(any(windows, target_os = "macos")))]
    let () = texture;
    let wrote = match frame {
        Frame::Cpu {
            bgra,
            halves: Some(halves),
            ..
        } if hdr => tile
            .write_rgba16f(window, &halves, true)
            .unwrap_or_else(|| tile.write_bgra(window, bgra.to_vec(), true)),
        Frame::Cpu { bgra, .. } => tile.write_bgra(window, bgra.to_vec(), true),
        #[cfg(any(windows, target_os = "macos"))]
        _ => {
            let written = match texture {
                #[cfg(windows)]
                Some(Gpu::D3D11(texture)) => {
                    tile.write_gpu(window, gpui::GpuTextureHandle(texture.as_raw()), true)
                }
                #[cfg(windows)]
                Some(Gpu::D3D12(ready)) if hdr && ready.is_half_float() => {
                    let handle = ready.handle();
                    tile.write_d3d12(window, handle, Arc::new(ready), true)
                }
                #[cfg(windows)]
                Some(Gpu::D3D12(ready)) => match ready.for_atlas() {
                    Ok(ready) => {
                        let handle = ready.handle();
                        tile.write_d3d12(window, handle, Arc::new(ready), true)
                    }
                    Err(error) => Err(format!(
                        "its half-float frame could not be mapped for the window: {error}"
                    )),
                },
                #[cfg(target_os = "macos")]
                Some(Gpu::Metal(ready)) if hdr && ready.is_half_float() => {
                    let (handle, wait) = (ready.handle(), ready.ready());
                    tile.write_metal(window, handle, wait, Arc::new(ready), true)
                }
                #[cfg(target_os = "macos")]
                Some(Gpu::Metal(ready)) => match ready.for_atlas() {
                    Ok(ready) => {
                        let (handle, wait) = (ready.handle(), ready.ready());
                        tile.write_metal(window, handle, wait, Arc::new(ready), true)
                    }
                    Err(error) => Err(format!(
                        "its half-float frame could not be mapped for the window: {error}"
                    )),
                },
                None => return (Shown::Nowhere, None),
            };
            match written {
                Ok(shown) => {
                    node.refused = None;
                    shown
                }
                Err(why) => {
                    node.seen = seq;
                    refused(node, why);
                    return (Shown::Nowhere, None);
                }
            }
        }
    };
    node.seen = seq;
    node.frame = frame_id;
    node.surface.lock_state().note_shown(seq);
    let first = !std::mem::replace(&mut node.ready, true);
    let shown = if !fits || first || wrote == Shown::Draw {
        touch.push(id);
        Shown::Draw
    } else {
        wrote
    };
    (shown, first.then_some(Event::Ready(size.0, size.1)))
}

/// For a frame copied on the surface's own queue: whether the copy has run, and the frame's `seq`,
/// id and size.
#[cfg(any(windows, target_os = "macos"))]
fn copied(texture: &Option<Gpu>) -> Option<(bool, (u64, u64, (u32, u32)))> {
    match texture {
        #[cfg(windows)]
        Some(Gpu::D3D12(ready)) => Some((ready.done(), ready.frame())),
        #[cfg(target_os = "macos")]
        Some(Gpu::Metal(ready)) => Some((ready.done(), ready.frame())),
        _ => None,
    }
}

#[cfg(not(any(windows, target_os = "macos")))]
fn copied(_: &()) -> Option<(bool, (u64, u64, (u32, u32)))> {
    None
}

/// Logged once per surface node and reason.
#[cfg(any(windows, target_os = "macos"))]
fn refused(node: &mut Node, why: String) {
    if node.refused.as_deref() == Some(why.as_str()) {
        return;
    }
    let msg = format!(
        "<sk-surface> \"{}\": a GPU frame is not shown: {why}",
        node.surface.name()
    );
    log::warn!("{msg}");
    crate::log::record("window", msg.clone());
    crate::render::warn_compat("surface-gpu-upload", &msg);
    node.refused = Some(why);
}

/// `surfacedisplaychange`'s `key`: "hdr" when the display is in HDR, "output-hdr" when the window
/// draws HDR, and "vrr" or "fixed" when its refresh is known to be variable or not.
fn display_flags(on: &SurfaceDisplay) -> String {
    let refresh = match on.variable_refresh {
        None => None,
        Some(false) => Some("fixed"),
        Some(true) => Some("vrr"),
    };
    [
        on.hdr.then_some("hdr"),
        on.output_hdr.then_some("output-hdr"),
        refresh,
    ]
    .into_iter()
    .flatten()
    .collect::<Vec<_>>()
    .join(" ")
}

fn dispatch(events: Vec<(u32, Event)>, window: &mut Window, cx: &mut App) {
    for (id, event) in events {
        let listeners = tree::read_tree(|t| t.node(id).map(|n| n.listeners)).unwrap_or(0);
        let flags;
        let (code, payload) = match event {
            Event::Resize(w, h, scale) => (
                tree::EV_SURFACERESIZE,
                EventPayload {
                    a: w as f64,
                    b: h as f64,
                    c: scale as f64,
                    d: 0.0,
                    key: None,
                },
            ),
            Event::Ready(w, h) => (
                tree::EV_SURFACEREADY,
                EventPayload {
                    a: w as f64,
                    b: h as f64,
                    c: 0.0,
                    d: 0.0,
                    key: None,
                },
            ),
            Event::Lost(reason) => (
                tree::EV_SURFACELOST,
                EventPayload {
                    a: 0.0,
                    b: 0.0,
                    c: 0.0,
                    d: 0.0,
                    key: Some(reason),
                },
            ),
            Event::Display(on) => (
                tree::EV_SURFACEDISPLAYCHANGE,
                EventPayload {
                    a: on.headroom as f64,
                    b: on.potential_headroom as f64,
                    c: on.refresh_hz.unwrap_or(0.0) as f64,
                    d: on.sdr_white_nits.unwrap_or(0.0) as f64,
                    key: Some({
                        flags = display_flags(&on);
                        &flags
                    }),
                },
            ),
        };
        if listeners & (1u64 << code) != 0 {
            crate::render::js_dispatch_app(id, code, &payload, window, cx);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::{FrameDropReason, FrameEvent};
    use super::*;

    #[test]
    fn a_node_follows_its_name_and_reports_physical_pixels() {
        let id = 0x7ffe_0001;
        assert!(note_build(id, 7, "tests:node-a").is_none(), "no frame yet");
        note_painted(
            id,
            100.5,
            50.0,
            1.5,
            #[cfg(target_os = "macos")]
            None,
        );
        let painted = NODES.with(|m| m.borrow()[&id].painted);
        assert_eq!(painted, (151, 75, 1.5));
        note_build(id, 7, "tests:node-b");
        let (name, painted) = NODES.with(|m| {
            let n = &m.borrow()[&id];
            (n.surface.name().to_string(), n.painted)
        });
        assert_eq!(name, "tests:node-b");
        assert_eq!(painted, (0, 0, 1.0), "a new surface starts unmeasured");
        note_build(id, 7, "");
        assert_eq!(count_of(id), 0, "no name, no node");
    }

    #[test]
    fn a_surface_in_the_document_is_attached_in_every_feature_set() {
        let (inside, outside) = tree::with_tree(|t| {
            let root = t.create_root();
            let tag = t.intern("sk-surface");
            let inside = t.create_element(tag);
            t.insert(inside, root, 0);
            (inside, t.create_element(tag))
        });
        assert!(attached(inside));
        assert!(!attached(outside));
    }

    #[test]
    fn an_hdr_picture_follows_a_headroom_ramp_every_frame_while_reports_stay_spaced() {
        let (id, win) = (0x7ffe_0003, 0x7ffe);
        note_build(id, win, "tests:ramp");
        NODES.with(|m| {
            let mut m = m.borrow_mut();
            let node = m.get_mut(&id).expect("built");
            (node.hdr, node.ready) = (true, true);
        });
        let start = std::time::Instant::now();
        let frames = 192;
        let mut reports = Vec::new();
        for f in 0..=frames {
            let headroom = 1.0 + 5.11 * f as f64 / frames as f64;
            let at = start + std::time::Duration::from_micros(f * 1_000_000 / 120);
            display::sample_edr(win, headroom, at);
            let mut touch = Vec::new();
            NODES.with(|m| follow_headroom(id, m.borrow_mut().get_mut(&id).unwrap(), &mut touch));
            let painted = paint_headroom(id);
            assert!(
                (painted - headroom as f32).abs() <= 1e-3 * painted,
                "frame {f}: painted at {painted}, the screen allows {headroom}"
            );
            assert!(f == 0 || touch == [id], "frame {f} is not repainted");
            let reported = display::of(win).output_headroom;
            if reports.last() != Some(&reported) {
                reports.push(reported);
            }
        }
        assert!(reports.len() <= 10, "{reports:?}");
        display::forget(win);
        note_build(id, win, "");
    }

    #[test]
    fn a_half_float_surface_asks_for_hdr_output_once_it_holds_a_frame() {
        let (id, win) = (0x7ffe_0004, 0x7ffd);
        note_build(id, win, "tests:idle-hdr");
        let surface = NODES.with(|m| {
            let mut m = m.borrow_mut();
            let node = m.get_mut(&id).expect("built");
            node.visible = true;
            node.surface.clone()
        });
        surface.set_format(SurfaceFormat::Rgba16Float);
        assert_eq!(wanted_headroom(win, 4.0), 1.0, "no frame yet");
        surface
            .present_rgba16f(&[0x3c00; 4], 1, 1, 4)
            .expect("presented");
        assert_eq!(wanted_headroom(win, 4.0), 4.0);
        surface.clear();
        assert_eq!(wanted_headroom(win, 4.0), 1.0, "cleared");
        note_build(id, win, "");
    }

    #[test]
    fn a_proxy_is_read_only_in_the_window_whose_backdrops_sampled_it() {
        let (a, b) = (0x7ffe_0005, 0x7ffe_0006);
        let (one, two) = (0x7ffc, 0x7ffb);
        let s = super::super::surface("tests:proxy-reads");
        let picture = note_proxy(a, one, &s, 100.4, 50.0, 2.0);
        assert_eq!(picture.size, (0, 0), "no frame yet");
        note_proxy(b, two, &s, 10.0, 10.0, 1.0);
        let read = |id: u32| NODES.with(|m| m.borrow()[&id].read);
        note_reads(one, &[u64::from(a), u64::from(b)]);
        assert!(read(a));
        assert!(!read(b), "window one's reads say nothing about window two");
        note_reads(two, &[u64::from(b)]);
        note_reads(one, &[]);
        assert!(!read(a));
        assert!(read(b));
        assert_eq!(NODES.with(|m| m.borrow()[&a].painted), (100, 50, 2.0));
        NODES.with(|m| {
            let mut m = m.borrow_mut();
            m.remove(&a).map(forget);
            m.remove(&b).map(forget);
        });
    }

    #[test]
    fn a_proxy_frame_is_presented_by_the_page_present_that_composited_it() {
        let (id, win, present_window) = (0x7ffe_0007, 0x7ffa, 0x7ffe_7700);
        let s = super::super::surface("tests:proxy-frames");
        note_proxy(id, win, &s, 64.0, 64.0, 1.0);
        let heard = Arc::new(std::sync::Mutex::new(Vec::new()));
        let sink = heard.clone();
        let _watch = s.on_frame(move |e| sink.lock().unwrap().push(e));
        let unread = s.present_bgra(&[0; 4], 1, 1, 4).unwrap();
        feedback::deliver();
        assert_eq!(
            std::mem::take(&mut *heard.lock().unwrap()),
            [FrameEvent::dropped(unread, FrameDropReason::Hidden)],
            "no backdrop reads it"
        );
        note_reads(win, &[u64::from(id)]);
        s.set_info(SurfaceInfo {
            visible: true,
            ..SurfaceInfo::default()
        });
        let skipped = s.present_bgra(&[0; 4], 1, 1, 4).unwrap();
        let composited = s.present_bgra(&[0; 4], 1, 1, 4).unwrap();
        let key = feedback::key(s.shared());
        feedback::with(|t| {
            t.settle(key, composited);
            t.pin(key, composited, present_window);
            t.carry(present_window, HashMap::from([(key, composited)]));
            t.queued(present_window, 1);
            t.resolved(
                present_window,
                1,
                gpui::present_feedback::PresentOutcome::Displayed {
                    at_ns: 5000,
                    refresh_ns: 1000,
                    exact: true,
                },
                |_| None,
            );
        });
        feedback::deliver();
        let heard = std::mem::take(&mut *heard.lock().unwrap());
        assert_eq!(
            heard[0],
            FrameEvent::dropped(skipped, FrameDropReason::Superseded)
        );
        assert!(
            matches!(heard[1], FrameEvent::Presented { frame, shown_at: 5000, .. } if frame == composited),
            "{heard:?}"
        );
        NODES.with(|m| m.borrow_mut().remove(&id).map(forget));
    }

    #[test]
    fn a_surface_both_shown_and_pictured_is_visible_and_wakes_while_either_use_is() {
        let (element, picture, win) = (0x7ffe_000a, 0x7ffe_000b, 0x7ff9);
        let s = super::super::surface("tests:both");
        note_build(element, win, "tests:both");
        note_proxy(picture, win, &s, 64.0, 64.0, 1.0);
        let tick = |shown: Option<bool>, read: bool| {
            NODES.with(|m| {
                let mut m = m.borrow_mut();
                match shown {
                    Some(shown) => m.get_mut(&element).expect("built").visible = shown,
                    None => {
                        m.remove(&element).map(forget);
                    }
                }
                m.get_mut(&picture).expect("painted").visible = read;
            });
            let mut infos = HashMap::new();
            NODES.with(|m| {
                for node in m.borrow().values() {
                    if Arc::ptr_eq(node.surface.shared(), s.shared()) {
                        let info = infos.entry(Arc::as_ptr(s.shared())).or_default();
                        fold_info(info, node, SurfaceDisplay::default(), Default::default());
                    }
                }
            });
            publish_infos(vec![s.clone()], &infos);
            (s.info().visible, s.wants_frames())
        };
        assert_eq!(
            tick(Some(true), false),
            (true, true),
            "the element is on screen"
        );
        assert_eq!(
            tick(Some(false), true),
            (true, true),
            "a backdrop reads the picture"
        );
        assert_eq!(
            tick(Some(false), false),
            (false, true),
            "neither, but an element still wakes as one"
        );
        assert_eq!(tick(None, false), (false, false), "only an unread picture");
        assert_eq!(tick(None, true), (true, true), "only a read picture");
        NODES.with(|m| m.borrow_mut().remove(&picture).map(forget));
    }

    fn count_of(id: u32) -> usize {
        NODES.with(|m| usize::from(m.borrow().contains_key(&id)))
    }
}
