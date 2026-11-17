//! The element side of `<sk-surface>`: per node, the [`LiveTile`] the surface's frame is written
//! into, the painted size reported to the producer and to JS, and the events.

use std::cell::RefCell;
use std::collections::HashMap;
use std::sync::Arc;

use gpui::{App, RenderImage, Window};

use super::{Frame, SurfaceHandle, SurfaceInfo};
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
    /// The content box, physical px, and the scale, from the last paint.
    painted: (u32, u32, f32),
    reported: (u32, u32),
    visible: bool,
    ready: bool,
}

thread_local! {
    static NODES: RefCell<HashMap<u32, Node>> = RefCell::new(HashMap::new());
    /// Events a pop-out's draw raised, for the next primary tick to dispatch.
    static DEFERRED: RefCell<Vec<(u32, Event)>> = const { RefCell::new(Vec::new()) };
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
            painted: (0, 0, 1.0),
            reported: (0, 0),
            visible: false,
            ready: false,
        });
        node.window = win;
        node.tile
            .as_ref()
            .filter(|t| t.window() == win && node.ready)
            .map(LiveTile::image)
    })
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
            let attached = crate::video::attached(id);
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
                let info = infos.entry(Arc::as_ptr(node.surface.shared())).or_default();
                // The largest visible element sets the size; a hidden one's last size only stands in.
                let rank = |visible: bool, w: u32, h: u32| (visible, u64::from(w) * u64::from(h));
                if rank(node.visible, node.painted.0, node.painted.1)
                    > rank(info.visible, info.width, info.height)
                {
                    info.width = node.painted.0;
                    info.height = node.painted.1;
                    info.scale = node.painted.2;
                }
                info.visible |= node.visible;
            }
            if node.window != here {
                draw |= node.visible && node.surface.lock_state().seq != node.seen;
                continue;
            }
            let (s, event) = write(window, here, id, node, &mut touch);
            shown = shown.max(s);
            events.extend(event.map(|e| (id, e)));
        }
    });
    live::note(shown);
    finish(touch, &mut draw);
    for surface in super::all() {
        let info = infos
            .get(&Arc::as_ptr(surface.shared()))
            .copied()
            .unwrap_or_default();
        surface.set_info(info);
    }
    dispatch(events, window, cx);
    draw
}

/// A pop-out's half of [`tick`], from its own draw: its nodes' tiles live in its atlas.
pub(crate) fn window_tick(window: &mut Window) {
    if !super::any() {
        return;
    }
    let here = windows::key(window);
    let mut touch = Vec::new();
    NODES.with(|m| {
        for (&id, node) in m.borrow_mut().iter_mut() {
            if node.window == here {
                if let (_, Some(event)) = write(window, here, id, node, &mut touch) {
                    DEFERRED.with(|d| d.borrow_mut().push((id, event)));
                }
            }
        }
    });
    let mut draw = false;
    finish(touch, &mut draw);
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
    touch: &mut Vec<u32>,
) -> (Shown, Option<Event>) {
    // One lock: a present on another thread may resize the surface's texture in between.
    let (seq, frame, lost, texture) = {
        #[cfg_attr(not(windows), allow(unused_mut))]
        let mut st = node.surface.lock_state();
        #[cfg(windows)]
        let texture = match st.frame {
            Some(Frame::D3D12 { .. }) => st.gpu12.ready().map(Gpu::D3D12),
            // Only a visible node's tick may draw a half-float frame's mapping.
            Some(Frame::Gpu { .. }) if node.visible && st.seq != node.seen => {
                match st.gpu_texture() {
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
        (st.seq, st.frame.clone(), st.lost, texture)
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
        Frame::Cpu { bgra, .. } => tile.write_bgra(window, bgra.to_vec(), true),
        #[cfg(any(windows, target_os = "macos"))]
        _ => {
            let written = match texture {
                #[cfg(windows)]
                Some(Gpu::D3D11(texture)) => {
                    tile.write_gpu(window, gpui::GpuTextureHandle(texture.as_raw()), true)
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

    fn count_of(id: u32) -> usize {
        NODES.with(|m| usize::from(m.borrow().contains_key(&id)))
    }
}
