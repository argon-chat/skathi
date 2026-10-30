//! `<canvas>` on screen: the bitmap a page draws, painted into the element's
//! box (docs/RUNTIME.md §5.10, CV-1).
//!
//! The 2D context is redjs's (vendor/redjs `webapi/canvas2d`), and
//! packages/vue-native/src/canvas.ts publishes each element's bitmap under the
//! element's host node id. This module is the host half: it fetches the
//! bitmap when it changed and paints it as a replaced element.
//!
//! * **Box.** The intrinsic size is the bitmap's, which is the `width` /
//!   `height` attributes in CSS px (300 × 150 by default), whether or not a
//!   context exists yet. CSS `width`/`height` scale it, one of them alone keeps
//!   the aspect ratio, and `object-fit` places it (initial value `fill`). The
//!   sizing rule is gpui's `Img` rule, so a `<canvas>` sizes as an `<img>` of
//!   the same pixel size does.
//! * **Cost.** An idle canvas costs nothing a frame. The pump asks redjs one
//!   question a frame ([`tick`]: one atomic swap, whatever the number of
//!   canvases) and repaints only when a presented canvas drew. A build that
//!   runs anyway (something else moved) compares the surface's generation and
//!   reuses the image it uploaded. A canvas that drew costs one snapshot (the
//!   queued commands rasterized), a swizzle to BGRA and one atlas upload; the
//!   image it replaces goes back to the atlas on the next frame.
//! * **Not honoured:** `image-rendering` (gpui samples sprites linearly, so a
//!   `pixelated` canvas scaled up is smoothed). Doc line, no advisory: vn-css
//!   does not parse the property.
//!
//! It is a child module of `render` (declared there with `#[path]`) so that it
//! builds with the replaced-element helpers `<img>` and `<video>` use.
//!
//! ## The seam step 2 swaps
//!
//! [`PixelSource`] is where the pixels come from, and the only thing step 2
//! (a GPU raster backend, held for the owner's decision) changes. Today it is
//! [`CpuBitmap`]: redjs rasterizes with vello_cpu into memory, and the bitmap
//! is uploaded into the window's sprite atlas. Step 2 swaps in either a
//! **readback ring** (the GPU renders, the bitmap is mapped back a frame later
//! and takes the same atlas path) or a **shared texture** ([`Pixels`] gains a
//! variant the element paints without a copy). The element, the generation
//! check and the repaint signal stay as they are.

use std::cell::RefCell;
use std::collections::HashMap;
use std::sync::Arc;

use gpui::prelude::*;
use gpui::{div, AnyElement, Context, ImageSource, RenderImage, Window, WindowControlArea};
use vn_runtime::redjs::webapi::canvas2d as rjs;

use super::{
    apply_computed_style, bind_events, focus_handle_for, gpui_object_fit, pointer_mask, tab_index_of,
    wants_style_hover, warn_compat, watch_focus, with_control_area,
};
use crate::tree::{self, Tree};
use crate::windows::{self, WinKey};

// ---------------------------------------------------------------------------
// The pixel source: CPU bitmap now; readback ring or shared texture later
// ---------------------------------------------------------------------------

/// What a canvas paints. Step 2 grows this, not the element.
#[derive(Clone)]
pub(crate) enum Pixels {
    /// A bitmap uploaded into the painting window's sprite atlas. Premultiplied
    /// BGRA, as gpui's atlas takes it.
    Atlas(Arc<RenderImage>),
    // Step 2, shared-texture flavour: `Texture(..)`, a GPU texture the element
    // paints without a copy.
}

/// One answer from a [`PixelSource`].
pub(crate) enum Fetch {
    /// Nothing is published under the key: no context was made yet.
    Unpublished,
    /// The bitmap is still the one at generation `since`.
    Unchanged,
    /// The bitmap moved: its new generation, and what to paint (`None` for a
    /// bitmap with nothing to show, 0 × 0 or too large for the atlas).
    Changed(u64, Option<Pixels>),
}

/// Where a presented canvas's pixels come from: **the step-2 seam**. A CPU
/// bitmap now ([`CpuBitmap`]); a GPU readback ring or a shared texture later.
pub(crate) trait PixelSource {
    /// The bitmap published under `key`, compared against generation `since`
    /// (0: none seen yet).
    fn fetch(key: u64, since: u64) -> Fetch;
}

/// Step 1: redjs's vello_cpu bitmap, read from its presentation registry,
/// swizzled and uploaded as a `RenderImage`.
pub(crate) struct CpuBitmap;

/// gpui's sprite atlas cannot hold a texture past this side (D3D11's limit,
/// `gpui_windows::directx_atlas`).
const ATLAS_MAX_SIDE: u32 = 16384;

impl PixelSource for CpuBitmap {
    fn fetch(key: u64, since: u64) -> Fetch {
        let Some((generation, image)) = rjs::present(key, since) else {
            return if rjs::present_size(key).is_some() { Fetch::Unchanged } else { Fetch::Unpublished };
        };
        if image.width == 0 || image.height == 0 {
            return Fetch::Changed(generation, None);
        }
        if image.width > ATLAS_MAX_SIDE || image.height > ATLAS_MAX_SIDE {
            // CV-1 (docs/RUNTIME.md §5.10).
            warn_compat(
                "canvas:atlas-size",
                &format!(
                    "a {}x{} <canvas> is not painted: the host's sprite atlas stops at {ATLAS_MAX_SIDE}px a side \
                     (it still draws and reads back)",
                    image.width, image.height
                ),
            );
            return Fetch::Changed(generation, None);
        }
        let bgra = to_bgra(&image.data);
        let Some(buffer) = image::RgbaImage::from_raw(image.width, image.height, bgra) else {
            return Fetch::Changed(generation, None);
        };
        // `RgbaImage` is a byte container here, not a colour claim: the bytes
        // are BGRA, which is what the atlas uploads (vn-lottie does the same).
        let render = RenderImage::new(vec![image::Frame::new(buffer)]);
        Fetch::Changed(generation, Some(Pixels::Atlas(Arc::new(render))))
    }
}

/// Premultiplied RGBA (redjs) to premultiplied BGRA (gpui's atlas).
fn to_bgra(rgba: &[u8]) -> Vec<u8> {
    let mut out = rgba.to_vec();
    for px in out.chunks_exact_mut(4) {
        px.swap(0, 2);
    }
    out
}

/// The source this build paints from.
type Source = CpuBitmap;

// ---------------------------------------------------------------------------
// Per-node state
// ---------------------------------------------------------------------------

struct Entry {
    /// The generation `pixels` was fetched at.
    generation: u64,
    pixels: Option<Pixels>,
    /// The window whose atlas holds `pixels`, recorded when it was fetched:
    /// a tile goes back to the atlas it is in (docs/MULTIWINDOW.md §5.1).
    window: WinKey,
}

thread_local! {
    static CANVASES: RefCell<HashMap<u32, Entry>> = RefCell::new(HashMap::new());
}

/// Hands an entry's image back to its window's atlas, on that window's next
/// frame (the scene that painted it has been presented by then).
fn retire(entry: Entry) {
    if let Some(Pixels::Atlas(image)) = entry.pixels {
        windows::queue_drops(entry.window, [image]);
    }
}

/// What node `id` paints this frame: the image it painted last, unless the
/// bitmap moved since.
fn current(id: u32, win: WinKey) -> Option<Pixels> {
    CANVASES.with(|m| {
        let mut m = m.borrow_mut();
        let since = m.get(&id).map_or(0, |e| e.generation);
        match Source::fetch(id as u64, since) {
            Fetch::Unchanged => {}
            Fetch::Unpublished => {
                if let Some(old) = m.remove(&id) {
                    retire(old);
                }
            }
            Fetch::Changed(generation, pixels) => {
                if let Some(old) = m.insert(id, Entry { generation, pixels, window: win }) {
                    retire(old);
                }
            }
        }
        m.get(&id).and_then(|e| e.pixels.clone())
    })
}

/// Drops the state of canvases that left the tree, their images included.
pub(crate) fn gc(live: impl Fn(u32) -> bool) {
    let gone: Vec<Entry> = CANVASES.with(|m| {
        let mut m = m.borrow_mut();
        let dead: Vec<u32> = m.keys().copied().filter(|&id| !live(id)).collect();
        dead.into_iter().filter_map(|id| m.remove(&id)).collect()
    });
    gone.into_iter().for_each(retire);
}

/// The pump's per-frame question: did any on-screen canvas draw, resize, or
/// get its context since the last frame? `true` repaints, the same as a tree
/// mutation would. One atomic swap.
pub(crate) fn tick() -> bool {
    rjs::take_presented_changed()
}

// ---------------------------------------------------------------------------
// The element
// ---------------------------------------------------------------------------

/// HTML's "rules for parsing non-negative integers", with the attribute's
/// default: the same reading canvas.ts gives `width`/`height`.
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

/// The canvas's intrinsic size in CSS px: its bitmap's, which is the
/// attributes'. Past redjs's bitmap limit the bitmap is 0 × 0 but the box
/// keeps the attribute size, as in Chromium.
fn intrinsic(t: &Tree, id: u32) -> (u32, u32) {
    (dimension(t.prop(id, "width"), 300), dimension(t.prop(id, "height"), 150))
}

/// gpui `Img`'s sizing rule, for a box that holds an image of `iw` × `ih`:
/// an `auto` dimension takes the intrinsic one, or follows the other one's
/// absolute length through the aspect ratio, which is also set unless CSS set
/// one.
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

/// `<canvas>`: a styled box at the bitmap's intrinsic size, with the bitmap
/// painted over its content box.
///
/// Not focusable without `tabindex`, as in HTML. Fallback content (the
/// element's children) is not rendered: a browser with scripting shows the
/// bitmap instead.
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
    let pixels = current(id, win);

    let mut d = with_control_area(div().id(id as usize), id, control_area, win);
    d = crate::aria::apply(d, t, id, node);
    if let Some(cs) = &node.computed {
        d = apply_computed_style(d, cs, id, now);
    }
    let (iw, ih) = intrinsic(t, id);
    size_as_replaced(d.style(), iw as f32, ih as f32, window.rem_size());

    if let Some(Pixels::Atlas(image)) = pixels {
        let fit = node
            .computed
            .as_ref()
            .and_then(|cs| cs.extras.object_fit)
            .map(gpui_object_fit)
            // The CSS initial value, as for `<img>`.
            .unwrap_or(gpui::ObjectFit::Fill);
        d = d.child(
            gpui::img(ImageSource::Render(image))
                .object_fit(fit)
                .size_full()
                .into_any_element(),
        );
    }

    let mask = pointer_mask(node);
    let bit = |ev: u32| mask & (1u64 << ev) != 0;
    let tab_index = tab_index_of(t, id);
    let focusable = tab_index.is_some()
        || bit(tree::EV_KEYDOWN)
        || bit(tree::EV_KEYUP)
        || bit(tree::EV_FOCUS)
        || bit(tree::EV_BLUR);
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
    let style_hover = wants_style_hover(t, id, node);
    bind_events(d, id, mask, click_focus, focusable, true, style_hover, cx).into_any_element()
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

        // No CSS size: the bitmap size.
        let mut s = gpui::StyleRefinement::default();
        size_as_replaced(&mut s, 300.0, 150.0, rem);
        assert_eq!((s.size.width, s.size.height), (px(300.0), px(150.0)));
        assert_eq!(s.aspect_ratio, Some(2.0));

        // One CSS dimension: the other follows the aspect ratio.
        let mut s = gpui::StyleRefinement::default();
        s.size.width = px(100.0);
        size_as_replaced(&mut s, 300.0, 150.0, rem);
        assert_eq!(s.size.height, px(50.0));

        // Both: CSS wins, and its own aspect-ratio is kept.
        let mut s = gpui::StyleRefinement::default();
        s.size.width = px(20.0);
        s.size.height = px(20.0);
        s.aspect_ratio = Some(1.0);
        size_as_replaced(&mut s, 300.0, 150.0, rem);
        assert_eq!((s.size.width, s.size.height, s.aspect_ratio), (px(20.0), px(20.0), Some(1.0)));
    }

    #[test]
    fn premultiplied_rgba_becomes_bgra() {
        assert_eq!(to_bgra(&[1, 2, 3, 4, 10, 20, 30, 40]), vec![3, 2, 1, 4, 30, 20, 10, 40]);
    }

    /// Upload only when the surface moved: the second fetch at the same
    /// generation hands back nothing new, and a draw does.
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
}
