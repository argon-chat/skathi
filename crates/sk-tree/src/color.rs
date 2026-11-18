//! The generated swatch inside a color input, and its value: HTML's "serialize a color well
//! control color" in sRGB. The originating element owns focus, keyboard activation and its
//! cancellable DOM click; the runtime opens the picker (`crate::color_picker`) only after that
//! click's handlers have run.
use gpui::prelude::*;
use gpui::{
    div, px, AnyElement, Bounds, Div, Hsla, Length, Pixels, Refineable, StyleRefinement, Window,
};

use crate::tree::{Node, Tree};

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

/// HTML's value sanitization: any CSS color, else black.
pub(crate) fn sanitize(value: &str, alpha: bool) -> String {
    serialize(parse(value).unwrap_or([0.0, 0.0, 0.0, 1.0]), alpha)
}

/// Channels round to eight bits, halves up; without `alpha` the value is `#rrggbb`, with it
/// `color(srgb r g b / a)`.
pub(crate) fn serialize([r, g, b, a]: [f32; 4], alpha: bool) -> String {
    let [r, g, b] = [r, g, b].map(byte);
    if !alpha {
        return format!("#{r:02x}{g:02x}{b:02x}");
    }
    let [r, g, b] = [r, g, b].map(|c| number(c as f64 / 255.0));
    let a = a.clamp(0.0, 1.0);
    if a >= 1.0 {
        format!("color(srgb {r} {g} {b})")
    } else {
        format!("color(srgb {r} {g} {b} / {})", number(a as f64))
    }
}

pub(crate) fn byte(c: f32) -> u8 {
    (c.clamp(0.0, 1.0) * 255.0 + 0.5).floor() as u8
}

/// CSS's number serialization at six significant digits.
fn number(v: f64) -> String {
    if v == 0.0 {
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

    #[test]
    fn any_css_color_sanitizes_to_lowercase_hex_and_anything_else_to_black() {
        assert_eq!(sanitize("#Ab12EF", false), "#ab12ef");
        assert_eq!(sanitize("#f05", false), "#ff0055");
        assert_eq!(sanitize("#1234", false), "#112233");
        assert_eq!(sanitize("#12345678", false), "#123456");
        assert_eq!(sanitize("rgb(255 128 0 / 50%)", false), "#ff8000");
        assert_eq!(sanitize("rebeccapurple", false), "#663399");
        assert_eq!(sanitize(" hsl(120deg 100% 25%) ", false), "#008000");
        for value in [
            "",
            "invalid",
            "#x00000",
            "#12345",
            "currentcolor",
            "canvastext",
        ] {
            assert_eq!(sanitize(value, false), "#000000", "{value:?}");
        }
    }

    #[test]
    fn with_alpha_the_value_is_srgb_color_function_with_eight_bit_channels() {
        assert_eq!(sanitize("#ff8000", true), "color(srgb 1 0.501961 0)");
        assert_eq!(sanitize("#ff000080", true), "color(srgb 1 0 0 / 0.501961)");
        assert_eq!(
            sanitize("rgb(10 20 30)", true),
            "color(srgb 0.0392157 0.0784314 0.117647)"
        );
        assert_eq!(
            sanitize("color(srgb 0.5 0.5 0.5 / 0.25)", true),
            "color(srgb 0.501961 0.501961 0.501961 / 0.25)"
        );
        assert_eq!(sanitize("transparent", true), "color(srgb 0 0 0 / 0)");
        assert_eq!(sanitize("nope", true), "color(srgb 0 0 0)");
        let once = sanitize("color(srgb 0.3 0.6 0.9 / 0.5)", true);
        assert_eq!(
            sanitize(&once, true),
            once,
            "a serialized value is a fixed point"
        );
    }
}
