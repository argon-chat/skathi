//! What the macOS status item is told, decided in plain Rust so it is unit-tested on every OS.
//!
//! The menu is a tree of plain nodes plus one action table in windows.rs's command-id order; each item
//! carries its slot and revision in its `tag` ([`tag`] / [`untag`]), so a selection from a replaced menu is
//! fenced as on Windows. A primary click is `on_click` when present, otherwise the menu; a secondary click
//! is always the menu. The icon is fitted in the 18-point square at 1x and 2x, drawn as a template when the
//! model asks (`Icon::template`) and for a one-tone grey glyph that would otherwise vanish.

use std::io::Cursor;

use super::{Callback, Icon, MenuItem};

/// The side of the square a status-bar icon is fitted in, in points (the menu bar is 22, 24 beside a notch).
pub const SIDE_PT: u32 = 18;
pub const SCALES: [u32; 2] = [1, 2];
pub const QUIT: &str = "Quit";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Node {
    Separator,
    Item {
        label: String,
        enabled: bool,
        checked: bool,
        action: usize,
    },
    Submenu {
        label: String,
        enabled: bool,
        items: Vec<Node>,
    },
}

pub fn menu(items: &[MenuItem], quit: Option<&Callback>) -> (Vec<Node>, Vec<Option<Callback>>) {
    let mut actions = Vec::new();
    let nodes = nodes(items, quit, &mut actions);
    (nodes, actions)
}

fn nodes(
    items: &[MenuItem],
    quit: Option<&Callback>,
    actions: &mut Vec<Option<Callback>>,
) -> Vec<Node> {
    let mut out = Vec::with_capacity(items.len());
    for item in items {
        let (label, disabled, checked, callback) = match item {
            MenuItem::Separator => {
                out.push(Node::Separator);
                continue;
            }
            MenuItem::Submenu {
                label,
                disabled,
                items,
            } => {
                let items = nodes(items, quit, actions);
                out.push(Node::Submenu {
                    label: label.clone(),
                    enabled: !disabled,
                    items,
                });
                continue;
            }
            MenuItem::Action {
                label,
                disabled,
                checked,
                on_select,
                ..
            } => (label.as_str(), *disabled, *checked, on_select.clone()),
            MenuItem::Quit { label, disabled } => (
                label.as_deref().unwrap_or(QUIT),
                *disabled,
                None,
                quit.cloned(),
            ),
        };
        actions.push(if disabled { None } else { callback });
        out.push(Node::Item {
            label: label.to_owned(),
            enabled: !disabled,
            checked: checked == Some(true),
            action: actions.len() - 1,
        });
    }
    out
}

/// A menu item's `tag`: the low 32 bits of its menu's revision, then its action slot (at most 512 per menu).
pub fn tag(revision: u64, action: usize) -> isize {
    (((revision & 0xffff_ffff) << 32) | (action as u64 & 0xffff_ffff)) as i64 as isize
}

pub fn untag(tag: isize) -> (u32, usize) {
    let tag = tag as i64 as u64;
    ((tag >> 32) as u32, (tag & 0xffff_ffff) as usize)
}

pub fn action_of(tag: isize, revision: u64) -> Option<usize> {
    let (built_for, action) = untag(tag);
    (u64::from(built_for) == revision & 0xffff_ffff).then_some(action)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Press {
    Click,
    Menu,
    Nothing,
}

pub fn press(secondary: bool, has_click: bool, has_menu: bool) -> Press {
    if !secondary && has_click {
        Press::Click
    } else if has_menu {
        Press::Menu
    } else {
        Press::Nothing
    }
}

pub fn secondary(right_button: bool, control: bool) -> bool {
    right_button || control
}

pub fn owns_menu(has_click: bool, has_menu: bool) -> bool {
    !has_click && has_menu
}

pub fn tooltip(tooltip: &str, app_name: impl FnOnce() -> Option<String>) -> String {
    if !tooltip.is_empty() {
        return tooltip.to_owned();
    }
    app_name()
        .filter(|name| !name.is_empty())
        .or_else(|| {
            std::env::current_exe().ok().and_then(|path| {
                path.file_stem()
                    .map(|stem| stem.to_string_lossy().into_owned())
            })
        })
        .unwrap_or_else(|| "Application".into())
}

pub fn template(icon: &Icon) -> bool {
    icon.template || monochrome(icon)
}

pub fn monochrome(icon: &Icon) -> bool {
    const VISIBLE: u8 = 16;
    const GREY: u8 = 24;
    const SHADES: u8 = 48;
    let (mut darkest, mut lightest, mut seen) = (u8::MAX, 0, false);
    for pixel in icon.rgba.chunks_exact(4) {
        if pixel[3] < VISIBLE {
            continue;
        }
        let (r, g, b) = (pixel[0], pixel[1], pixel[2]);
        if r.abs_diff(g) > GREY || g.abs_diff(b) > GREY || r.abs_diff(b) > GREY {
            return false;
        }
        let shade = ((u16::from(r) + u16::from(g) + u16::from(b)) / 3) as u8;
        darkest = darkest.min(shade);
        lightest = lightest.max(shade);
        seen = true;
    }
    seen && lightest - darkest <= SHADES
}

pub fn raster(icon: &Icon, side: u32) -> Vec<u8> {
    let side = side.max(1);
    let mut pixels = icon.rgba.to_vec();
    for pixel in pixels.chunks_exact_mut(4) {
        let alpha = u16::from(pixel[3]);
        for channel in &mut pixel[..3] {
            *channel = ((u16::from(*channel) * alpha + 127) / 255) as u8;
        }
    }
    let Some(input) = image::RgbaImage::from_raw(icon.width, icon.height, pixels) else {
        return vec![0; side as usize * side as usize * 4];
    };
    let (width, height) = fit(icon.width, icon.height, side);
    let resized =
        image::imageops::resize(&input, width, height, image::imageops::FilterType::Triangle);
    let mut canvas = image::RgbaImage::new(side, side);
    image::imageops::overlay(
        &mut canvas,
        &resized,
        i64::from((side - width) / 2),
        i64::from((side - height) / 2),
    );
    let mut pixels = canvas.into_raw();
    for pixel in pixels.chunks_exact_mut(4) {
        let alpha = u16::from(pixel[3]);
        for channel in &mut pixel[..3] {
            if let Some(straight) = (u16::from(*channel) * 255 + alpha / 2).checked_div(alpha) {
                *channel = straight.min(255) as u8;
            }
        }
    }
    pixels
}

pub fn fit(width: u32, height: u32, side: u32) -> (u32, u32) {
    let scale =
        (f64::from(side) / f64::from(width.max(1))).min(f64::from(side) / f64::from(height.max(1)));
    let width = ((f64::from(width) * scale).round() as u32).clamp(1, side);
    let height = ((f64::from(height) * scale).round() as u32).clamp(1, side);
    (width, height)
}

pub fn png(icon: &Icon, scale: u32) -> Result<Vec<u8>, String> {
    use image::ImageEncoder;
    let side = SIDE_PT * scale;
    let pixels = raster(icon, side);
    let mut out = Cursor::new(Vec::new());
    image::codecs::png::PngEncoder::new(&mut out)
        .write_image(&pixels, side, side, image::ExtendedColorType::Rgba8)
        .map_err(|error| format!("encoding the tray icon: {error}"))?;
    Ok(out.into_inner())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    };

    fn icon(width: u32, height: u32, pixel: [u8; 4]) -> Icon {
        Icon::rgba(width, height, pixel.repeat((width * height) as usize)).unwrap()
    }

    #[test]
    fn the_menu_numbers_actions_as_windows_does_and_keeps_disabled_slots() {
        let hits = Arc::new(AtomicUsize::new(0));
        let counter = hits.clone();
        let quit: Callback = Arc::new(move || {
            counter.fetch_add(10, Ordering::Relaxed);
        });
        let open = hits.clone();
        let items = vec![
            MenuItem::action("Open", move || {
                open.fetch_add(1, Ordering::Relaxed);
            }),
            MenuItem::Separator,
            MenuItem::submenu(
                "Status",
                vec![
                    MenuItem::Action {
                        label: "Muted".into(),
                        disabled: true,
                        checked: Some(true),
                        default: false,
                        on_select: Some(Arc::new(|| panic!("disabled"))),
                    },
                    MenuItem::Action {
                        label: "A & B".into(),
                        disabled: false,
                        checked: Some(false),
                        default: true,
                        on_select: None,
                    },
                ],
            ),
            MenuItem::Quit {
                label: None,
                disabled: false,
            },
        ];
        let (nodes, actions) = menu(&items, Some(&quit));
        assert_eq!(actions.len(), 4);
        assert_eq!(
            nodes,
            vec![
                Node::Item {
                    label: "Open".into(),
                    enabled: true,
                    checked: false,
                    action: 0
                },
                Node::Separator,
                Node::Submenu {
                    label: "Status".into(),
                    enabled: true,
                    items: vec![
                        Node::Item {
                            label: "Muted".into(),
                            enabled: false,
                            checked: true,
                            action: 1
                        },
                        Node::Item {
                            label: "A & B".into(),
                            enabled: true,
                            checked: false,
                            action: 2
                        },
                    ],
                },
                Node::Item {
                    label: QUIT.into(),
                    enabled: true,
                    checked: false,
                    action: 3
                },
            ]
        );
        assert!(actions[1].is_none(), "a disabled item runs nothing");
        assert!(actions[2].is_none());
        actions[0].as_ref().unwrap()();
        actions[3].as_ref().unwrap()();
        assert_eq!(hits.load(Ordering::Relaxed), 11);
        let (_, without_quit) = menu(
            &[MenuItem::Quit {
                label: Some("Exit".into()),
                disabled: true,
            }],
            Some(&quit),
        );
        assert!(without_quit[0].is_none());
    }

    #[test]
    fn tags_carry_the_revision_and_fence_a_replaced_menu() {
        for (revision, action) in [
            (0, 0),
            (7, 511),
            (u64::from(u32::MAX), 3),
            (u64::MAX, 12),
            (1 << 40 | 5, 9),
        ] {
            let tag = tag(revision, action);
            assert_eq!(untag(tag), (revision as u32, action));
            assert_eq!(action_of(tag, revision), Some(action));
            assert_eq!(action_of(tag, revision.wrapping_add(1)), None);
        }
    }

    #[test]
    fn clicks_follow_the_windows_activation_rule() {
        assert_eq!(press(false, true, true), Press::Click);
        assert_eq!(press(false, true, false), Press::Click);
        assert_eq!(press(true, true, true), Press::Menu);
        assert_eq!(press(true, true, false), Press::Nothing);
        assert_eq!(press(false, false, true), Press::Menu);
        assert_eq!(press(false, false, false), Press::Nothing);
        assert!(secondary(true, false) && secondary(false, true) && !secondary(false, false));
        assert!(owns_menu(false, true));
        assert!(!owns_menu(true, true) && !owns_menu(false, false));
    }

    #[test]
    fn the_tooltip_falls_back_to_the_application_name() {
        assert_eq!(tooltip("Argon", || Some("Other".into())), "Argon");
        assert_eq!(tooltip("", || Some("Argon".into())), "Argon");
        assert!(!tooltip("", || Some(String::new())).is_empty());
        assert!(!tooltip("", || None).is_empty());
    }

    #[test]
    fn one_grey_tone_is_a_template_and_a_logo_keeps_its_colours() {
        let mut glyph = icon(4, 4, [0, 0, 0, 255]);
        assert!(template(&glyph));
        let mut pixels = glyph.rgba.to_vec();
        pixels[3] = 40;
        pixels[4..8].copy_from_slice(&[200, 30, 30, 0]);
        glyph = Icon::rgba(4, 4, pixels).unwrap();
        assert!(monochrome(&glyph));
        assert!(monochrome(&icon(2, 2, [250, 250, 248, 255])));
        assert!(!monochrome(&icon(2, 2, [255, 0, 0, 255])));
        let mut shaded = icon(2, 1, [20, 20, 20, 255]).rgba.to_vec();
        shaded[4..8].copy_from_slice(&[220, 220, 220, 255]);
        assert!(!monochrome(&Icon::rgba(2, 1, shaded).unwrap()));
        assert!(
            !monochrome(&icon(2, 2, [0, 0, 0, 0])),
            "nothing visible is not a glyph"
        );
        let mut asked = icon(2, 2, [255, 0, 0, 255]);
        assert!(!template(&asked));
        asked.template = true;
        assert!(template(&asked));
    }

    #[test]
    fn icons_fit_the_glyph_square_at_both_scales() {
        assert_eq!(fit(512, 512, 18), (18, 18));
        assert_eq!(fit(64, 32, 36), (36, 18));
        assert_eq!(fit(1, 400, 18), (1, 18));
        let wide = icon(4, 2, [255, 0, 0, 128]);
        let pixels = raster(&wide, 36);
        assert_eq!(pixels.len(), 36 * 36 * 4);
        assert_eq!(pixels[3], 0);
        let middle = &pixels[(18 * 36 + 18) * 4..][..4];
        assert_eq!(middle[0], 255);
        assert!((126..=130).contains(&middle[3]), "{middle:?}");
        for scale in SCALES {
            let png = png(&wide, scale).unwrap();
            let decoded = image::load_from_memory(&png).unwrap();
            assert_eq!(
                (decoded.width(), decoded.height()),
                (SIDE_PT * scale, SIDE_PT * scale)
            );
        }
    }
}
