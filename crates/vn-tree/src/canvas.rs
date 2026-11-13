//! `<canvas>` on screen: the bitmap a page draws (published by `canvas.ts` under the element's host
//! node id), fetched when it changed and painted into the element's box as a replaced element.
//!
//! An idle canvas costs nothing: the pump asks redjs one question a frame ([`tick`]) and repaints
//! only when a presented canvas drew. A canvas that keeps drawing is paint-only (`crate::live`); one
//! still for [`STILL`] is drawn from a filtered copy when shown smaller than its bitmap. Not
//! honoured: `image-rendering` (gpui samples sprites linearly).
//!
//! [`PixelSource`] keeps presentation independent of the backend (CPU 2D bitmap or a native GPU
//! frame); only explicit export may wait for a GPU readback on this thread.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use gpui::prelude::*;
use gpui::{div, AnyElement, Context, ImageSource, RenderImage, Window, WindowControlArea};
use vn_runtime::redjs::webapi::canvas2d as rjs;
#[cfg(feature = "webgpu")]
use vn_runtime::redjs::webapi::webgpu::canvas as gpu;

use super::{
    apply_computed_style, bind_events, focus_handle_for, gpui_object_fit, pointer_mask, tab_index_of,
    warn_compat, watch_focus, with_control_area,
};
use crate::tree::Tree;
use crate::windows::{self, WinKey};


#[derive(Clone)]
pub(crate) enum Pixels {
    /// A bitmap uploaded into the window's atlas: BGRA with straight alpha, as gpui's atlas takes it.
    Atlas(Arc<RenderImage>),
    #[cfg(feature = "webgpu")]
    SolidBlack { width: u32, height: u32 },
    #[cfg(feature = "webgpu")]
    Texture(NativePixels),
}

#[cfg(feature = "webgpu")]
#[derive(Clone)]
pub(crate) struct NativePixels {
    frame: Arc<gpu::NativePresentation>,
    image: Arc<RenderImage>,
    uploaded: std::rc::Rc<Cell<Option<WinKey>>>,
}
#[cfg(feature = "webgpu")]
static NATIVE_UPLOADS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
#[cfg(feature = "webgpu")]
static NATIVE_FALLBACKS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
#[cfg(feature = "webgpu")]
pub(crate) fn native_stats() -> (u64, u64) {
    use std::sync::atomic::Ordering::Relaxed;
    (NATIVE_UPLOADS.load(Relaxed), NATIVE_FALLBACKS.load(Relaxed))
}

pub(crate) enum Fetch {
    Unpublished,
    Unchanged,
    /// The bitmap moved: its new generation, and what to paint (`None` for nothing to show, 0 × 0 or
    /// too large for the atlas).
    Changed(u64, Option<Pixels>),
}

/// Where a presented canvas's pixels come from: CPU rasterization or a completed WebGPU readback;
/// never waits for the GPU.
pub(crate) trait PixelSource {
    fn fetch(key: u64, since: u64) -> Fetch;
}

pub(crate) struct CpuBitmap;

/// gpui's atlas cannot hold a texture past this side (D3D11's limit).
const ATLAS_MAX_SIDE: u32 = 16384;

fn atlas_frame(generation: u64, width: u32, height: u32, rgba: &[u8]) -> Fetch {
    if width == 0 || height == 0 {
        return Fetch::Changed(generation, None);
    }
    if width > ATLAS_MAX_SIDE || height > ATLAS_MAX_SIDE {
        warn_compat(
            "canvas:atlas-size",
            &format!(
                "a {width}x{height} <canvas> is not painted: the host's sprite atlas stops at {ATLAS_MAX_SIDE}px a side \
                 (it still draws and reads back)",
            ),
        );
        return Fetch::Changed(generation, None);
    }
    let expected = width as usize * height as usize * 4;
    if rgba.len() != expected {
        return Fetch::Changed(generation, None);
    }
    let Some(buffer) = image::RgbaImage::from_raw(width, height, to_bgra(rgba)) else {
        return Fetch::Changed(generation, None);
    };
    let render = RenderImage::new(vec![image::Frame::new(buffer)]);
    Fetch::Changed(generation, Some(Pixels::Atlas(Arc::new(render))))
}

impl PixelSource for CpuBitmap {
    fn fetch(key: u64, since: u64) -> Fetch {
        let Some((generation, image)) = rjs::present(key, since) else {
            return if rjs::present_size(key).is_some() {
                Fetch::Unchanged
            } else {
                Fetch::Unpublished
            };
        };
        atlas_frame(generation, image.width, image.height, &image.data)
    }
}

/// A registered GPU context owns its key even before its first readback. Never fall through to a
/// stale 2D bitmap while a GPU frame is pending.
pub(crate) struct CanvasBitmap;

impl PixelSource for CanvasBitmap {
    fn fetch(key: u64, since: u64) -> Fetch {
        #[cfg(feature = "webgpu")]
        if gpu::present_size(key).is_some() {
            return match gpu::present(key, since) {
                Some((generation, frame)) => {
                    if frame.opaque_blank {
                        let size = gpu::present_size(key);
                        return match size {
                            Some((width, height)) if width > 0 && height > 0 =>
                                Fetch::Changed(generation, Some(Pixels::SolidBlack { width, height })),
                            _ => Fetch::Changed(generation, None),
                        };
                    }
                    if let Some(native) = &frame.native {
                        if frame.width > 0 && frame.height > 0 && frame.width <= ATLAS_MAX_SIDE && frame.height <= ATLAS_MAX_SIDE {
                            return Fetch::Changed(generation,Some(Pixels::Texture(NativePixels {
                                frame:native.clone(),
                                image:Arc::new(RenderImage::from_gpu(gpui::size(gpui::DevicePixels(frame.width as i32),gpui::DevicePixels(frame.height as i32)))),
                                uploaded:std::rc::Rc::new(Cell::new(None)),
                            })));
                        }
                        return Fetch::Changed(generation,None);
                    }
                    atlas_frame(generation, frame.width, frame.height, &frame.rgba)
                }
                None => Fetch::Unchanged,
            };
        }
        CpuBitmap::fetch(key, since)
    }
}

fn take_presented_changed() -> bool {
    let cpu = rjs::take_presented_changed();
    #[cfg(feature = "webgpu")]
    {
        let gpu = gpu::take_presented_changed();
        return cpu | gpu;
    }
    #[cfg(not(feature = "webgpu"))]
    cpu
}

/// A canvas that changes again within this of its last change is animating.
const MOVING: Duration = Duration::from_millis(250);
/// An animating canvas that has not changed for this long is still again.
const STILL: Duration = Duration::from_millis(300);

fn as_painted(pixels: Pixels, moving: bool) -> Pixels {
    match pixels {
        Pixels::Atlas(image) if moving => {
            Pixels::Atlas(Arc::try_unwrap(image).map(|i| Arc::new(i.dynamic())).unwrap_or_else(|a| a))
        }
        Pixels::Atlas(image) => Pixels::Atlas(image),
        #[cfg(feature = "webgpu")]
        solid @ Pixels::SolidBlack { .. } => solid,
        #[cfg(feature = "webgpu")]
        native @ Pixels::Texture(_) => native,
    }
}

fn still_copy(image: &RenderImage) -> Option<Arc<RenderImage>> {
    let size = image.size(0);
    let bytes = image.as_bytes(0)?.to_vec();
    let buffer = image::RgbaImage::from_raw(size.width.0 as u32, size.height.0 as u32, bytes)?;
    Some(Arc::new(RenderImage::new(vec![image::Frame::new(buffer)])))
}

fn to_bgra(rgba: &[u8]) -> Vec<u8> {
    let mut out = rgba.to_vec();
    for px in out.chunks_exact_mut(4) {
        px.swap(0, 2);
        let a = u32::from(px[3]);
        if a != 0 && a != 255 {
            for c in &mut px[..3] {
                *c = ((u32::from(*c) * 255 + a / 2) / a).min(255) as u8;
            }
        }
    }
    out
}

type Source = CanvasBitmap;


struct Entry {
    generation: u64,
    pixels: Option<Pixels>,
    window: WinKey,
    changed: Instant,
    moving: bool,
}

pub(crate) fn mem_stats() -> (usize, usize) {
    CANVASES.with(|c| {
        let c = c.borrow();
        let bytes = c
            .values()
            .filter_map(|e| match &e.pixels {
                Some(Pixels::Atlas(img)) => Some(img.as_bytes(0).map_or(0, <[u8]>::len)),
                #[cfg(feature = "webgpu")]
                Some(Pixels::SolidBlack { .. } | Pixels::Texture(_)) => None,
                None => None,
            })
            .sum();
        (c.len(), bytes)
    })
}

thread_local! {
    static CANVASES: RefCell<HashMap<u32, Entry>> = RefCell::new(HashMap::new());
    static BUILT: RefCell<HashMap<u32, WinKey>> = RefCell::new(HashMap::new());
    static STILL_AT: Cell<Option<Instant>> = const { Cell::new(None) };
}

/// Hands an image back to its window's atlas on that window's next frame, with its resampled copies.
fn retire_pixels(pixels: Option<Pixels>, window: WinKey) {
    let image = match pixels {
        Some(Pixels::Atlas(image)) => image,
        #[cfg(feature = "webgpu")]
        Some(Pixels::Texture(native)) => native.image,
        _ => return,
    };
    crate::resample::forget(&image);
    windows::queue_drops(window,[image]);
}

fn retire(entry: Entry) {
    retire_pixels(entry.pixels, entry.window);
}

fn current(id: u32, win: WinKey) -> Option<Pixels> {
    current_at(id, win, Instant::now())
}

fn current_at(id: u32, win: WinKey, now: Instant) -> Option<Pixels> {
    CANVASES.with(|m| {
        let mut m = m.borrow_mut();
        let since = m.get(&id).map_or(0, |e| e.generation);
        match Source::fetch(id as u64, since) {
            Fetch::Unchanged => {
                #[cfg(feature = "webgpu")]
                if let Some(e) = m.get_mut(&id).filter(|e| e.window != win) {
                    if let Some(Pixels::Texture(native)) = &mut e.pixels {
                        // A deferred drop in the old window must not remove a tile created if this
                        // canvas moves back before it runs.
                        let next = Arc::new(RenderImage::from_gpu(native.image.size(0)));
                        let previous = std::mem::replace(&mut native.image, next);
                        native.uploaded = std::rc::Rc::new(Cell::new(None));
                        windows::queue_drops(e.window,[previous]);
                        e.window = win;
                    }
                }
                if let Some(e) = m.get_mut(&id).filter(|e| e.moving && now.duration_since(e.changed) >= STILL) {
                    e.moving = false;
                    let still = match &e.pixels {
                        Some(Pixels::Atlas(image)) => still_copy(image),
                        #[cfg(feature = "webgpu")]
                        Some(Pixels::SolidBlack { .. } | Pixels::Texture(_)) => None,
                        None => None,
                    };
                    if let Some(still) = still {
                        let old = e.pixels.replace(Pixels::Atlas(still));
                        let old_window = std::mem::replace(&mut e.window, win);
                        retire_pixels(old, old_window);
                    }
                }
            }
            Fetch::Unpublished => {
                if let Some(old) = m.remove(&id) {
                    retire(old);
                }
            }
            Fetch::Changed(generation, pixels) => {
                let moving = m.get(&id).is_some_and(|e| now.duration_since(e.changed) < MOVING);
                if moving {
                    STILL_AT.with(|s| s.set(Some(now + STILL)));
                }
                let pixels = pixels.map(|p| as_painted(p, moving));
                if let Some(old) = m.insert(id, Entry { generation, pixels, window: win, changed: now, moving }) {
                    retire(old);
                }
            }
        }
        m.get(&id).and_then(|e| e.pixels.clone())
    })
}

pub(crate) fn gc(live: impl Fn(u32) -> bool) {
    let mut dead = std::collections::HashSet::new();
    BUILT.with(|b| {
        b.borrow_mut().retain(|&id, _| {
            let keep = live(id);
            if !keep {
                dead.insert(id);
            }
            keep
        })
    });
    let gone: Vec<Entry> = CANVASES.with(|m| {
        let mut m = m.borrow_mut();
        dead.extend(m.keys().copied().filter(|&id| !live(id)));
        dead.iter().filter_map(|id| m.remove(id)).collect()
    });
    // Keep the previous 2D registry lifetime: it follows the JS context, so a retained 2D canvas
    // can be reinserted without losing its presentation lookup.
    #[cfg(feature = "webgpu")]
    for id in dead {
        gpu::unregister_present(id as u64);
    }
    gone.into_iter().for_each(retire);
}

/// The pump's per-frame work; `true` asks for a draw, as a tree mutation would (also when an
/// animation just stopped).
pub(crate) fn tick(window: &mut Window) -> bool {
    let presented = take_presented_changed();
    let now = Instant::now();
    let still = STILL_AT.with(|s| match s.get() {
        Some(at) if now >= at => {
            s.set(None);
            true
        }
        _ => false,
    });
    if !presented {
        return still;
    }
    let here = windows::key(window);
    let built: Vec<(u32, WinKey)> = BUILT.with(|b| b.borrow().iter().map(|(&id, &w)| (id, w)).collect());
    let mut draw = still;
    for (id, win) in built {
        if win != here {
            draw = true;
            continue;
        }
        match fetch_live(id, here, now, window) {
            crate::live::Shown::Draw => draw = true,
            shown => crate::live::note(shown),
        }
    }
    draw
}

fn fetch_live(id: u32, win: WinKey, now: Instant, window: &mut Window) -> crate::live::Shown {
    use crate::live::{shown, Shown};
    let visible = crate::live::visible(id, win);
    let seen = |s: Shown| if visible { s } else { Shown::Nowhere };
    CANVASES.with(|m| {
        let mut m = m.borrow_mut();
        let since = m.get(&id).map_or(0, |e| e.generation);
        match Source::fetch(id as u64, since) {
            Fetch::Unchanged => Shown::Nowhere,
            Fetch::Unpublished => match m.remove(&id) {
                Some(old) => {
                    retire(old);
                    seen(Shown::Draw)
                }
                None => Shown::Nowhere,
            },
            Fetch::Changed(generation, pixels) => {
                #[cfg(feature = "webgpu")]
                let mut pixels = pixels;
                let moving = m.get(&id).is_some_and(|e| now.duration_since(e.changed) < MOVING);
                if moving {
                    STILL_AT.with(|s| s.set(Some(now + STILL)));
                }
                #[cfg(feature = "webgpu")]
                if let Some(Pixels::Texture(native)) = pixels.as_mut() {
                    if let Some(e) = m.get(&id).filter(|e|e.window == win) {
                        if let Some(Pixels::Texture(old)) = &e.pixels {
                            if old.image.size(0) == native.image.size(0) { native.image=old.image.clone(); }
                        }
                    }
                    let write=upload_native(native,window,win);
                    let same_id=m.get(&id).is_some_and(|e|matches!(&e.pixels,Some(Pixels::Texture(old)) if old.image.id==native.image.id));
                    let old=m.insert(id,Entry {generation,pixels,window:win,changed:now,moving});
                    if !same_id { if let Some(old)=old {retire(old);} }
                    return shown(write,visible);
                }
                if let (true, Some(Pixels::Atlas(fresh)), Some(e)) = (moving, &pixels, m.get_mut(&id)) {
                    let live = match &e.pixels {
                        Some(Pixels::Atlas(old)) if e.window == win && old.is_dynamic() && old.size(0) == fresh.size(0) => {
                            Some(old.id)
                        }
                        _ => None,
                    };
                    if let Some(old_id) = live {
                        let Some(Pixels::Atlas(fresh)) = pixels else { unreachable!() };
                        if let Ok(mut next) = Arc::try_unwrap(fresh) {
                            next.id = old_id;
                            let next = Arc::new(next.dynamic());
                            let write = window
                                .write_image_tile(&next, 0, gpui::TileSource::Bytes)
                                .unwrap_or(gpui::TileWrite::Placed);
                            e.pixels = Some(Pixels::Atlas(next));
                            e.generation = generation;
                            e.changed = now;
                            e.moving = true;
                            return shown(write, visible);
                        }
                        e.generation = 0;
                        return seen(Shown::Draw);
                    }
                }
                let pixels = pixels.map(|p| as_painted(p, moving));
                if let Some(old) = m.insert(id, Entry { generation, pixels, window: win, changed: now, moving }) {
                    retire(old);
                }
                seen(Shown::Draw)
            }
        }
    })
}

#[cfg(feature = "webgpu")]
fn upload_native(native: &NativePixels, window: &mut Window, win: WinKey) -> gpui::TileWrite {
    let mut write = gpui::TileWrite::Refused;
    // A COM reference alone does not retain wgpu's allocation.
    struct FrameLease {
        _frame: Arc<dyn vn_runtime::redjs::webapi::webgpu::canvas::NativeFrame>,
    }
    let owner = Arc::new(FrameLease { _frame: native.frame.frame.clone() });
    let result = native.frame.frame.copy_d3d12(&mut |resource, ready_fence, ready_value| {
        match window.write_image_tile_d3d12(
            &native.image, 0,
            gpui::D3D12TextureHandle { resource, ready_fence, ready_value }, owner.clone(),
        ) {
            Ok(result) => write = result,
            Err(error) => warn_compat("canvas:native-d3d12-copy", &format!("D3D12 canvas import failed: {error:#}")),
        }
        write != gpui::TileWrite::Refused
    });
    if result == Ok(true) && write != gpui::TileWrite::Refused {
        native.uploaded.set(Some(win));
        NATIVE_UPLOADS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        return write;
    }
    let result = native.frame.frame.copy_d3d11(&mut |texture| {
        write = window
            .write_image_tile(
                &native.image,
                0,
                gpui::TileSource::Gpu(gpui::GpuTextureHandle(texture)),
            )
            .unwrap_or(gpui::TileWrite::Refused);
        write != gpui::TileWrite::Refused
    });
    if result == Ok(true) && write != gpui::TileWrite::Refused {
        native.uploaded.set(Some(win));
        NATIVE_UPLOADS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        write
    } else {
        gpui::TileWrite::Refused
    }
}
#[cfg(feature = "webgpu")]
fn native_image(
    id: u32,
    native: NativePixels,
    window: &mut Window,
    win: WinKey,
) -> Option<Arc<RenderImage>> {
    if native.uploaded.get() == Some(win) && window.has_image_atlas_tile(&native.image, 0) {
        return Some(native.image);
    }
    if upload_native(&native, window, win) != gpui::TileWrite::Refused {
        return Some(native.image);
    }
    NATIVE_FALLBACKS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    warn_compat(
        "canvas:native-import",
        "native GPU canvas import was refused; falling back to an explicit CPU snapshot",
    );
    let snapshot = native.frame.snapshot().ok()?;
    let Fetch::Changed(_, Some(Pixels::Atlas(cpu))) =
        atlas_frame(0, snapshot.width, snapshot.height, &snapshot.rgba)
    else {
        return None;
    };
    CANVASES.with(|map| {
        if let Some(e) = map.borrow_mut().get_mut(&id) {
            let old = e.pixels.replace(Pixels::Atlas(cpu.clone()));
            retire_pixels(old, e.window);
            e.window = win;
        }
    });
    Some(cpu)
}


fn dimension(raw: Option<&str>, fallback: u32) -> u32 {
    let Some(raw) = raw else { return fallback };
    let s = raw.trim_start_matches([' ', '\t', '\n', '\x0c', '\r']);
    let s = s.strip_prefix('+').unwrap_or(s);
    let digits = &s[..s.find(|c: char| !c.is_ascii_digit()).unwrap_or(s.len())];
    match digits.parse::<u64>() {
        Ok(v) if v <= i32::MAX as u64 => v as u32,
        _ => fallback,
    }
}

fn intrinsic(t: &Tree, id: u32) -> (u32, u32) {
    (dimension(t.prop(id, "width"), 300), dimension(t.prop(id, "height"), 150))
}

/// gpui `Img`'s sizing rule for a box holding an `iw` × `ih` image.
fn size_as_replaced(style: &mut gpui::StyleRefinement, iw: f32, ih: f32, rem: gpui::Pixels) {
    use gpui::{DefiniteLength, Length};
    let auto = |l: Option<Length>| matches!(l, None | Some(Length::Auto));
    let absolute = |l: Option<Length>| match l {
        Some(Length::Definite(DefiniteLength::Absolute(a))) => Some(f32::from(a.to_pixels(rem))),
        _ => None,
    };
    if iw > 0.0 && ih > 0.0 && style.aspect_ratio.is_none() {
        style.aspect_ratio = Some(iw / ih);
    }
    if auto(style.size.width) {
        let w = match absolute(style.size.height) {
            Some(h) if ih > 0.0 => iw * h / ih,
            _ => iw,
        };
        style.size.width = Some(gpui::px(w).into());
    }
    if auto(style.size.height) {
        let h = match absolute(style.size.width) {
            Some(w) if iw > 0.0 => ih * w / iw,
            _ => ih,
        };
        style.size.height = Some(gpui::px(h).into());
    }
}

#[cfg(feature = "webgpu")]
fn solid_bounds(
    fit: &gpui::ObjectFit,
    bounds: gpui::Bounds<gpui::Pixels>,
    width: u32,
    height: u32,
) -> Option<gpui::Bounds<gpui::Pixels>> {
    if width == 0 || height == 0 || bounds.size.width <= gpui::px(0.) || bounds.size.height <= gpui::px(0.) {
        return None;
    }
    if !f32::from(bounds.size.width).is_finite() || !f32::from(bounds.size.height).is_finite() {
        return None;
    }
    let size = gpui::size(
        gpui::DevicePixels(i32::try_from(width).ok()?),
        gpui::DevicePixels(i32::try_from(height).ok()?),
    );
    let fitted = if matches!(fit, gpui::ObjectFit::None) {
        // GPUI's None helper anchors top-left, so centre this intrinsic-size drawing buffer
        // explicitly.
        let size = gpui::size(gpui::px(width as f32), gpui::px(height as f32));
        gpui::Bounds::new(
            gpui::point(
                bounds.origin.x + (bounds.size.width - size.width) / 2.,
                bounds.origin.y + (bounds.size.height - size.height) / 2.,
            ),
            size,
        )
    } else {
        fit.get_bounds(bounds, size)
    };
    Some(fitted)
}

#[cfg(feature = "webgpu")]
fn solid_black(width: u32, height: u32, fit: gpui::ObjectFit) -> AnyElement {
    gpui::canvas(
        |_, _, _| (),
        move |bounds, (), window, _| {
            if let Some(fitted) = solid_bounds(&fit, bounds, width, height) {
                let paint = fitted.intersect(&bounds);
                if paint.size.width > gpui::px(0.) && paint.size.height > gpui::px(0.) {
                    window.paint_quad(gpui::fill(paint, gpui::black()));
                }
            }
        },
    ).size_full().into_any_element()
}

pub(super) fn build<V: 'static>(
    t: &Tree,
    id: u32,
    node: &crate::tree::Node,
    window: &mut Window,
    cx: &mut Context<V>,
    now: std::time::Instant,
    control_area: Option<WindowControlArea>,
) -> AnyElement {
    let win = windows::key(window);
    #[cfg(feature = "webgpu")]
    gpu::register_present(id as u64);
    BUILT.with(|b| b.borrow_mut().insert(id, win));
    let pixels = current(id, win);

    let mut d = with_control_area(div().id(id as usize), id, control_area, win);
    d = crate::aria::apply(d, t, id, node);
    if let Some(cs) = &node.computed {
        d = apply_computed_style(d, cs, id, now);
    }
    let (iw, ih) = intrinsic(t, id);
    size_as_replaced(d.style(), iw as f32, ih as f32, window.rem_size());

    let fit = node
        .computed
        .as_ref()
        .and_then(|cs| cs.extras.object_fit)
        .map(gpui_object_fit)
        .unwrap_or(gpui::ObjectFit::Fill);
    let picture = match pixels {
        Some(Pixels::Atlas(image)) => Some(gpui::img(ImageSource::Render(image))
            .object_fit(fit).size_full().into_any_element()),
        #[cfg(feature = "webgpu")]
        Some(Pixels::SolidBlack { width, height }) => Some(solid_black(width, height, fit)),
        #[cfg(feature = "webgpu")]
        Some(Pixels::Texture(native)) => native_image(id,native,window,win).map(|image|
            gpui::img(ImageSource::Render(image)).object_fit(fit).size_full().into_any_element()),
        None => None,
    };
    if let Some(picture) = picture {
        let picture = super::clip_replaced(picture, d.style(), window.rem_size());
        d = d.child(picture);
    }

    let mask = pointer_mask(node);
    let tab_index = tab_index_of(t, id);
    let focusable = tab_index.is_some();
    let styled_focus = node.computed.as_ref().is_some_and(|cs| cs.needs_focus_handle());
    let handle = (focusable || styled_focus).then(|| focus_handle_for(id, tab_index, cx));
    if let Some(h) = &handle {
        d = d.track_focus(h);
        if focusable {
            watch_focus(id, h, window, cx);
        }
        if let Some(cs) = &node.computed {
            if let Some(f) = cs.focus.clone() {
                d = d.focus(move |_| (*f).clone());
            }
            if let Some(f) = cs.focus_visible.clone() {
                d = d.focus_visible(move |_| (*f).clone());
            }
        }
    }
    let click_focus = handle.filter(|_| focusable);
    bind_events(d, id, mask, click_focus, focusable, true, cx).into_any_element()
}

#[cfg(test)]
mod tests {
    use super::*;
    use rjs::cmd::{Cmd, Image};
    use rjs::surface::{Surface, SurfaceRef};
    use std::sync::Mutex;

    #[test]
    fn width_and_height_parse_as_html_has_them() {
        assert_eq!(dimension(None, 300), 300);
        assert_eq!(dimension(Some("64"), 300), 64);
        assert_eq!(dimension(Some(" \t+12px"), 300), 12);
        assert_eq!(dimension(Some("0"), 150), 0);
        assert_eq!(dimension(Some("-4"), 150), 150);
        assert_eq!(dimension(Some("abc"), 150), 150);
        assert_eq!(dimension(Some("2147483648"), 300), 300);
    }

    #[test]
    fn the_box_sizes_as_an_img_of_the_same_pixels() {
        let rem = gpui::px(16.0);
        let px = |v: f32| Some(gpui::Length::from(gpui::px(v)));

        let mut s = gpui::StyleRefinement::default();
        size_as_replaced(&mut s, 300.0, 150.0, rem);
        assert_eq!((s.size.width, s.size.height), (px(300.0), px(150.0)));
        assert_eq!(s.aspect_ratio, Some(2.0));

        let mut s = gpui::StyleRefinement::default();
        s.size.width = px(100.0);
        size_as_replaced(&mut s, 300.0, 150.0, rem);
        assert_eq!(s.size.height, px(50.0));

        let mut s = gpui::StyleRefinement::default();
        s.size.width = px(20.0);
        s.size.height = px(20.0);
        s.aspect_ratio = Some(1.0);
        size_as_replaced(&mut s, 300.0, 150.0, rem);
        assert_eq!((s.size.width, s.size.height, s.aspect_ratio), (px(20.0), px(20.0), Some(1.0)));
    }

    #[test]
    fn premultiplied_rgba_becomes_straight_bgra() {
        assert_eq!(to_bgra(&[1, 2, 3, 255, 0, 0, 0, 0]), vec![3, 2, 1, 255, 0, 0, 0, 0]);
        assert_eq!(to_bgra(&[128, 128, 128, 128]), vec![255, 255, 255, 128]);
        assert_eq!(to_bgra(&[64, 0, 0, 64]), vec![0, 0, 255, 64]);
    }

    #[test]
    fn an_empty_or_incomplete_frame_clears_a_previous_atlas_image() {
        let generation = 1 << 63;
        assert!(
            matches!(atlas_frame(generation, 0, 5, &[]), Fetch::Changed(g, None) if g == generation)
        );
        assert!(
            matches!(atlas_frame(generation, 2, 1, &[0, 0, 0, 255]), Fetch::Changed(g, None) if g == generation)
        );
        let Fetch::Changed(g, Some(Pixels::Atlas(image))) =
            atlas_frame(generation, 1, 1, &[128, 0, 0, 128])
        else {
            panic!("a complete GPU readback must reach the atlas");
        };
        assert_eq!(
            g, generation,
            "the GPU generation domain survives presentation"
        );
        assert_eq!(image.as_bytes(0).unwrap(), &[0, 0, 255, 128]);
    }

    #[cfg(feature = "webgpu")]
    #[test]
    fn opaque_blank_has_intrinsics_but_no_bitmap_memory() {
        let id = 0xC0FF_EE02;
        let before = mem_stats();
        CANVASES.with(|m| m.borrow_mut().insert(id, Entry {
            generation: 1 << 63,
            pixels: Some(Pixels::SolidBlack { width: 8192, height: 4096 }),
            window: windows::UNBOUND,
            changed: Instant::now(),
            moving: false,
        }));
        assert_eq!(mem_stats(), (before.0 + 1, before.1));
        let pixels = CANVASES.with(|m| m.borrow_mut().remove(&id).unwrap().pixels);
        assert!(matches!(as_painted(pixels.unwrap(), true), Pixels::SolidBlack { width: 8192, height: 4096 }));
        assert_eq!(mem_stats(), before);
    }

    #[cfg(feature = "webgpu")]
    #[test]
    fn opaque_blank_obeys_object_fit_and_centres_intrinsic_size() {
        use gpui::{point, px, size, Bounds, ObjectFit};
        let bounds = Bounds::new(point(px(10.), px(20.)), size(px(200.), px(100.)));
        let fitted = |fit| solid_bounds(&fit, bounds, 100, 100).unwrap();
        assert_eq!(fitted(ObjectFit::Fill), bounds);
        assert_eq!(fitted(ObjectFit::Contain), Bounds::new(point(px(60.), px(20.)), size(px(100.), px(100.))));
        assert_eq!(fitted(ObjectFit::Cover), Bounds::new(point(px(10.), px(-30.)), size(px(200.), px(200.))));
        assert_eq!(fitted(ObjectFit::None), Bounds::new(point(px(60.), px(20.)), size(px(100.), px(100.))));
        assert_eq!(fitted(ObjectFit::ScaleDown), fitted(ObjectFit::None));
        assert_eq!(solid_bounds(&ObjectFit::ScaleDown, bounds, 400, 200), Some(bounds));
        assert!(solid_bounds(&ObjectFit::Fill, bounds, 0, 100).is_none());
        assert!(solid_bounds(&ObjectFit::Fill, Bounds::new(bounds.origin, size(px(0.), px(100.))), 100, 100).is_none());
    }

    #[test]
    fn the_cpu_source_uploads_only_what_changed() {
        let key = 0xC0FF_EE00;
        assert!(matches!(CpuBitmap::fetch(key, 0), Fetch::Unpublished));

        let surface: SurfaceRef = Arc::new(Mutex::new(Surface::new(2, 1, rjs::Options::default())));
        rjs::register_present(key, &surface);
        let put = |rgba: [u8; 4]| Cmd::Put { x: 0, y: 0, image: Arc::new(Image::from_premul(1, 1, rgba.to_vec(), true)) };
        surface.lock().unwrap().push(put([255, 0, 0, 255]));

        let Fetch::Changed(g1, Some(Pixels::Atlas(image))) = CpuBitmap::fetch(key, 0) else {
            panic!("a published bitmap is fetched");
        };
        assert_eq!(image.size(0), gpui::size(gpui::DevicePixels(2), gpui::DevicePixels(1)));
        assert_eq!(&image.as_bytes(0).unwrap()[..4], &[0, 0, 255, 255], "red, as BGRA");
        assert!(matches!(CpuBitmap::fetch(key, g1), Fetch::Unchanged));

        surface.lock().unwrap().push(put([0, 0, 255, 255]));
        let Fetch::Changed(g2, Some(Pixels::Atlas(image))) = CpuBitmap::fetch(key, g1) else {
            panic!("a draw is a change");
        };
        assert_ne!(g1, g2);
        assert_eq!(&image.as_bytes(0).unwrap()[..4], &[255, 0, 0, 255], "blue, as BGRA");

        surface.lock().unwrap().reset(0, 0);
        assert!(matches!(CpuBitmap::fetch(key, g2), Fetch::Changed(_, None)), "a 0x0 bitmap paints nothing");

        rjs::unregister_present(key);
        assert!(matches!(CpuBitmap::fetch(key, g2), Fetch::Unpublished));
    }

    #[test]
    fn an_animating_canvas_is_dynamic_until_it_is_still() {
        let id: u32 = 0xC0FF_EE01;
        let surface: SurfaceRef = Arc::new(Mutex::new(Surface::new(2, 1, rjs::Options::default())));
        rjs::register_present(id as u64, &surface);
        let put = |rgba: [u8; 4]| Cmd::Put { x: 0, y: 0, image: Arc::new(Image::from_premul(1, 1, rgba.to_vec(), true)) };
        let atlas = |p: Option<Pixels>| match p {
            Some(Pixels::Atlas(image)) => image,
            _ => panic!("a bitmap"),
        };
        let t0 = Instant::now();

        surface.lock().unwrap().push(put([255, 0, 0, 255]));
        let first = atlas(current_at(id, windows::UNBOUND, t0));
        assert!(!first.is_dynamic(), "a canvas that drew once is a still image");

        surface.lock().unwrap().push(put([0, 255, 0, 255]));
        let frame = atlas(current_at(id, windows::UNBOUND, t0 + Duration::from_millis(100)));
        assert!(frame.is_dynamic(), "drawing again within MOVING: a frame of an animation");
        let again = atlas(current_at(id, windows::UNBOUND, t0 + Duration::from_millis(200)));
        assert!(Arc::ptr_eq(&frame, &again), "unchanged and not yet still: the same frame");

        let still = atlas(current_at(id, windows::UNBOUND, t0 + Duration::from_millis(100) + STILL));
        assert!(!still.is_dynamic(), "still again: re-wrapped");
        assert_ne!(still.id, frame.id);
        assert_eq!(still.as_bytes(0), frame.as_bytes(0), "the same pixels");

        rjs::unregister_present(id as u64);
        assert!(current_at(id, windows::UNBOUND, t0 + Duration::from_secs(1)).is_none());
    }
}
