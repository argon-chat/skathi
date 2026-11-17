//! The generated swatch inside a color input. The originating element owns focus, keyboard
//! activation and its cancellable DOM click; the runtime opens the picker only after that click's
//! handlers have run.
use gpui::prelude::*;
use gpui::{div, px, AnyElement, Div, Length, Refineable, StyleRefinement, Window};

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

fn value_rgb(value: Option<&str>) -> u32 {
    let value = value.unwrap_or("").trim();
    let Some(hex) = value.strip_prefix('#') else {
        return 0;
    };
    match hex.len() {
        6 | 8 if hex.bytes().all(|byte| byte.is_ascii_hexdigit()) => {
            u32::from_str_radix(&hex[..6], 16).unwrap_or(0)
        }
        3 | 4 if hex.bytes().all(|byte| byte.is_ascii_hexdigit()) => {
            let value = u32::from_str_radix(&hex[..3], 16).unwrap_or(0);
            ((value & 0xf00) << 12)
                | ((value & 0xf00) << 8)
                | ((value & 0xf0) << 8)
                | ((value & 0xf0) << 4)
                | ((value & 0xf) << 4)
                | (value & 0xf)
        }
        _ => 0,
    }
}

pub(crate) fn children(t: &Tree, id: u32, node: &Node, window: &Window) -> AnyElement {
    let (active, focused) = crate::windows::here(window, |state| {
        (state.active == Some(id), state.focused_node == id)
    });
    let hovered = crate::tree::is_hovered(id);
    let visible = focused && window.last_input_was_keyboard();
    let mut wrapper = div().size_full().p(px(2.0));
    let mut swatch = div()
        .size_full()
        .border_1()
        .border_color(gpui::rgb(0x777777))
        .bg(gpui::rgb(value_rgb(t.prop(id, "value"))));
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
    fn swatch_matches_opaque_hex_value_and_invalid_is_black() {
        assert_eq!(value_rgb(Some("#Ab12EF")), 0xab12ef);
        assert_eq!(value_rgb(Some("#f05")), 0xff0055);
        assert_eq!(value_rgb(Some("#1234")), 0x112233);
        assert_eq!(value_rgb(Some("#12345678")), 0x123456);
        for value in [None, Some("invalid"), Some("#x00000"), Some("#12345")] {
            assert_eq!(value_rgb(value), 0);
        }
    }
}
