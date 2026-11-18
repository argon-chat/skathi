//! The element side of `<sk-surface>`: per node, the [`LiveTile`] the surface's frame is written
//! into, the painted size reported to the producer and to JS, and the events.

use std::cell::RefCell;
use std::collections::HashMap;
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
}

thread_local! {
    static NODES: RefCell<HashMap<u32, Node>> = RefCell::new(HashMap::new());
    /// Events a pop-out's draw raised, for the next primary tick to dispatch.
    static DEFERRED: RefCell<Vec<(u32, Event)>> = const { RefCell::new(Vec::new()) };
    /// Each window's native handle, as present feedback names it.
    static PRESENT_KEYS: RefCell<HashMap<WinKey, u64>> = RefCell::new(HashMap::new());
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
        let node = m.entry(id).or_insert_with(|| Node {
            surface: super::surface(name),
            window: win,
            tile: None,
            seen: 0,
            frame: 0,
            painted: (0, 0, 1.0),
            reported: (0, 0),
            visible: false,
            ready: false,
            display: None,
            headroom: 1.0,
            hdr: false,
        });
        node.window = win;
        node.tile
            .as_ref()
            .filter(|t| t.window() == win && node.ready)
            .map(LiveTile::image)
    })
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
        window.with_image_headroom(headroom, |window| self.1.paint(window, cx));
    }
}

/// Called from the paint with the content box in window pixels.
pub(crate) fn note_painted(id: u32, width: f32, height: f32, scale: f32) {
    NODES.with(|m| {
        if let Some(node) = m.borrow_mut().get_mut(&id) {
            node.painted = (
                (width * scale).round().max(0.0) as u32,
                (height * scale).round().max(0.0) as u32,
                scale,
            );
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
    let page_hidden = live::page_hidden();
    let here_shown = window.is_visible();
    let mut events = DEFERRED.with(|d| std::mem::take(&mut *d.borrow_mut()));
    let mut touch = Vec::new();
    let mut draw = false;
    let mut infos: HashMap<*const super::Shared, SurfaceInfo> = HashMap::new();
    let mut shown = Shown::Nowhere;
    NODES.with(|m| {
        for (&id, node) in m.borrow_mut().iter_mut() {
            let attached = attached(id);
            node.visible = attached
                && !page_hidden
                && (node.window != here || here_shown)
                && node.painted.0 > 0
                && node.painted.1 > 0
                && live::visible(id, node.window);
            if attached && node.painted.0 > 0 && (node.painted.0, node.painted.1) != node.reported {
                node.reported = (node.painted.0, node.painted.1);
                events.push((
                    id,
                    Event::Resize(node.painted.0, node.painted.1, node.painted.2),
                ));
            }
            if attached {
                let on = display::of(node.window);
                if on.known && node.display.is_none_or(|d| on.differs(&d)) {
                    node.display = Some(on);
                    events.push((id, Event::Display(on)));
                }
                let limit = crate::inherit::dynamic_range_limit(id);
                note_headroom(id, node, limit.headroom(on.output_headroom), &mut touch);
                let info = infos.entry(Arc::as_ptr(node.surface.shared())).or_default();
                // The largest visible element sets the size; a hidden one's last size only stands in.
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
            if node.window != here {
                draw |= node.visible && node.surface.lock_state().seq != node.seen;
            }
        }
    });
    let hdr = update_output(window, here);
    NODES.with(|m| {
        for (&id, node) in m.borrow_mut().iter_mut() {
            if node.window == here {
                let (s, event) = write(window, here, id, node, hdr, &mut touch);
                shown = shown.max(s);
                events.extend(event.map(|e| (id, e)));
            }
        }
    });
    live::note(shown);
    display::note_tick(infos.values().any(|info| info.visible));
    finish(touch, &mut draw);
    for surface in super::all() {
        let info = infos
            .get(&Arc::as_ptr(surface.shared()))
            .copied()
            .unwrap_or_default();
        surface.set_info(info);
    }
    carry(window, here);
    dispatch(events, window, cx);
    draw
}

/// Tells present feedback which frame of each surface `here`'s next present shows.
fn carry(window: &Window, here: WinKey) {
    if !feedback::watching() {
        return;
    }
    let Some(key) = PRESENT_KEYS.with(|k| {
        let mut keys = k.borrow_mut();
        if let Some(&key) = keys.get(&here) {
            return Some(key);
        }
        let key = feedback::window_key(window, here)?;
        keys.insert(here, key);
        Some(key)
    }) else {
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

/// Moves a node to the headroom it may now be painted with; a picture shown in HDR is repainted.
fn note_headroom(id: u32, node: &mut Node, headroom: f32, touch: &mut Vec<u32>) {
    if (headroom - node.headroom).abs() > 1e-3 * headroom {
        node.headroom = headroom;
        if node.hdr && node.ready {
            touch.push(id);
        }
    }
}

/// Puts `here` in HDR while a visible half-float surface in it may be brighter than SDR white.
fn update_output(window: &Window, here: WinKey) -> bool {
    let wanted = NODES.with(|m| {
        m.borrow().values().any(|n| {
            n.window == here
                && n.visible
                && n.headroom > 1.0
                && n.surface.format() == SurfaceFormat::Rgba16Float
        })
    });
    display::update_output(window, here, wanted)
}

/// A pop-out's half of [`tick`], from its own draw: its nodes' tiles live in its atlas.
pub(crate) fn window_tick(window: &mut Window) {
    if !super::any() {
        return;
    }
    let here = windows::key(window);
    let hdr = update_output(window, here);
    let mut touch = Vec::new();
    NODES.with(|m| {
        for (&id, node) in m.borrow_mut().iter_mut() {
            if node.window == here {
                if let (_, Some(event)) = write(window, here, id, node, hdr, &mut touch) {
                    DEFERRED.with(|d| d.borrow_mut().push((id, event)));
                }
            }
        }
    });
    let mut draw = false;
    finish(touch, &mut draw);
    carry(window, here);
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
    // One lock: a present on another thread may resize the surface's texture in between.
    let (seq, frame_id, frame, lost, texture) = {
        #[cfg_attr(not(windows), allow(unused_mut))]
        let mut st = node.surface.lock_state();
        #[cfg(windows)]
        let texture = match st.frame {
            Some(Frame::D3D12 { .. }) => st.gpu12.ready().map(Gpu::D3D12),
            // Only a visible node's tick may draw a half-float frame's mapping.
            Some(Frame::Gpu { .. }) if node.visible && st.seq != node.seen => {
                match st.gpu_texture(hdr) {
                    Ok(texture) => texture.map(Gpu::D3D11),
                    Err(error) => {
                        refused(&error);
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
        (st.seq, st.frame_id, st.frame.clone(), st.lost, texture)
    };
    if seq == node.seen {
        return (Shown::Nowhere, None);
    }
    // The copy into the surface's texture waits on the producer's fence on the GPU; handing it to
    // the atlas before it ran would stall the renderer's queue on the producer.
    #[cfg(windows)]
    if let Some(Gpu::D3D12(ready)) = &texture {
        if node.visible && !ready.done() {
            sk_runtime::frames::wake();
            return (Shown::Nowhere, None);
        }
    }
    #[cfg(target_os = "macos")]
    if let Some(Gpu::Metal(ready)) = &texture {
        if node.visible && !ready.done() {
            sk_runtime::frames::wake();
            return (Shown::Nowhere, None);
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
    let size = frame.size();
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
                    Err(error) => {
                        refused(&error);
                        return (Shown::Nowhere, None);
                    }
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
                    Err(error) => {
                        refused(&error);
                        return (Shown::Nowhere, None);
                    }
                },
                None => return (Shown::Nowhere, None),
            };
            match written {
                Some(shown) => shown,
                None => {
                    node.seen = seq;
                    crate::render::warn_compat(
                        "surface-gpu-upload",
                        "<sk-surface>: the renderer refused the surface's texture (SK_LOG=info \
                         names the reason); the frame is not shown",
                    );
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

/// A half-float frame whose 8-bit mapping could not be drawn.
#[cfg(any(windows, target_os = "macos"))]
fn refused(error: &super::SurfaceError) {
    crate::log::record("window", format!("<sk-surface>: {error}"));
    crate::render::warn_compat(
        "surface-gpu-map",
        "<sk-surface>: a half-float frame could not be mapped for the window (SK_LOG=info names \
         the reason); it is not shown",
    );
}

/// `surfacedisplaychange`'s `key`: "hdr" when the display is in HDR, then "vrr" or "fixed" when its
/// refresh is known to be variable or not.
fn display_flags(on: &SurfaceDisplay) -> &'static str {
    match (on.hdr, on.variable_refresh) {
        (false, None) => "",
        (true, None) => "hdr",
        (false, Some(false)) => "fixed",
        (true, Some(false)) => "hdr fixed",
        (false, Some(true)) => "vrr",
        (true, Some(true)) => "hdr vrr",
    }
}

fn dispatch(events: Vec<(u32, Event)>, window: &mut Window, cx: &mut App) {
    for (id, event) in events {
        let listeners = tree::read_tree(|t| t.node(id).map(|n| n.listeners)).unwrap_or(0);
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
                    key: Some(display_flags(&on)),
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
    use super::*;

    #[test]
    fn a_node_follows_its_name_and_reports_physical_pixels() {
        let id = 0x7ffe_0001;
        assert!(note_build(id, 7, "tests:node-a").is_none(), "no frame yet");
        note_painted(id, 100.5, 50.0, 1.5);
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

    fn count_of(id: u32) -> usize {
        NODES.with(|m| usize::from(m.borrow().contains_key(&id)))
    }
}
