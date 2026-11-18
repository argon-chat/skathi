//! The generated swatch inside a color input, and its value: HTML's "serialize a color well
//! control color", plus Skathi's `hdr`. The originating element owns focus, keyboard activation
//! and its cancellable DOM click; the runtime opens the picker (`crate::color_picker`) only after
//! that click's handlers have run.
use gpui::prelude::*;
use gpui::{
    div, px, AnyElement, Bounds, Div, Hsla, Length, Pixels, Refineable, StyleRefinement, Window,
};
use sk_css::RgbSpace;

use crate::tree::{Node, Tree};

/// The attributes that shape a color input's value.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Mode {
    pub alpha: bool,
    /// `colorspace="display-p3"`; otherwise HTML's limited sRGB.
    pub p3: bool,
    /// Skathi's `hdr`: the value is linear light, with components past 1.
    pub hdr: bool,
}

impl Mode {
    pub(crate) fn of(t: &Tree, id: u32) -> Mode {
        Mode::new(
            has_alpha(t, id),
            t.prop(id, "colorspace").unwrap_or(""),
            t.prop(id, "hdr").is_some_and(|v| v != "false"),
        )
    }

    pub(crate) fn new(alpha: bool, colorspace: &str, hdr: bool) -> Mode {
        Mode {
            alpha,
            p3: colorspace.eq_ignore_ascii_case("display-p3"),
            hdr,
        }
    }

    /// The space `value` is in.
    pub(crate) fn space(self) -> RgbSpace {
        match (self.p3, self.hdr) {
            (false, false) => RgbSpace::Srgb,
            (false, true) => RgbSpace::SrgbLinear,
            (true, false) => RgbSpace::DisplayP3,
            (true, true) => RgbSpace::DisplayP3Linear,
        }
    }

    /// The gamma-encoded space the picker edits in.
    pub(crate) fn working(self) -> RgbSpace {
        if self.p3 {
            RgbSpace::DisplayP3
        } else {
            RgbSpace::Srgb
        }
    }
}

/// UA defaults only fill properties the author has not specified; `dark` is a dark `color-scheme`.
pub(crate) fn defaults(style: &mut StyleRefinement, dark: bool) {
    style
        .size
        .width
        .get_or_insert(Length::Definite(px(50.0).into()));
    style
        .size
        .height
        .get_or_insert(Length::Definite(px(27.0).into()));
    style.padding.top.get_or_insert(px(2.0).into());
    style.padding.right.get_or_insert(px(2.0).into());
    style.padding.bottom.get_or_insert(px(2.0).into());
    style.padding.left.get_or_insert(px(2.0).into());
    style.border_widths.top.get_or_insert(px(1.0).into());
    style.border_widths.right.get_or_insert(px(1.0).into());
    style.border_widths.bottom.get_or_insert(px(1.0).into());
    style.border_widths.left.get_or_insert(px(1.0).into());
    let (border, face) = if dark {
        (0x858585, 0x6b6b6b)
    } else {
        (0x767676, 0xefefef)
    };
    style.border_color.get_or_insert(gpui::rgb(border).into());
    style.background.get_or_insert(gpui::rgb(face).into());
}

pub(crate) fn is_color_input(t: &Tree, id: u32) -> bool {
    t.node(id).is_some_and(|n| t.name(n.tag) == "input")
        && t.prop(id, "type")
            .is_some_and(|v| v.trim().eq_ignore_ascii_case("color"))
}

/// The `alpha` attribute: the value carries an alpha channel and the picker shows its slider.
pub(crate) fn has_alpha(t: &Tree, id: u32) -> bool {
    t.prop(id, "alpha").is_some_and(|v| v != "false")
}

pub(crate) fn parse(value: &str) -> Option<[f32; 4]> {
    sk_css::parse_color(value)
}

/// HTML's value sanitization: any CSS color converted to the input's space, else black.
pub(crate) fn sanitize(value: &str, mode: Mode) -> String {
    serialize(
        sk_css::parse_color_in(value, mode.space()).unwrap_or([0.0, 0.0, 0.0, 1.0]),
        mode,
    )
}

/// `c` is in `mode.space()`. Limited sRGB rounds channels to eight bits, halves up, and is
/// `#rrggbb` without `alpha`, `color(srgb r g b / a)` with it; Display P3 is
/// `color(display-p3 r g b)`; `hdr` is `srgb-linear` or `display-p3-linear`, unclamped.
pub(crate) fn serialize([r, g, b, a]: [f32; 4], mode: Mode) -> String {
    let a = if mode.alpha { a.clamp(0.0, 1.0) } else { 1.0 };
    if mode.space() == RgbSpace::Srgb {
        let [r, g, b] = [r, g, b].map(byte);
        if !mode.alpha {
            return format!("#{r:02x}{g:02x}{b:02x}");
        }
        return function("srgb", [r, g, b].map(|c| c as f64 / 255.0), a);
    }
    function(space_name(mode.space()), [r, g, b].map(f64::from), a)
}

/// `color(<space> r g b / a)`, the alpha left out when opaque.
pub(crate) fn css_in([r, g, b, a]: [f32; 4], space: RgbSpace) -> String {
    function(space_name(space), [r, g, b].map(f64::from), a)
}

fn space_name(space: RgbSpace) -> &'static str {
    match space {
        RgbSpace::Srgb => "srgb",
        RgbSpace::SrgbLinear => "srgb-linear",
        RgbSpace::DisplayP3 => "display-p3",
        RgbSpace::DisplayP3Linear => "display-p3-linear",
    }
}

fn function(name: &str, rgb: [f64; 3], a: f32) -> String {
    let [r, g, b] = rgb.map(number);
    if a >= 1.0 {
        format!("color({name} {r} {g} {b})")
    } else {
        format!("color({name} {r} {g} {b} / {})", number(a as f64))
    }
}

/// The exposure range of `hdr`, in stops either way.
pub(crate) const MAX_STOPS: i32 = 10;

/// What the picker edits: a base color in the working space, each channel in 0..=1, and an
/// exposure in stops. The stops are `round(log2(brightest))` of an HDR value, one more when that
/// leaves the base past 1, so the value comes back unchanged.
pub(crate) fn split([r, g, b, a]: [f32; 4], mode: Mode) -> ([f32; 4], i32) {
    let a = a.clamp(0.0, 1.0);
    let unit = |c: f32| c.clamp(0.0, 1.0);
    if !mode.hdr {
        return ([unit(r), unit(g), unit(b), a], 0);
    }
    let max = r.max(g).max(b);
    let mut stops = 0;
    if max > 1.0 {
        stops = max.log2().round() as i32;
        if max / 2f32.powi(stops) > 1.0001 {
            stops += 1;
        }
        stops = stops.clamp(-MAX_STOPS, MAX_STOPS);
    }
    let scale = 2f32.powi(-stops);
    let [r, g, b] = sk_css::convert_rgb(
        [r, g, b].map(|c| unit(c * scale)),
        mode.space(),
        mode.working(),
    )
    .map(unit);
    ([r, g, b, a], stops)
}

/// The inverse of [`split`]: the color in `mode.space()`.
pub(crate) fn join([r, g, b, a]: [f32; 4], stops: i32, mode: Mode) -> [f32; 4] {
    if !mode.hdr {
        return [r, g, b, a];
    }
    let scale = 2f32.powi(stops.clamp(-MAX_STOPS, MAX_STOPS));
    let [r, g, b] = sk_css::convert_rgb([r, g, b], mode.working(), mode.space()).map(|c| c * scale);
    [r, g, b, a]
}

/// Any CSS color as the picker's base and stops.
pub(crate) fn read(text: &str, mode: Mode) -> Option<([f32; 4], i32)> {
    sk_css::parse_color_in(text, mode.space()).map(|c| split(c, mode))
}

/// What the window paints for `rgb` in `space`: sRGB, clipped.
pub(crate) fn display(rgb: [f32; 3], space: RgbSpace) -> [f32; 3] {
    sk_css::convert_rgb(rgb, space, RgbSpace::Srgb).map(|c| c.clamp(0.0, 1.0))
}

pub(crate) fn byte(c: f32) -> u8 {
    (c.clamp(0.0, 1.0) * 255.0 + 0.5).floor() as u8
}

/// CSS's number serialization at six significant digits.
pub(crate) fn number(v: f64) -> String {
    if v == 0.0 || !v.is_finite() {
        return "0".into();
    }
    let decimals = (5 - v.abs().log10().floor() as i32).max(0) as usize;
    let s = format!("{v:.decimals$}");
    if s.contains('.') {
        s.trim_end_matches('0').trim_end_matches('.').to_string()
    } else {
        s
    }
}

pub(crate) fn hsla([r, g, b, a]: [f32; 4]) -> Hsla {
    gpui::Rgba { r, g, b, a }.into()
}

/// A checkerboard in `bounds`, under a translucent color. Its cells are not clipped to rounded
/// corners: the caller paints [`mask_corners`] over them.
pub(crate) fn paint_checker(bounds: Bounds<Pixels>, cell: f32, dark: bool, window: &mut Window) {
    let (base, alt) = if dark {
        (gpui::rgb(0x4a4a4a), gpui::rgb(0x6a6a6a))
    } else {
        (gpui::rgb(0xffffff), gpui::rgb(0xd6d6d6))
    };
    window.paint_quad(gpui::fill(bounds, base));
    let (x0, y0) = (f32::from(bounds.origin.x), f32::from(bounds.origin.y));
    let (w, h) = (f32::from(bounds.size.width), f32::from(bounds.size.height));
    let (cols, rows) = ((w / cell).ceil() as usize, (h / cell).ceil() as usize);
    for row in 0..rows {
        for col in (row % 2..cols).step_by(2) {
            let x = x0 + col as f32 * cell;
            let y = y0 + row as f32 * cell;
            let cw = cell.min(x0 + w - x);
            let ch = cell.min(y0 + h - y);
            window.paint_quad(gpui::fill(
                Bounds::new(gpui::point(px(x), px(y)), gpui::size(px(cw), px(ch))),
                alt,
            ));
        }
    }
}

/// Covers what lies outside a `radius` rounded rectangle on `bounds` with `surface`: the rounding
/// for content gpui cannot clip, on a solid background.
pub(crate) fn mask_corners(
    bounds: Bounds<Pixels>,
    radius: f32,
    surface: Hsla,
    window: &mut Window,
) {
    if radius <= 0.0 {
        return;
    }
    let band = radius;
    let outer = bounds.dilate(px(band));
    window.paint_quad(gpui::quad(
        outer,
        px(radius + band),
        gpui::transparent_black(),
        px(band),
        surface,
        gpui::BorderStyle::Solid,
    ));
}

pub(crate) fn children(t: &Tree, id: u32, node: &Node, window: &Window) -> AnyElement {
    let (active, focused) = crate::windows::here(window, |state| {
        (state.active == Some(id), state.focused_node == id)
    });
    let hovered = crate::tree::is_hovered(id);
    let visible = focused && window.last_input_was_keyboard();
    let rgba = parse(t.prop(id, "value").unwrap_or("")).unwrap_or([0.0, 0.0, 0.0, 1.0]);
    let rgba = if has_alpha(t, id) {
        rgba
    } else {
        [rgba[0], rgba[1], rgba[2], 1.0]
    };
    let mut wrapper = div().size_full().p(px(2.0));
    let mut swatch = div()
        .size_full()
        .border_1()
        .border_color(gpui::rgb(0x777777))
        .bg(hsla(rgba));
    if rgba[3] < 1.0 {
        let dark = crate::scheme::declared(t, id, window) == Some(true);
        swatch = swatch.child(
            gpui::canvas(
                |_, _, _| (),
                move |bounds, (), window, _| {
                    paint_checker(bounds, 4.0, dark, window);
                    window.paint_quad(gpui::fill(bounds, hsla(rgba)));
                },
            )
            .size_full(),
        );
    }
    if let Some(computed) = &node.computed {
        if let Some(part) = &computed.color_swatch_wrapper {
            wrapper = apply(wrapper, part, active, hovered, visible, focused);
        }
        if let Some(part) = &computed.color_swatch {
            swatch = apply(swatch, part, active, hovered, visible, focused);
        }
    }
    wrapper.child(swatch).into_any_element()
}

fn apply(
    mut element: Div,
    part: &sk_css::ControlPseudo,
    active: bool,
    hovered: bool,
    visible: bool,
    focused: bool,
) -> Div {
    let (style, extras) = part.resolve(active, hovered, visible, focused, focused);
    element.style().refine(style);
    if extras.visibility_hidden == Some(true) {
        element = element.opacity(0.0);
    }
    element
}

#[cfg(test)]
mod tests {
    use super::*;

    const LIMITED: Mode = Mode {
        alpha: false,
        p3: false,
        hdr: false,
    };
    const ALPHA: Mode = Mode {
        alpha: true,
        ..LIMITED
    };
    const P3: Mode = Mode {
        p3: true,
        ..LIMITED
    };
    const HDR: Mode = Mode {
        hdr: true,
        ..LIMITED
    };

    #[test]
    fn any_css_color_sanitizes_to_lowercase_hex_and_anything_else_to_black() {
        assert_eq!(sanitize("#Ab12EF", LIMITED), "#ab12ef");
        assert_eq!(sanitize("#f05", LIMITED), "#ff0055");
        assert_eq!(sanitize("#1234", LIMITED), "#112233");
        assert_eq!(sanitize("#12345678", LIMITED), "#123456");
        assert_eq!(sanitize("rgb(255 128 0 / 50%)", LIMITED), "#ff8000");
        assert_eq!(sanitize("rebeccapurple", LIMITED), "#663399");
        assert_eq!(sanitize(" hsl(120deg 100% 25%) ", LIMITED), "#008000");
        assert_eq!(sanitize("color(display-p3 1 0 0)", LIMITED), "#ff0000");
        for value in [
            "",
            "invalid",
            "#x00000",
            "#12345",
            "currentcolor",
            "canvastext",
        ] {
            assert_eq!(sanitize(value, LIMITED), "#000000", "{value:?}");
        }
    }

    #[test]
    fn with_alpha_the_value_is_srgb_color_function_with_eight_bit_channels() {
        assert_eq!(sanitize("#ff8000", ALPHA), "color(srgb 1 0.501961 0)");
        assert_eq!(sanitize("#ff000080", ALPHA), "color(srgb 1 0 0 / 0.501961)");
        assert_eq!(
            sanitize("rgb(10 20 30)", ALPHA),
            "color(srgb 0.0392157 0.0784314 0.117647)"
        );
        assert_eq!(
            sanitize("color(srgb 0.5 0.5 0.5 / 0.25)", ALPHA),
            "color(srgb 0.501961 0.501961 0.501961 / 0.25)"
        );
        assert_eq!(sanitize("transparent", ALPHA), "color(srgb 0 0 0 / 0)");
        assert_eq!(sanitize("nope", ALPHA), "color(srgb 0 0 0)");
        let once = sanitize("color(srgb 0.3 0.6 0.9 / 0.5)", ALPHA);
        assert_eq!(sanitize(&once, ALPHA), once);
    }

    #[test]
    fn display_p3_converts_without_eight_bit_rounding_and_keeps_out_of_gamut() {
        assert_eq!(
            sanitize("color(display-p3 1 0 0)", P3),
            "color(display-p3 1 0 0)"
        );
        assert_eq!(
            sanitize("#ff0000", P3),
            "color(display-p3 0.917488 0.200287 0.138561)"
        );
        assert_eq!(
            sanitize("color(display-p3 0.123456 0.5 1 / 0.5)", P3),
            "color(display-p3 0.123456 0.5 1)",
            "no alpha without the attribute"
        );
        assert_eq!(
            sanitize(
                "color(display-p3 0.123456 0.5 1 / 0.5)",
                Mode { alpha: true, ..P3 }
            ),
            "color(display-p3 0.123456 0.5 1 / 0.5)"
        );
        let rec2020 = sanitize("color(rec2020 0 1 0)", P3);
        assert!(rec2020.contains(" -"), "{rec2020}");
        assert_eq!(sanitize("bogus", P3), "color(display-p3 0 0 0)");
        assert!(Mode::new(false, "DISPLAY-P3", false).p3);
        assert!(!Mode::new(false, " display-p3", false).p3);
    }

    #[test]
    fn hdr_values_are_linear_and_keep_components_past_one() {
        assert_eq!(sanitize("#ffffff", HDR), "color(srgb-linear 1 1 1)");
        assert_eq!(
            sanitize("color(srgb-linear 8 4 0.5)", HDR),
            "color(srgb-linear 8 4 0.5)"
        );
        assert_eq!(
            sanitize(
                "color(srgb-linear 2 0 0 / 0.5)",
                Mode { alpha: true, ..HDR }
            ),
            "color(srgb-linear 2 0 0 / 0.5)"
        );
        let both = Mode { p3: true, ..HDR };
        assert_eq!(
            sanitize("color(display-p3-linear 4 2 1)", both),
            "color(display-p3-linear 4 2 1)"
        );
        let once = sanitize("color(srgb-linear 3 0.2 0.1)", both);
        assert!(once.starts_with("color(display-p3-linear "), "{once}");
        assert_eq!(sanitize(&once, both), once);
    }

    #[test]
    fn hdr_values_split_into_base_and_stops_and_join_back() {
        let ([r, g, b, _], stops) = split([8.0, 4.0, 0.0, 1.0], HDR);
        assert_eq!(stops, 3);
        let want = [1.0, sk_css::from_linear(0.5), 0.0];
        assert!(
            [r, g, b]
                .iter()
                .zip(want)
                .all(|(x, y)| (x - y).abs() < 1e-5),
            "{r} {g} {b}"
        );
        assert_eq!(
            split([0.25, 0.25, 0.25, 1.0], HDR).1,
            0,
            "dim values keep 0 stops"
        );
        assert_eq!(
            split([1.3, 0.0, 0.0, 1.0], HDR).1,
            1,
            "round(log2(1.3)) would leave the base past 1"
        );
        assert_eq!(split([1e6, 0.0, 0.0, 1.0], HDR).1, MAX_STOPS);
        for mode in [HDR, Mode { p3: true, ..HDR }] {
            for (base, stops) in [([0.8, 0.4, 0.1, 1.0], 4), ([1.0, 0.0, 0.5, 0.5], -3)] {
                let value = join(base, stops, mode);
                let text = serialize(
                    value,
                    Mode {
                        alpha: true,
                        ..mode
                    },
                );
                let (b, _) = read(
                    &text,
                    Mode {
                        alpha: true,
                        ..mode
                    },
                )
                .unwrap();
                let back = join(b, read(&text, mode).unwrap().1, mode);
                for (x, y) in value.iter().zip(back) {
                    assert!(
                        (x - y).abs() < 1e-3 * x.abs().max(1.0),
                        "{text}: {value:?} {back:?}"
                    );
                }
            }
        }
        assert_eq!(split([0.5, 1.5, 0.0, 1.0], P3), ([0.5, 1.0, 0.0, 1.0], 0));
    }
}
