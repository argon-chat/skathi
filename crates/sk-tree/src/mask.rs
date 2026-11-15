//! CSS `mask`: the painter gpui calls ([`gpui::VnMask`]) to draw a mask's layers over the element's
//! border box. An SVG layer is rasterised as an alpha mask at the device size it is drawn at, so an
//! icon stays sharp at any scale; a raster image and a gradient paint as a background layer would.

use std::cell::RefCell;
use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::rc::Rc;
use std::sync::Arc;

use gpui::{
    point, px, size, App, Bounds, Corners, Hsla, ImageSource, Pixels, Resource, SharedString,
    Window,
};
use sk_css::{MaskStack, PaintLayer};

use crate::bgimage::{head, is_svg, load, tiles, Load};

/// Gives the element its `mask`, if it has one. `leaf`: it paints nothing but its own background.
pub(crate) fn apply(
    style: &mut gpui::StyleRefinement,
    id: u32,
    extras: &sk_css::Extras,
    leaf: bool,
) {
    let Some(stack) = extras.resolve_mask_stack() else {
        return;
    };
    let tint_leaf = leaf
        && !stack.luminance
        && stack
            .layers
            .iter()
            .all(|l| matches!(l, PaintLayer::Image { .. }));
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    id.hash(&mut hasher);
    format!("{stack:?}").hash(&mut hasher);
    let key = hasher.finish();
    let stack = Rc::new(stack);
    style.vn_mask = Some(gpui::VnMask {
        luminance: stack.luminance,
        tint_leaf,
        key,
        paint: Some(Rc::new(move |bounds, tint, window, cx| {
            paint(id, &stack, bounds, tint, window, cx)
        })),
    });
}

fn paint(
    id: u32,
    stack: &MaskStack,
    area: Bounds<Pixels>,
    tint: Option<Hsla>,
    window: &mut Window,
    cx: &mut App,
) {
    if area.size.width <= px(0.) || area.size.height <= px(0.) {
        return;
    }
    let clip = Some(gpui::ContentMask {
        bounds: area,
        ..Default::default()
    });
    window.with_content_mask(clip, |window| {
        for layer in &stack.layers {
            match layer {
                PaintLayer::Gradient(background) => window.paint_quad(gpui::quad(
                    area,
                    Corners::default(),
                    *background,
                    gpui::Edges::default(),
                    gpui::transparent_black(),
                    gpui::BorderStyle::default(),
                )),
                PaintLayer::Image {
                    url,
                    size: mask_size,
                    position,
                    repeat,
                } => {
                    let Some(source) = crate::img::background_source(id, url) else {
                        continue;
                    };
                    let svg = !stack.luminance && is_svg(&source, url);
                    let image = if svg {
                        match svg_mask(&source, window, cx) {
                            Load::Pending => continue,
                            Load::Failed => {
                                failed(url);
                                continue;
                            }
                            Load::Ready(mask) => Image::Svg(mask),
                        }
                    } else {
                        match load(&source, window, cx) {
                            Load::Pending => continue,
                            Load::Failed => {
                                failed(url);
                                continue;
                            }
                            Load::Ready(image) => Image::Raster(image, is_svg(&source, url)),
                        }
                    };
                    let natural = image.natural();
                    let frame = (
                        f32::from(area.origin.x),
                        f32::from(area.origin.y),
                        f32::from(area.size.width),
                        f32::from(area.size.height),
                    );
                    let Ok(tiles) = tiles(frame, natural, *mask_size, *position, *repeat) else {
                        crate::render::warn_compat(
                            "mask-image-too-many-tiles",
                            "mask-image: a layer would need too many tiles to cover its box, so \
                             it is left out and the element is hidden there. Give it a larger \
                             mask-size, or `no-repeat`",
                        );
                        continue;
                    };
                    for (x, y, w, h) in tiles {
                        let tile = Bounds {
                            origin: point(px(x), px(y)),
                            size: size(px(w), px(h)),
                        };
                        image.paint(area, tile, tint, window, cx);
                    }
                }
            }
        }
    });
}

fn failed(url: &str) {
    crate::render::warn_compat(
        &format!("mask-image-failed:{}", head(url)),
        &format!(
            "mask-image: url({}) could not be loaded — the layer counts as transparent, as in a \
             browser, so the element is hidden where it would show. The URL takes the same forms \
             and is resolved the same way as an `<img src>`'s",
            head(url)
        ),
    );
}

enum Image {
    Svg(Arc<SvgMask>),
    /// An image, and whether gpui rasterised it from an SVG at twice its size.
    Raster(Arc<gpui::RenderImage>, bool),
}

impl Image {
    fn natural(&self) -> Option<(f32, f32)> {
        let (w, h) = match self {
            Image::Svg(svg) => svg.natural,
            Image::Raster(image, svg) => {
                let raw = image.natural_size(0);
                let scale = if *svg {
                    gpui::SMOOTH_SVG_SCALE_FACTOR
                } else {
                    1.0
                };
                (raw.width.0 as f32 / scale, raw.height.0 as f32 / scale)
            }
        };
        (w > 0.0 && h > 0.0).then_some((w, h))
    }

    fn paint(
        &self,
        area: Bounds<Pixels>,
        tile: Bounds<Pixels>,
        tint: Option<Hsla>,
        window: &mut Window,
        cx: &mut App,
    ) {
        match self {
            Image::Svg(svg) => {
                // `preserveAspectRatio`'s default: the picture fits the tile, centred.
                let (nw, nh) = svg.natural;
                let scale = (f32::from(tile.size.width) / nw).min(f32::from(tile.size.height) / nh);
                let fitted = size(px(nw * scale), px(nh * scale));
                let at = tile.origin
                    + point(
                        (tile.size.width - fitted.width) / 2.,
                        (tile.size.height - fitted.height) / 2.,
                    );
                let color = tint.unwrap_or_else(gpui::white);
                let _ = window.paint_svg(
                    Bounds {
                        origin: at,
                        size: fitted,
                    },
                    svg.key.clone(),
                    Some(&svg.bytes),
                    gpui::TransformationMatrix::unit(),
                    color,
                    cx,
                );
            }
            Image::Raster(image, _) => {
                let paint = |window: &mut Window| {
                    window.paint_image(area, tile, Corners::default(), image.clone(), 0, false)
                };
                let _ = match tint {
                    Some(tint) => window.with_image_tint(tint, paint),
                    None => paint(window),
                };
            }
        }
    }
}

/// An SVG mask image: its bytes, the atlas key its rasters go under, and its natural size.
pub(crate) struct SvgMask {
    bytes: Vec<u8>,
    key: SharedString,
    natural: (f32, f32),
}

impl SvgMask {
    fn parse(bytes: Vec<u8>) -> Option<Arc<SvgMask>> {
        let tree = usvg::Tree::from_data(&bytes, &usvg::Options::default()).ok()?;
        let natural = (tree.size().width(), tree.size().height());
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        bytes.hash(&mut hasher);
        let key = format!("sk-mask:{:x}", hasher.finish()).into();
        Some(Arc::new(SvgMask {
            bytes,
            key,
            natural,
        }))
    }
}

thread_local! {
    /// The SVGs that came in memory (`data:`, `blob:`), by gpui image id.
    static INLINE: RefCell<HashMap<u64, Option<Arc<SvgMask>>>> = RefCell::new(HashMap::new());
}

fn svg_mask(source: &ImageSource, window: &mut Window, cx: &mut App) -> Load<Arc<SvgMask>> {
    let mask = match source {
        ImageSource::Resource(resource) => match window.use_asset::<SvgMaskLoader>(resource, cx) {
            None => return Load::Pending,
            Some(mask) => mask,
        },
        ImageSource::Image(image) => INLINE.with(|m| {
            let mut m = m.borrow_mut();
            // An animation swapping data: URLs must not grow this without bound.
            if m.len() >= 64 && !m.contains_key(&image.id) {
                m.clear();
            }
            m.entry(image.id)
                .or_insert_with(|| SvgMask::parse(image.bytes.clone()))
                .clone()
        }),
        _ => None,
    };
    match mask {
        Some(mask) => Load::Ready(mask),
        None => Load::Failed,
    }
}

enum SvgMaskLoader {}

impl gpui::Asset for SvgMaskLoader {
    type Source = Resource;
    type Output = Option<Arc<SvgMask>>;

    fn load(
        source: Self::Source,
        cx: &mut App,
    ) -> impl std::future::Future<Output = Self::Output> + Send + 'static {
        let client = cx.http_client();
        let assets = cx.asset_source().clone();
        async move {
            let bytes = match source {
                Resource::Path(path) => std::fs::read(path.as_ref()).ok()?,
                Resource::Uri(uri) => crate::fonts::http_bytes(client, uri.as_ref()).await.ok()?,
                Resource::Embedded(key) => assets.load(&key).ok().flatten()?.to_vec(),
            };
            SvgMask::parse(bytes)
        }
    }
}
