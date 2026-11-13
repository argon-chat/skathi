//! The DOM surface the JS mirror nodes answer from (compat roadmap G2).

use crate::live_state::{LiveRef, LiveStates};
use crate::tree::{self, Kind, Tree};

/// What a [`query`] call is asking for.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum QueryMode {
    /// `root.querySelectorAll(sel)` — every DESCENDANT of `root` that matches, in document order.
    Descendants,
    /// `document.querySelectorAll(sel)` — the document element followed by every descendant, in
    /// document order.
    Document,
    /// `el.matches(sel)` — the node itself, nothing else.
    SelfOnly,
    /// `el.closest(sel)` — the node, then its ancestors, nearest first; the first match wins.
    Closest,
}

/// Runs `selector` against the retained tree through the cascade's own matcher.
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
        // A script's query filters `:hover` / `:active` / `:focus` / `:focus-visible` /
        // `:focus-within` by the live state, as a browser's does; the cascade's matcher answers
        // them "would match" (`crate::live_state`).
        let live = LiveStates::take(t);
        let candidates: Vec<LiveRef<'_>> = ids.iter().map(|&id| LiveRef::new(t, id, &live)).collect();
        let result = vn_css::query_all_in(selector, &candidates, Some(LiveRef::new(t, root, &live)))?;
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

/// Every element under `id`, in document order, `id` excluded.
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

/// `id` then every ancestor, nearest first — `closest()`'s candidate order.
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

/// A browser lays out on read; so does this. Only for a node with no rectangle anywhere while the
/// tree holds changes no frame has taken (built this same JS wave: boot, a `v-if` that just opened),
/// which `onMounted` + `nextTick` measures. Returns `true` when the answer stays STALE: the host
/// cannot draw inside a gpui input listener, so the layout could not be forced.
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

/// The same forced layout, for a read whose answer depends on the node's CONTENT rather than only
/// on its own box: `scrollWidth` / `scrollHeight`, and every write that has to be clamped against
/// them. Also for a node that HAS a rectangle: `list.append(row); list.scrollTop = list.scrollHeight`
/// would otherwise clamp against the height from before the append. Returns `true` when stale
/// (`sync_layout` cannot draw inside the window's own update; the build counter says whether it did).
#[must_use]
pub(crate) fn layout_if_stale(id: u32) -> bool {
    if id == 0 || !tree::read_tree(|t| t.node(id).is_some()) || !crate::layout_owed() {
        return false;
    }
    let before = crate::render::build_count();
    crate::sync_layout_by(crate::perf::SyncCaller::Scroll);
    crate::render::build_count() == before
}

/// `getBoundingClientRect()` in CSS px relative to the APP ROOT (JS's viewport, what `position: fixed`
/// resolves against); window-client coordinates would make floating-ui measure and clip in different
/// spaces. `None` = not painted (detached, `display: none`, `<KeepAlive>`). The `bool` says a missing
/// answer is STALE, not "not painted".
pub fn bounds(id: u32) -> (Option<(f32, f32, f32, f32)>, bool) {
    let stale = force_layout_if_unpainted(id);
    (bounds_now(id), stale)
}

/// Where CSS `transform` drew the node, in the same space as [`bounds`]: the bounding box of its
/// border box under its own transform and its ancestors'.
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

fn bounds_now(id: u32) -> Option<(f32, f32, f32, f32)> {
    let (x, y, w, h) = crate::render::bounds_of(id)?;
    let (ox, oy) = root_origin();
    let (sx, sy) = crate::scroll::unpainted_shift(id);
    Some((x - ox + sx, y - oy + sy, w, h))
}

/// `getComputedStyle(el)` — the useful subset, as `name=value` lines.
pub fn computed_style(id: u32) -> Option<String> {
    tree::flush_pending_style();
    tree::read_tree(|t| {
        let default = vn_css::ComputedStates::default();
        let cs = t.node(id)?.computed.as_ref().unwrap_or(&default);
        let mut out = String::new();
        let mut put = |k: &str, v: String| {
            out.push_str(k);
            out.push('=');
            out.push_str(&v);
            out.push('\n');
        };

        // Every property in the published set ALWAYS has a value: a browser's computed style has no
        // "absent", and a component that reads `display` expects a keyword, not "".
        put(
            "display",
            match cs.base.display.unwrap_or(gpui::Display::Block) {
                gpui::Display::Block => "block",
                gpui::Display::Flex => "flex",
                gpui::Display::Grid => "grid",
                gpui::Display::None => "none",
            }
            .into(),
        );
        // `fixed` is laid out as absolute (see the mapper) but the AUTHORED value is what a
        // component reads back, and @floating-ui branches on exactly this string.
        put(
            "position",
            match (cs.extras.fixed, cs.base.position) {
                (Some(true), _) => "fixed",
                (_, Some(gpui::Position::Absolute)) => "absolute",
                (_, Some(gpui::Position::Relative)) => "relative",
                (_, None) => "static",
            }
            .into(),
        );
        // The CSS value, inherited, not gpui's paint switch: a screen-reader-only box paints
        // nothing but is `visible`, and a child of a `hidden` box is `hidden`
        // (render::css_visible).
        put("visibility", if crate::render::css_visible(t, id) { "visible" } else { "hidden" }.into());
        // `auto` and `clip` as computed (vn_css::Extras::overflow_names), not the `scroll` and
        // `hidden` gpui folds them into.
        let [overflow_x, overflow_y] = cs.extras.overflow_names(&cs.base);
        put("overflow-x", overflow_x.into());
        put("overflow-y", overflow_y.into());
        put(
            "overflow",
            if overflow_x == overflow_y { overflow_x.to_string() } else { format!("{overflow_x} {overflow_y}") },
        );
        // `auto` is the CSS initial value AND an exact description of what we do: gpui's scrollbars
        // are overlays, so no gutter is ever reserved and none can be. (Reka's scroll lock reads it.)
        put("scrollbar-gutter", "auto".into());
        put("opacity", format!("{}", cs.base.opacity.unwrap_or(1.0)));
        put(
            "z-index",
            cs.extras.z().map(|z| z.to_string()).unwrap_or_else(|| "auto".into()),
        );
        put(
            "pointer-events",
            if cs.extras.pointer_events_none == Some(true) { "none" } else { "auto" }.into(),
        );
        // The serialisation is the function list rather than the `matrix()` a browser resolves to,
        // because that is what the cascade actually holds; floating-ui's `isContainingBlock` only
        // tests it against `"none"`.
        put("transform", transform_text(cs.base.vn_transform));
        // These are the INDIVIDUAL transform properties, which vue-native does not map —
        // `transform` above is the shorthand and the only one that applies — plus three that are
        // genuinely not implemented.
        for name in ["translate", "rotate", "scale", "perspective", "filter"] {
            put(name, "none".into());
        }
        put("will-change", "auto".into());
        put("contain", "none".into());
        put("container-type", "normal".into());
        put(
            "backdrop-filter",
            match cs.base.backdrop_blur {
                Some(r) => format!("blur({}px)", f32::from(r)),
                None => "none".into(),
            },
        );
        // reka-ui's `Presence` reads `animationName` to decide whether to wait for `animationend`, so
        // this must be the truth or an exit animation never plays or never finishes.
        if cs.animations.is_empty() {
            put("animation-name", "none".into());
            put("animation-duration", "0s".into());
            put("animation-delay", "0s".into());
            put("animation-fill-mode", "none".into());
        } else {
            let join = |f: &dyn Fn(&vn_css::AnimationSpec) -> String| {
                cs.animations.iter().map(f).collect::<Vec<_>>().join(", ")
            };
            put("animation-name", join(&|s| {
                if s.name.is_empty() { "none".into() } else { s.name.to_string() }
            }));
            put("animation-duration", join(&|s| secs(s.duration)));
            put("animation-delay", join(&|s| secs(s.delay)));
            put(
                "animation-fill-mode",
                join(&|s| {
                    match s.fill {
                        vn_css::FillMode::None => "none",
                        vn_css::FillMode::Forwards => "forwards",
                        vn_css::FillMode::Backwards => "backwards",
                        vn_css::FillMode::Both => "both",
                    }
                    .to_string()
                }),
            );
        }
        if cs.transitions.is_empty() {
            put("transition-property", "all".into());
            put("transition-duration", "0s".into());
            put("transition-delay", "0s".into());
        } else {
            let join = |f: &dyn Fn(&vn_css::TransitionSpec) -> String| {
                cs.transitions.iter().map(f).collect::<Vec<_>>().join(", ")
            };
            put("transition-property", join(&|s| s.property.name().to_string()));
            put("transition-duration", join(&|s| secs(s.duration)));
            put("transition-delay", join(&|s| secs(s.delay)));
        }
        // The USED size, i.e. the painted rectangle — the only place a real number exists.
        let (w, h) = crate::render::bounds_of(id).map(|(_, _, w, h)| (w, h)).unwrap_or((0.0, 0.0));
        put("width", format!("{w}px"));
        put("height", format!("{h}px"));
        // gpui lays out left-to-right unconditionally; floating-ui asks.
        put("direction", "ltr".into());
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
        // A node not cascaded yet inherits: the nearest cascaded ancestor's, and with none the
        // root's, the rem basis.
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
        let px = px.unwrap_or_else(vn_css::rem_basis);
        put("font-size", format!("{px}px"));
        put("color", css_rgb(color.unwrap_or_else(gpui::black)));
        for (name, value) in cs.vars.iter() {
            put(name, value.to_string());
        }
        Some(out)
    })
}

fn secs(d: std::time::Duration) -> String {
    format!("{}s", d.as_secs_f32())
}

/// A colour as a browser serialises a computed one: `rgb(r, g, b)`, or `rgba(r, g, b, a)` when it
/// is not opaque.
fn css_rgb(c: gpui::Hsla) -> String {
    let c = gpui::Rgba::from(c);
    let ch = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
    let (r, g, b) = (ch(c.r), ch(c.g), ch(c.b));
    if c.a >= 1.0 {
        format!("rgb({r}, {g}, {b})")
    } else {
        // Two decimals at most, as browsers print it (0.5, not 0.500000).
        let a = (c.a.clamp(0.0, 1.0) * 100.0).round() / 100.0;
        format!("rgba({r}, {g}, {b}, {a})")
    }
}

/// CSSOM and WAAPI serialize the same full affine transform.
fn transform_text(t: Option<gpui::TransformSpec>) -> String {
    t.map(vn_css::transform_css).unwrap_or_else(|| "none".into())
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
        assert_eq!(query(".match", root, QueryMode::Document).unwrap(), vec![root, child]);
        assert_eq!(query(".match", root, QueryMode::Descendants).unwrap(), vec![child]);
        assert_eq!(query(":root", root, QueryMode::Document).unwrap(), vec![root]);
        assert!(query(":root", root, QueryMode::Descendants).unwrap().is_empty());
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
        tree::set_engine(vn_css::Engine::compile(".outer { font-size: 20px; color: rgba(255, 0, 0, 0.5) }"));
        tree::restyle_all();
        assert_eq!(prop(outer, "font-size"), "20px");
        assert_eq!(prop(inner, "font-size"), "20px", "inherited from .outer");
        assert_eq!(prop(inner, "color"), "rgba(255, 0, 0, 0.5)");
        assert_eq!(prop(bare, "font-size"), format!("{}px", vn_css::rem_basis()));
        assert_eq!(prop(bare, "color"), "rgb(0, 0, 0)");
        for name in ["width", "height", "position", "direction", "animation-name", "transition-delay", "transition-duration"] {
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
        tree::set_engine(vn_css::Engine::compile(
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

    /// `.cut` is `clip` beside `visible`, which pairs with nothing.
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
            (make("list", 0), make("wide", 1), make("cut", 2), make("", 3))
        });
        tree::set_engine(vn_css::Engine::compile(
            ".list { overflow-y: auto } .wide { overflow-x: hidden } .cut { overflow-x: clip }",
        ));
        tree::restyle_all();
        let axes = |id| [prop(id, "overflow-x"), prop(id, "overflow-y"), prop(id, "overflow")];
        assert_eq!(axes(list), ["auto", "auto", "auto"]);
        assert_eq!(axes(wide), ["hidden", "auto", "hidden auto"]);
        assert_eq!(axes(cut), ["clip", "visible", "clip visible"]);
        assert_eq!(axes(plain), ["visible", "visible", "visible"]);
        let x = tree::with_tree(|t| t.node(list).and_then(|n| n.computed.as_ref()).map(|cs| cs.base.overflow.x));
        assert_eq!(x, Some(Some(gpui::Overflow::Scroll)));
    }
}
