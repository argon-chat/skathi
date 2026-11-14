//! `background-image: url()` — loaded through `<img src>`'s resolver and painted by sk-tree, under
//! the element's content.
//!
//! gpui has no image fill, so an element with a `url()` layer gets one extra absolutely positioned
//! child, first, that paints the whole stack bottom layer first; it takes no part in layout and no
//! hitbox. Differences: the painting area is the padding box, a relative URL resolves against the
//! app root (not the stylesheet), the image scrolls with content in a scroll container, `space` and
//! `round` repeat plainly, and an animated GIF shows its first frame.

use std::sync::Arc;

use gpui::{
    canvas, point, px, size, AnyElement, App, Bounds, Corners, ImageFormat, ImageSource,
    IntoElement, Pixels, RenderImage, Resource, Styled, Window,
};
use sk_css::{BackgroundSize, PaintLayer, TileLength};

/// More tiles than this in one layer (a 2px pattern over a large box) are not painted; the layer is
/// skipped with an advisory instead of flooding the scene.
const MAX_TILES: usize = 4096;

pub(crate) fn layer_element(
    id: u32,
    stack: Vec<PaintLayer>,
    radii: crate::render::PaddingRadii,
) -> AnyElement {
    let mut c = canvas(
        |_, _, _| (),
        move |bounds, (), window, cx| {
            let radii = radii.resolve(bounds.size, window.rem_size());
            paint_stack(id, &stack, bounds, radii, window, cx)
        },
    );
    let style = c.style();
    style.position = Some(gpui::Position::Absolute);
    let zero = || Some(gpui::Length::Definite(px(0.).into()));
    style.inset.top = zero();
    style.inset.right = zero();
    style.inset.bottom = zero();
    style.inset.left = zero();
    c.into_any_element()
}

fn paint_stack(
    id: u32,
    stack: &[PaintLayer],
    area: Bounds<Pixels>,
    radii: Corners<Pixels>,
    window: &mut Window,
    cx: &mut App,
) {
    if area.size.width <= px(0.) || area.size.height <= px(0.) {
        return;
    }
    for layer in stack {
        match layer {
            PaintLayer::Gradient(background) => {
                window.paint_quad(gpui::quad(
                    area,
                    radii,
                    *background,
                    gpui::Edges::default(),
                    gpui::transparent_black(),
                    gpui::BorderStyle::default(),
                ));
            }
            PaintLayer::Image {
                url,
                size: bg_size,
                position,
                repeat,
            } => {
                let Some(source) = crate::img::background_source(id, url) else {
                    continue; // the resolver has said why
                };
                let image = match load(&source, window, cx) {
                    Load::Pending => continue,
                    Load::Failed => {
                        crate::render::warn_compat(
                            &format!("bg-image-failed:{}", head(url)),
                            &format!(
                                "background-image: url({}) could not be loaded — the layer is not \
                                 painted, as a browser paints nothing for a broken image. The URL \
                                 takes the same forms and is resolved the same way as an \
                                 `<img src>`'s",
                                head(url)
                            ),
                        );
                        continue;
                    }
                    Load::Ready(image) => image,
                };
                let raw = image.natural_size(0);
                let scale = if is_svg(&source, url) {
                    gpui::SMOOTH_SVG_SCALE_FACTOR
                } else {
                    1.0
                };
                let natural = (raw.width.0 as f32 / scale, raw.height.0 as f32 / scale);
                let natural = (natural.0 > 0.0 && natural.1 > 0.0).then_some(natural);
                let frame = (
                    f32::from(area.origin.x),
                    f32::from(area.origin.y),
                    f32::from(area.size.width),
                    f32::from(area.size.height),
                );
                let tiles = match tiles(frame, natural, *bg_size, *position, *repeat) {
                    Ok(tiles) => tiles,
                    Err(count) => {
                        crate::render::warn_compat(
                            "bg-image-too-many-tiles",
                            &format!(
                                "background-image: a layer would need {count} tiles to cover its \
                                 box — more than {MAX_TILES}, so it is not painted. Give it a \
                                 larger background-size, or `no-repeat`"
                            ),
                        );
                        continue;
                    }
                };
                for (x, y, w, h) in tiles {
                    let tile = Bounds {
                        origin: point(px(x), px(y)),
                        size: size(px(w), px(h)),
                    };
                    let corners = corners_for(area, tile, radii);
                    let _ = window.paint_image(area, tile, corners, image.clone(), 0, false);
                }
            }
        }
    }
}

enum Load {
    Pending,
    Failed,
    Ready(Arc<RenderImage>),
}

fn load(source: &ImageSource, window: &mut Window, cx: &mut App) -> Load {
    match source {
        ImageSource::Resource(resource) => {
            match window.use_asset::<gpui::ImgResourceLoader>(resource, cx) {
                None => Load::Pending,
                Some(Ok(image)) => Load::Ready(image),
                Some(Err(_)) => Load::Failed,
            }
        }
        ImageSource::Image(image) => match image.clone().use_render_image(window, cx) {
            Some(image) => Load::Ready(image),
            None => Load::Pending,
        },
        _ => Load::Failed,
    }
}

/// gpui rasterises an SVG at twice its size for smoothness, so its natural size is half the decoded
/// one.
fn is_svg(source: &ImageSource, url: &str) -> bool {
    match source {
        ImageSource::Image(image) => image.format == ImageFormat::Svg,
        ImageSource::Resource(Resource::Path(p)) => {
            p.extension().is_some_and(|e| e.eq_ignore_ascii_case("svg"))
        }
        _ => crate::img::names_svg(url),
    }
}

fn head(url: &str) -> String {
    let mut s: String = url.chars().take(60).collect();
    if s.len() < url.len() {
        s.push('…');
    }
    s
}

/// The box's corner radii where the visible part of `tile` reaches a corner of the box, and square
/// corners elsewhere: gpui rounds the corners of each sprite it paints, and only the ones on the
/// box's own corners are round.
fn corners_for(
    area: Bounds<Pixels>,
    tile: Bounds<Pixels>,
    radii: Corners<Pixels>,
) -> Corners<Pixels> {
    let visible = area.intersect(&tile);
    let near = |a: Pixels, b: Pixels| (f32::from(a) - f32::from(b)).abs() < 0.5;
    let left = near(visible.left(), area.left());
    let right = near(visible.right(), area.right());
    let top = near(visible.top(), area.top());
    let bottom = near(visible.bottom(), area.bottom());
    Corners {
        top_left: if top && left { radii.top_left } else { px(0.) },
        top_right: if top && right {
            radii.top_right
        } else {
            px(0.)
        },
        bottom_right: if bottom && right {
            radii.bottom_right
        } else {
            px(0.)
        },
        bottom_left: if bottom && left {
            radii.bottom_left
        } else {
            px(0.)
        },
    }
}

pub(crate) fn tiles(
    area: (f32, f32, f32, f32),
    natural: Option<(f32, f32)>,
    bg_size: BackgroundSize,
    position: (TileLength, TileLength),
    repeat: (bool, bool),
) -> Result<Vec<(f32, f32, f32, f32)>, usize> {
    let (ax, ay, aw, ah) = area;
    let (tw, th) = tile_size((aw, ah), natural, bg_size);
    if !(tw > 0.0 && th > 0.0) || !tw.is_finite() || !th.is_finite() {
        return Ok(Vec::new());
    }
    let offset = |p: TileLength, free: f32| match p {
        TileLength::Pixels(v) => v,
        TileLength::Fraction(f) => free * f,
    };
    let x = offset(position.0, aw - tw);
    let y = offset(position.1, ah - th);
    let run = |origin: f32, tile: f32, extent: f32, repeats: bool| -> Vec<f32> {
        if !repeats {
            return vec![origin];
        }
        let first = origin - (origin / tile).ceil() * tile;
        let count = ((extent - first) / tile).ceil().max(0.0) as usize;
        (0..count).map(|i| first + i as f32 * tile).collect()
    };
    let (xs, ys) = (run(x, tw, aw, repeat.0), run(y, th, ah, repeat.1));
    let count = xs.len().saturating_mul(ys.len());
    if count > MAX_TILES {
        return Err(count);
    }
    let mut out = Vec::with_capacity(count);
    for &ty in &ys {
        for &tx in &xs {
            if tx + tw <= 0.0 || ty + th <= 0.0 || tx >= aw || ty >= ah {
                continue;
            }
            out.push((ax + tx, ay + ty, tw, th));
        }
    }
    Ok(out)
}

fn tile_size(area: (f32, f32), natural: Option<(f32, f32)>, bg_size: BackgroundSize) -> (f32, f32) {
    let (aw, ah) = area;
    match bg_size {
        BackgroundSize::Cover | BackgroundSize::Contain => match natural {
            Some((iw, ih)) => {
                let (sx, sy) = (aw / iw, ah / ih);
                let s = if bg_size == BackgroundSize::Cover {
                    sx.max(sy)
                } else {
                    sx.min(sy)
                };
                (iw * s, ih * s)
            }
            None => (aw, ah),
        },
        BackgroundSize::Explicit(w, h) => {
            let resolve = |v: TileLength, basis: f32| match v {
                TileLength::Pixels(p) => p,
                TileLength::Fraction(f) => f * basis,
            };
            match (w.map(|w| resolve(w, aw)), h.map(|h| resolve(h, ah))) {
                (Some(w), Some(h)) => (w, h),
                (Some(w), None) => (w, natural.map_or(ah, |(iw, ih)| w * ih / iw)),
                (None, Some(h)) => (natural.map_or(aw, |(iw, ih)| h * iw / ih), h),
                (None, None) => natural.unwrap_or((aw, ah)),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sk_css::TileLength::{Fraction as F, Pixels as P};

    const NO: (bool, bool) = (false, false);
    const YES: (bool, bool) = (true, true);

    #[test]
    fn a_percentage_sprite_lands_on_its_cell() {
        let (iw, ih, cell) = (2040.0_f32, 408.0_f32, 68.0_f32);
        let (cx, cy) = (19.0 * cell, 3.0 * cell);
        let size = BackgroundSize::Explicit(Some(F(iw / cell)), Some(F(ih / cell)));
        let position = (F(cx / (iw - cell)), F(cy / (ih - cell)));
        let t = tiles((0.0, 0.0, 22.0, 22.0), Some((iw, ih)), size, position, NO).unwrap();
        assert_eq!(t.len(), 1);
        let (x, y, w, h) = t[0];
        assert!(
            (w - 660.0).abs() < 0.01 && (h - 132.0).abs() < 0.01,
            "{t:?}"
        );
        assert!((x + 19.0 * 22.0).abs() < 0.01, "{x}");
        assert!((y + 3.0 * 22.0).abs() < 0.01, "{y}");
    }

    #[test]
    fn a_pixel_sprite_keeps_the_one_tile_that_shows() {
        let size = BackgroundSize::Explicit(Some(P(660.0)), Some(P(132.0)));
        let t = tiles(
            (10.0, 20.0, 22.0, 22.0),
            Some((2040.0, 408.0)),
            size,
            (P(-418.0), P(-66.0)),
            YES,
        )
        .unwrap();
        assert_eq!(t, vec![(10.0 - 418.0, 20.0 - 66.0, 660.0, 132.0)]);
    }

    #[test]
    fn cover_and_contain_scale_to_the_box() {
        let area = (0.0, 0.0, 300.0, 100.0);
        let cover = tiles(
            area,
            Some((100.0, 100.0)),
            BackgroundSize::Cover,
            (F(0.5), F(0.5)),
            NO,
        )
        .unwrap();
        assert_eq!(cover, vec![(0.0, -100.0, 300.0, 300.0)]);
        let contain = tiles(
            area,
            Some((100.0, 100.0)),
            BackgroundSize::Contain,
            (F(0.5), F(0.5)),
            NO,
        )
        .unwrap();
        assert_eq!(contain, vec![(100.0, 0.0, 100.0, 100.0)]);
    }

    #[test]
    fn auto_takes_the_natural_size_and_one_auto_axis_keeps_the_ratio() {
        let area = (0.0, 0.0, 100.0, 100.0);
        let auto = BackgroundSize::Explicit(None, None);
        assert_eq!(
            tiles(area, Some((40.0, 20.0)), auto, (F(0.0), F(0.0)), NO).unwrap(),
            vec![(0.0, 0.0, 40.0, 20.0)]
        );
        let half = BackgroundSize::Explicit(Some(F(0.5)), None);
        assert_eq!(
            tiles(area, Some((40.0, 20.0)), half, (F(0.0), F(0.0)), NO).unwrap(),
            vec![(0.0, 0.0, 50.0, 25.0)]
        );
    }

    #[test]
    fn a_repeat_covers_the_box_from_the_positioned_tile() {
        let size = BackgroundSize::Explicit(Some(P(40.0)), Some(P(40.0)));
        let t = tiles(
            (0.0, 0.0, 100.0, 40.0),
            None,
            size,
            (P(10.0), P(0.0)),
            (true, false),
        )
        .unwrap();
        let xs: Vec<f32> = t.iter().map(|t| t.0).collect();
        assert_eq!(xs, vec![-30.0, 10.0, 50.0, 90.0]);
        assert!(t.iter().all(|t| t.1 == 0.0));
    }

    #[test]
    fn a_layer_that_needs_too_many_tiles_is_refused() {
        let size = BackgroundSize::Explicit(Some(P(1.0)), Some(P(1.0)));
        assert_eq!(
            tiles(
                (0.0, 0.0, 1000.0, 1000.0),
                None,
                size,
                (F(0.0), F(0.0)),
                YES
            ),
            Err(1_000_000)
        );
    }

    #[test]
    fn only_the_tiles_on_the_box_corners_are_rounded() {
        let area = Bounds {
            origin: point(px(0.), px(0.)),
            size: size(px(100.), px(100.)),
        };
        let r = Corners::all(px(8.));
        let whole = corners_for(
            area,
            Bounds {
                origin: point(px(-10.), px(-10.)),
                size: size(px(200.), px(200.)),
            },
            r,
        );
        assert_eq!(whole, r);
        let top_left = corners_for(
            area,
            Bounds {
                origin: point(px(0.), px(0.)),
                size: size(px(50.), px(50.)),
            },
            r,
        );
        assert_eq!(top_left.top_left, px(8.));
        assert_eq!(top_left.bottom_right, px(0.));
        assert_eq!(top_left.top_right, px(0.));
    }
}
