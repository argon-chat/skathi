//! The DOM surface the JS mirror nodes answer from.

use crate::live_state::{LiveRef, LiveStates};
use crate::tree::{self, Kind, Tree};

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum QueryMode {
    Descendants,
    Document,
    SelfOnly,
    Closest,
}

pub fn query(selector: &str, root: u32, mode: QueryMode) -> Result<Vec<u32>, String> {
    tree::read_tree(|t| {
        if t.node(root).is_none() {
            return Ok(Vec::new());
        }
        let ids: Vec<u32> = match mode {
            QueryMode::Descendants => descendants(t, root),
            QueryMode::Document => std::iter::once(root).chain(descendants(t, root)).collect(),
            QueryMode::SelfOnly => vec![root],
            QueryMode::Closest => ancestors_from(t, root),
        };
        // Live state answers `:hover` / `:active` / `:focus*` as a browser would; the cascade's matcher
        // says "would match" (`crate::live_state`).
        let live = LiveStates::take(t);
        let candidates: Vec<LiveRef<'_>> =
            ids.iter().map(|&id| LiveRef::new(t, id, &live)).collect();
        let result =
            sk_css::query_all_in(selector, &candidates, Some(LiveRef::new(t, root, &live)))?;
        for w in &result.warnings {
            crate::log::record("css", format!("[query] {w}"));
        }
        let mut matched: Vec<u32> = result.matched.iter().map(|&i| ids[i]).collect();
        if mode == QueryMode::Closest {
            matched.truncate(1);
        }
        Ok(matched)
    })
}

fn descendants(t: &Tree, id: u32) -> Vec<u32> {
    let mut out = Vec::new();
    fn walk(t: &Tree, id: u32, out: &mut Vec<u32>) {
        let Some(node) = t.node(id) else { return };
        for &c in &node.children {
            if t.node(c).is_some_and(|n| n.kind == Kind::Element) {
                out.push(c);
            }
            walk(t, c, out);
        }
    }
    walk(t, id, &mut out);
    out
}

fn ancestors_from(t: &Tree, id: u32) -> Vec<u32> {
    let mut out = Vec::new();
    let mut cursor = id;
    // A corrupt parent chain must not hang the call.
    for _ in 0..4096 {
        match t.node(cursor) {
            Some(n) => {
                if n.kind == Kind::Element {
                    out.push(cursor);
                }
                if n.parent == 0 {
                    break;
                }
                cursor = n.parent;
            }
            None => break,
        }
    }
    out
}

/// A browser lays out on read; so does this: only for a node with no rectangle while the tree holds
/// changes no frame has taken (`onMounted` + `nextTick` measuring). Returns `true` when the answer
/// stays STALE (the host cannot draw inside a gpui input listener).
fn force_layout_if_unpainted(id: u32) -> bool {
    if id == 0 {
        return false;
    }
    if crate::render::bounds_of(id).is_some() {
        return false;
    }
    if !tree::read_tree(|t| t.node(id).is_some()) || !crate::layout_owed() {
        return false;
    }
    let before = crate::render::build_count();
    crate::sync_layout_by(crate::perf::SyncCaller::Bounds);
    crate::render::build_count() == before && crate::render::bounds_of(id).is_none()
}

/// The same forced layout for reads that depend on the node's content (`scrollWidth`/`scrollHeight`
/// and the writes clamped against them), also for a node that has a rectangle
/// (`list.append(row); list.scrollTop = list.scrollHeight`). Returns `true` when stale.
#[must_use]
pub(crate) fn layout_if_stale(id: u32) -> bool {
    if id == 0 || !tree::read_tree(|t| t.node(id).is_some()) || !crate::layout_owed() {
        return false;
    }
    let before = crate::render::build_count();
    crate::sync_layout_by(crate::perf::SyncCaller::Scroll);
    crate::render::build_count() == before
}

/// `getBoundingClientRect()` in CSS px relative to the APP ROOT (what `position: fixed` resolves
/// against); window-client coordinates would make floating-ui measure and clip in different spaces.
/// `None` = not painted; the `bool` says a missing answer is STALE.
pub fn bounds(id: u32) -> (Option<(f32, f32, f32, f32)>, bool) {
    let stale = force_layout_if_unpainted(id);
    (bounds_now(id), stale)
}

/// [`bounds`] for a script's geometry read: lays out first when changes are pending, as a
/// browser's forced reflow does.
pub(crate) fn current_bounds(id: u32) -> (Option<(f32, f32, f32, f32)>, bool) {
    if id == 0 || crate::render::bounds_of(id).is_none() {
        return bounds(id);
    }
    if crate::layout_owed() && tree::read_tree(|t| t.node(id).is_some()) {
        crate::sync_layout_by(crate::perf::SyncCaller::Bounds);
    }
    (bounds_now(id), false)
}

pub fn transformed(id: u32) -> Option<(f32, f32, f32, f32)> {
    let (x, y, w, h) = crate::render::transformed_of(id)?;
    let (ox, oy) = root_origin();
    let (sx, sy) = crate::scroll::unpainted_shift(id);
    Some((x - ox + sx, y - oy + sy, w, h))
}

fn root_origin() -> (f32, f32) {
    crate::render::bounds_of(tree::read_tree(|t| t.root))
        .map(|(rx, ry, _, _)| (rx, ry))
        .unwrap_or((0.0, 0.0))
}

/// How many element descendants `id` has and how many of them the last frame painted inside its
/// nearest scroll container (`id` itself counts). `None` outside a scroll container or before a paint.
/// Never lays out: a dev advisory must not change the frame it measures.
pub fn visible_share(id: u32) -> Option<(usize, usize)> {
    let (chain, ids) = tree::read_tree(|t| (ancestors_from(t, id), descendants(t, id)));
    let scroller = chain
        .into_iter()
        .find(|&a| crate::render::scroll_handle_of(a).is_some())?;
    let window = crate::windows::window_of_node(scroller);
    crate::windows::peek(window, |w| {
        let &(vx, vy, vw, vh) = w.bounds.get(&scroller)?;
        let seen = ids
            .iter()
            .filter(|&id| {
                w.bounds.get(id).is_some_and(|&(x, y, width, height)| {
                    x < vx + vw && x + width > vx && y < vy + vh && y + height > vy
                })
            })
            .count();
        Some((ids.len(), seen))
    })
    .flatten()
}

fn bounds_now(id: u32) -> Option<(f32, f32, f32, f32)> {
    let (x, y, w, h) = crate::render::bounds_of(id)?;
    let (ox, oy) = root_origin();
    let (sx, sy) = crate::scroll::unpainted_shift(id);
    Some((x - ox + sx, y - oy + sy, w, h))
}

/// The properties `getComputedStyle` answers, besides every custom property. Each ALWAYS has a value: a
/// browser's computed style has no "absent", and a component that reads `display` expects a keyword, not "".
pub const COMPUTED_PROPERTIES: &[&str] = &[
    "display",
    "position",
    "visibility",
    "overflow-x",
    "overflow-y",
    "overflow",
    "scrollbar-gutter",
    "opacity",
    "z-index",
    "pointer-events",
    "transform",
    "transform-origin",
    "zoom",
    "translate",
    "rotate",
    "scale",
    "perspective",
    "filter",
    "will-change",
    "contain",
    "container-type",
    "backdrop-filter",
    "animation-name",
    "animation-duration",
    "animation-delay",
    "animation-fill-mode",
    "transition-property",
    "transition-duration",
    "transition-delay",
    "width",
    "height",
    "padding-top",
    "padding-right",
    "padding-bottom",
    "padding-left",
    "margin-top",
    "margin-right",
    "margin-bottom",
    "margin-left",
    "border-top-width",
    "border-right-width",
    "border-bottom-width",
    "border-left-width",
    "direction",
    "font-size",
    "color",
    "text-shadow",
];

/// One computed property of `id` (a [`COMPUTED_PROPERTIES`] name or a custom property); `None` for a node
/// not in the tree, or a name outside both.
pub fn computed_value(id: u32, name: &str) -> Option<String> {
    tree::flush_pending_style();
    tree::read_tree(|t| computed_in(t, id, name))
}

/// `(name, value)` for every [`COMPUTED_PROPERTIES`] entry, then each custom property.
pub fn computed_entries(id: u32) -> Option<Vec<(String, String)>> {
    tree::flush_pending_style();
    tree::read_tree(|t| {
        let n = t.node(id)?;
        let mut out: Vec<(String, String)> = COMPUTED_PROPERTIES
            .iter()
            .filter_map(|&k| computed_in(t, id, k).map(|v| (k.to_string(), v)))
            .collect();
        if let Some(cs) = n.computed.as_ref() {
            out.extend(cs.vars.iter().map(|(k, v)| (k.to_string(), v.to_string())));
        }
        Some(out)
    })
}

#[cfg(test)]
/// [`computed_entries`] as `name=value` lines.
pub fn computed_style(id: u32) -> Option<String> {
    let mut out = String::new();
    for (k, v) in computed_entries(id)? {
        out.push_str(&k);
        out.push('=');
        out.push_str(&v);
        out.push('\n');
    }
    Some(out)
}

fn computed_in(t: &Tree, id: u32, name: &str) -> Option<String> {
    let default = sk_css::ComputedStates::default();
    let cs = t.node(id)?.computed.as_ref().unwrap_or(&default);
    if name.starts_with("--") {
        return cs.vars.get(name).map(str::to_string);
    }
    let animations = |f: &dyn Fn(&sk_css::AnimationSpec) -> String| {
        cs.animations.iter().map(f).collect::<Vec<_>>().join(", ")
    };
    let transitions = |f: &dyn Fn(&sk_css::TransitionSpec) -> String| {
        cs.transitions.iter().map(f).collect::<Vec<_>>().join(", ")
    };
    Some(match name {
        "display" => match cs.base.display.unwrap_or(gpui::Display::Block) {
            gpui::Display::Block => "block",
            gpui::Display::Flex => "flex",
            gpui::Display::Grid => "grid",
            gpui::Display::None => "none",
        }
        .into(),
        // `fixed` is laid out as absolute (see the mapper) but the AUTHORED value is what a
        // component reads back, and @floating-ui branches on exactly this string.
        "position" => match (cs.extras.fixed, cs.base.position) {
            (Some(true), _) => "fixed",
            _ if cs.extras.sticky == Some(true) => "sticky",
            (_, Some(gpui::Position::Absolute)) => "absolute",
            (_, Some(gpui::Position::Relative)) => "relative",
            (_, None) => "static",
        }
        .into(),
        // The CSS value, inherited, not gpui's paint switch: a screen-reader-only box paints nothing
        // but is `visible` (render::css_visible).
        "visibility" => if crate::render::css_visible(t, id) {
            "visible"
        } else {
            "hidden"
        }
        .into(),
        // `auto` and `clip` as computed (sk_css::Extras::overflow_names), not the `scroll` and
        // `hidden` gpui folds them into.
        "overflow-x" | "overflow-y" | "overflow" => {
            let [x, y] = cs.extras.overflow_names(&cs.base);
            match name {
                "overflow-x" => x.to_string(),
                "overflow-y" => y.to_string(),
                _ if x == y => x.to_string(),
                _ => format!("{x} {y}"),
            }
        }
        // `auto` is the CSS initial value AND an exact description of what we do: gpui's scrollbars
        // are overlays, so no gutter is ever reserved and none can be. (Reka's scroll lock reads it.)
        "scrollbar-gutter" => "auto".into(),
        "opacity" => format!("{}", cs.base.opacity.unwrap_or(1.0)),
        "z-index" => cs
            .extras
            .z()
            .map(|z| z.to_string())
            .unwrap_or_else(|| "auto".into()),
        "pointer-events" => if cs.extras.pointer_events_none == Some(true) {
            "none"
        } else {
            "auto"
        }
        .into(),
        // The function list, not the `matrix()` a browser resolves to: it is what the cascade holds,
        // and floating-ui only tests it against `"none"`.
        "transform" => transform_text(cs.base.vn_transform),
        // Resolved against the border box, as a browser's computed value is: Svelte's `flip` divides it
        // by the size.
        "transform-origin" => {
            let (w, h) = crate::render::bounds_of(id).map_or((0.0, 0.0), |(_, _, w, h)| (w, h));
            let at = |len: gpui::TransformLength, size: f32| match len {
                gpui::TransformLength::Pixels(px) => px,
                gpui::TransformLength::Fraction(f) => f * size,
            };
            let (ox, oy) = cs.base.vn_transform.map_or(
                (
                    gpui::TransformLength::Fraction(0.5),
                    gpui::TransformLength::Fraction(0.5),
                ),
                |t| t.origin,
            );
            format!("{}px {}px", at(ox, w), at(oy, h))
        }
        "zoom" => "1".into(),
        // The individual transform properties are not mapped (`transform` is the only one that
        // applies), plus three not implemented.
        "translate" | "rotate" | "scale" | "perspective" | "filter" => "none".into(),
        "will-change" => "auto".into(),
        "contain" => "none".into(),
        "container-type" => "normal".into(),
        "backdrop-filter" => match cs.base.backdrop_blur {
            Some(r) => format!("blur({}px)", f32::from(r)),
            None => "none".into(),
        },
        // reka-ui's `Presence` reads `animationName` to decide whether to wait for `animationend`, so
        // this must be the truth or an exit animation never plays or never finishes.
        "animation-name" if cs.animations.is_empty() => "none".into(),
        "animation-duration" | "animation-delay" if cs.animations.is_empty() => "0s".into(),
        "animation-fill-mode" if cs.animations.is_empty() => "none".into(),
        "animation-name" => animations(&|s| {
            if s.name.is_empty() {
                "none".into()
            } else {
                s.name.to_string()
            }
        }),
        "animation-duration" => animations(&|s| secs(s.duration)),
        "animation-delay" => animations(&|s| signed_secs(s.delay)),
        "animation-fill-mode" => animations(&|s| {
            match s.fill {
                sk_css::FillMode::None => "none",
                sk_css::FillMode::Forwards => "forwards",
                sk_css::FillMode::Backwards => "backwards",
                sk_css::FillMode::Both => "both",
            }
            .to_string()
        }),
        "transition-property" if cs.transitions.is_empty() => "all".into(),
        "transition-duration" | "transition-delay" if cs.transitions.is_empty() => "0s".into(),
        "transition-property" => transitions(&|s| s.property.name().to_string()),
        "transition-duration" => transitions(&|s| secs(s.duration)),
        "transition-delay" => transitions(&|s| signed_secs(s.delay)),
        "width" | "height" => {
            let (w, h) = crate::render::bounds_of(id)
                .map(|(_, _, w, h)| (w, h))
                .unwrap_or((0.0, 0.0));
            format!("{}px", if name == "width" { w } else { h })
        }
        _ if edge_of(name).is_some() => edge_px(t, id, cs, name)?,
        "direction" => "ltr".into(),
        "font-size" => {
            let mut px = None;
            let mut at = id;
            while let Some(n) = t.node(at) {
                if let Some(c) = n.computed.as_ref() {
                    px = Some(c.font_size);
                    break;
                }
                if n.parent == 0 || n.parent == at {
                    break;
                }
                at = n.parent;
            }
            format!("{}px", px.unwrap_or_else(sk_css::rem_basis))
        }
        "color" => {
            let mut color = None;
            let mut at = id;
            while let Some(n) = t.node(at) {
                if let Some(c) = n.computed.as_ref() {
                    color = color.or(c.base.text.color);
                }
                if color.is_some() || n.parent == 0 || n.parent == at {
                    break;
                }
                at = n.parent;
            }
            css_rgb(color.unwrap_or_else(gpui::black))
        }
        "text-shadow" => {
            let (mut shadows, mut color) = (None, None);
            let mut at = id;
            while let Some(n) = t.node(at) {
                if let Some(c) = n.computed.as_ref() {
                    shadows = shadows.or_else(|| c.base.text.text_shadow.clone());
                    color = color.or(c.base.text.color);
                }
                if (shadows.is_some() && color.is_some()) || n.parent == 0 || n.parent == at {
                    break;
                }
                at = n.parent;
            }
            sk_css::text_shadow_css(
                shadows.as_deref().unwrap_or_default(),
                color.unwrap_or_else(gpui::black),
            )
        }
        _ => return None,
    })
}

#[derive(Clone, Copy)]
enum EdgeBox {
    Padding,
    Margin,
    Border,
}

fn edge_of(name: &str) -> Option<(EdgeBox, &str)> {
    if let Some(side) = name.strip_prefix("padding-") {
        return Some((EdgeBox::Padding, side));
    }
    if let Some(side) = name.strip_prefix("margin-") {
        return Some((EdgeBox::Margin, side));
    }
    let side = name.strip_prefix("border-")?.strip_suffix("-width")?;
    Some((EdgeBox::Border, side))
}

fn side_of<T: Clone + std::fmt::Debug + Default + PartialEq>(
    edges: &gpui::EdgesRefinement<T>,
    side: &str,
) -> Option<T> {
    match side {
        "top" => edges.top.clone(),
        "right" => edges.right.clone(),
        "bottom" => edges.bottom.clone(),
        "left" => edges.left.clone(),
        _ => None,
    }
}

/// A padding, margin or border-width longhand as a browser resolves it: px, a percentage of the parent's width
/// (every side, as CSS has it), `auto` margins as 0.
fn edge_px(t: &Tree, id: u32, cs: &sk_css::ComputedStates, name: &str) -> Option<String> {
    let (which, side) = edge_of(name)?;
    let rem = gpui::px(sk_css::rem_basis());
    let parent_width = || {
        let parent = t.node(id).map(|n| n.parent).unwrap_or(0);
        let w = crate::render::bounds_of(parent).map_or(0.0, |(_, _, w, _)| w);
        gpui::AbsoluteLength::Pixels(gpui::px(w))
    };
    let px = match which {
        EdgeBox::Padding => side_of(&cs.base.padding, side)
            .map_or(gpui::px(0.0), |l| l.to_pixels(parent_width(), rem)),
        EdgeBox::Margin => match side_of(&cs.base.margin, side) {
            Some(gpui::Length::Definite(l)) => l.to_pixels(parent_width(), rem),
            Some(gpui::Length::Auto) | None => gpui::px(0.0),
        },
        EdgeBox::Border => {
            side_of(&cs.base.border_widths, side).map_or(gpui::px(0.0), |l| l.to_pixels(rem))
        }
    };
    Some(format!("{}px", f32::from(px)))
}

fn secs(d: std::time::Duration) -> String {
    format!("{}s", d.as_secs_f32())
}

fn signed_secs(d: sk_css::Delay) -> String {
    format!("{}s", d.as_secs_f64() as f32)
}

fn css_rgb(c: gpui::Hsla) -> String {
    let c = gpui::Rgba::from(c);
    let ch = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
    let (r, g, b) = (ch(c.r), ch(c.g), ch(c.b));
    if c.a >= 1.0 {
        format!("rgb({r}, {g}, {b})")
    } else {
        let a = (c.a.clamp(0.0, 1.0) * 100.0).round() / 100.0;
        format!("rgba({r}, {g}, {b}, {a})")
    }
}

fn transform_text(t: Option<gpui::TransformSpec>) -> String {
    t.map(sk_css::transform_css)
        .unwrap_or_else(|| "none".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn prop(id: u32, name: &str) -> String {
        let head = format!("{name}=");
        computed_style(id)
            .unwrap_or_default()
            .lines()
            .find_map(|l| l.strip_prefix(head.as_str()).map(str::to_string))
            .unwrap_or_default()
    }

    #[test]
    fn document_queries_include_the_document_element_but_element_queries_exclude_self() {
        let (root, child) = tree::with_tree(|tree| {
            let root = tree.create_root();
            tree.set_class(root, "match");
            let tag = tree.intern("div");
            let child = tree.create_element(tag);
            tree.set_class(child, "match");
            tree.insert(child, root, 0);
            (root, child)
        });
        assert_eq!(
            query(".match", root, QueryMode::Document).unwrap(),
            vec![root, child]
        );
        assert_eq!(
            query(".match", root, QueryMode::Descendants).unwrap(),
            vec![child]
        );
        assert_eq!(
            query(":root", root, QueryMode::Document).unwrap(),
            vec![root]
        );
        assert!(query(":root", root, QueryMode::Descendants)
            .unwrap()
            .is_empty());
    }

    #[test]
    fn computed_style_reports_the_inherited_font_size_and_color() {
        let (outer, inner, bare) = tree::with_tree(|t| {
            let root = t.create_root();
            let div = t.intern("div");
            let outer = t.create_element(div);
            t.insert(outer, root, 0);
            t.set_class(outer, "outer");
            let inner = t.create_element(div);
            t.insert(inner, outer, 0);
            let bare = t.create_element(div);
            t.insert(bare, root, 1);
            (outer, inner, bare)
        });
        tree::set_engine(sk_css::Engine::compile(
            ".outer { font-size: 20px; color: rgba(255, 0, 0, 0.5) }",
        ));
        tree::restyle_all();
        assert_eq!(prop(outer, "font-size"), "20px");
        assert_eq!(prop(inner, "font-size"), "20px", "inherited from .outer");
        assert_eq!(prop(inner, "color"), "rgba(255, 0, 0, 0.5)");
        assert_eq!(
            prop(bare, "font-size"),
            format!("{}px", sk_css::rem_basis())
        );
        assert_eq!(prop(bare, "color"), "rgb(0, 0, 0)");
        for name in [
            "width",
            "height",
            "position",
            "direction",
            "animation-name",
            "transition-delay",
            "transition-duration",
        ] {
            assert!(!prop(inner, name).is_empty(), "{name}");
        }
    }

    #[test]
    fn computed_animation_none_cancels_presence_and_preserves_mixed_list_slots() {
        let (single, mixed) = tree::with_tree(|t| {
            let root = t.create_root();
            let div = t.intern("div");
            let single = t.create_element(div);
            t.set_class(single, "single");
            t.insert(single, root, 0);
            let mixed = t.create_element(div);
            t.set_class(mixed, "mixed");
            t.insert(mixed, root, 1);
            (single, mixed)
        });
        tree::set_engine(sk_css::Engine::compile(
            "@keyframes spin { from { opacity: 0 } to { opacity: 1 } } \
             @keyframes fade { from { opacity: 1 } to { opacity: 0 } } \
             .single { animation-name: spin; animation-duration: 250ms; animation-fill-mode: forwards } \
             .mixed { animation-name: none, spin, none, fade; animation-duration: 100ms, 300ms; animation-delay: 20ms }",
        ));
        tree::restyle_all();
        assert_eq!(prop(single, "animation-name"), "spin");
        assert_eq!(prop(mixed, "animation-name"), "none, spin, none, fade");
        assert_eq!(prop(mixed, "animation-duration"), "0.1s, 0.3s, 0.1s, 0.3s");
        assert_eq!(prop(mixed, "animation-delay"), "0.02s, 0.02s, 0.02s, 0.02s");

        // The none slot keeps the other components, but must serialize the literal keyword so it
        // does not wait for animationend from an inert animation.
        tree::with_tree(|t| {
            let name = t.intern("animation-name");
            t.set_style(single, name, "none".into());
        });
        tree::restyle_all();
        assert_eq!(prop(single, "animation-name"), "none");
        assert_eq!(prop(single, "animation-duration"), "0.25s");
        assert_eq!(prop(single, "animation-fill-mode"), "forwards");
    }

    #[test]
    fn computed_box_edges_resolve_to_px() {
        let (boxed, bare) = tree::with_tree(|t| {
            let root = t.create_root();
            let div = t.intern("div");
            let boxed = t.create_element(div);
            t.set_class(boxed, "boxed");
            t.insert(boxed, root, 0);
            let bare = t.create_element(div);
            t.insert(bare, root, 1);
            (boxed, bare)
        });
        tree::set_engine(sk_css::Engine::compile(
            ".boxed { padding: 4px 1rem; margin: 3px auto 0; border-top: 2px solid red }",
        ));
        tree::restyle_all();
        let rem = format!("{}px", sk_css::rem_basis());
        assert_eq!(prop(boxed, "padding-top"), "4px");
        assert_eq!(prop(boxed, "padding-left"), rem);
        assert_eq!(prop(boxed, "margin-top"), "3px");
        assert_eq!(prop(boxed, "margin-right"), "0px", "auto");
        assert_eq!(prop(boxed, "border-top-width"), "2px");
        assert_eq!(prop(boxed, "border-bottom-width"), "0px");
        for name in ["padding-bottom", "margin-left", "border-left-width"] {
            assert_eq!(prop(bare, name), "0px", "{name}");
        }
    }

    #[test]
    fn computed_overflow_pairs_the_axes_and_reports_auto_and_clip() {
        let (list, wide, cut, plain) = tree::with_tree(|t| {
            let root = t.create_root();
            let div = t.intern("div");
            let mut make = |class: &str, at: u32| {
                let n = t.create_element(div);
                t.insert(n, root, at);
                if !class.is_empty() {
                    t.set_class(n, class);
                }
                n
            };
            (
                make("list", 0),
                make("wide", 1),
                make("cut", 2),
                make("", 3),
            )
        });
        tree::set_engine(sk_css::Engine::compile(
            ".list { overflow-y: auto } .wide { overflow-x: hidden } .cut { overflow-x: clip }",
        ));
        tree::restyle_all();
        let axes = |id| {
            [
                prop(id, "overflow-x"),
                prop(id, "overflow-y"),
                prop(id, "overflow"),
            ]
        };
        assert_eq!(axes(list), ["auto", "auto", "auto"]);
        assert_eq!(axes(wide), ["hidden", "auto", "hidden auto"]);
        assert_eq!(axes(cut), ["clip", "visible", "clip visible"]);
        assert_eq!(axes(plain), ["visible", "visible", "visible"]);
        let x = tree::with_tree(|t| {
            t.node(list)
                .and_then(|n| n.computed.as_ref())
                .map(|cs| cs.base.overflow.x)
        });
        assert_eq!(x, Some(Some(gpui::Overflow::Scroll)));
    }
}
