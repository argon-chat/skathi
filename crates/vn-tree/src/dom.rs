//! The DOM surface the JS mirror nodes answer from (compat roadmap G2).
//!
//! `packages/vue-native/src/runtime.js` puts `closest` / `matches` / `contains`
//! / `querySelector(All)` / `getBoundingClientRect` / `getComputedStyle` on the
//! mirror node prototype. Two of those three questions can only be answered
//! here:
//!
//! * **selector matching** lives in the Rust cascade (`vn_css::query_all`), and
//!   answering it in JS would mean a second, divergent matcher — the exact
//!   silent-divergence this project exists to avoid. So the mirror asks the real
//!   one, synchronously, through [`query`].
//! * **layout** exists only as the rectangle gpui hands each element during
//!   prepaint; `render::BOUNDS` captures it per painted node, and [`bounds`]
//!   reads it back. A node that was not painted reports nothing, which the JS
//!   side turns into an all-zero rect — what a browser reports for a detached
//!   or `display: none` element.
//!
//! Everything is serialized as a small line/comma string rather than JSON: the
//! payloads are ids and numbers, the callers are three lines of JS each, and a
//! JSON round trip per `getBoundingClientRect()` would be the wrong tax on a
//! call floating-ui makes dozens of times per open.

use crate::tree::{self, Kind, NodeRef, Tree};

/// What a [`query`] call is asking for.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum QueryMode {
    /// `root.querySelectorAll(sel)` — every DESCENDANT of `root` that matches,
    /// in document order. `root` itself is never a candidate, as in the DOM.
    Descendants,
    /// `el.matches(sel)` — the node itself, nothing else.
    SelfOnly,
    /// `el.closest(sel)` — the node, then its ancestors, nearest first; the
    /// first match wins.
    Closest,
}

/// Runs `selector` against the retained tree through the cascade's own matcher.
///
/// `Err` carries the same advisory the sheet compiler prints for a selector the
/// engine cannot run (`:has()`, `::before`, `:visited`), so a `querySelector`
/// that cannot be answered explains itself instead of reporting "no match".
pub fn query(selector: &str, root: u32, mode: QueryMode) -> Result<Vec<u32>, String> {
    tree::read_tree(|t| {
        if t.node(root).is_none() {
            // A detached node still exists in the mirror; in the DOM its
            // queries simply find nothing.
            return Ok(Vec::new());
        }
        let ids: Vec<u32> = match mode {
            QueryMode::Descendants => descendants(t, root),
            QueryMode::SelfOnly => vec![root],
            QueryMode::Closest => ancestors_from(t, root),
        };
        let candidates: Vec<NodeRef<'_>> = ids.iter().map(|&id| NodeRef { tree: t, id }).collect();
        let result = vn_css::query_all(selector, &candidates)?;
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

/// `getBoundingClientRect()` — `(x, y, width, height)` in CSS px, relative to
/// the window's client area, from the last frame the node was painted in.
///
/// `None` = never painted this frame: detached, `display: none`, or parked in a
/// `<KeepAlive>` cache container. A browser answers an all-zero rect for all
/// three, which is what the JS side substitutes.
pub fn bounds(id: u32) -> Option<(f32, f32, f32, f32)> {
    crate::render::bounds_of(id)
}

/// `getComputedStyle(el)` — the useful subset, as `name=value` lines.
///
/// A browser's `CSSStyleDeclaration` has ~340 longhands, every one of them
/// resolved. We have a `StyleRefinement`, which is the folded set of
/// declarations that actually applied, plus the element's custom properties.
/// So this reports:
///
/// * the properties reused browser components actually read at runtime —
///   `transition-*` (Vue's own `<Transition>` measures its timeout with them),
///   `transform`, `display`, `position`, `overflow-*`, `direction`;
/// * the USED width/height, off the painted rectangle, because that is the one
///   place a real number exists;
/// * every custom property, so `getPropertyValue("--x")` works.
///
/// Properties nothing declared are simply absent — the JS side answers `""`
/// for them and warns once, rather than inventing an initial value it would
/// have to keep in sync with a browser's.
pub fn computed_style(id: u32) -> Option<String> {
    tree::read_tree(|t| {
        let cs = t.node(id)?.computed.as_ref()?;
        let mut out = String::new();
        let mut put = |k: &str, v: String| {
            out.push_str(k);
            out.push('=');
            out.push_str(&v);
            out.push('\n');
        };

        if let Some(d) = cs.base.display {
            put(
                "display",
                match d {
                    gpui::Display::Block => "block",
                    gpui::Display::Flex => "flex",
                    gpui::Display::Grid => "grid",
                    gpui::Display::None => "none",
                }
                .into(),
            );
        }
        if let Some(p) = cs.base.position {
            // `fixed` is laid out as absolute (see the mapper) but the AUTHORED
            // value is what a component reads back, and floating-ui branches on
            // exactly this string.
            put(
                "position",
                match (cs.extras.fixed, p) {
                    (Some(true), _) => "fixed",
                    (_, gpui::Position::Absolute) => "absolute",
                    (_, gpui::Position::Relative) => "relative",
                }
                .into(),
            );
        }
        if let Some(v) = cs.base.visibility {
            put(
                "visibility",
                match v {
                    gpui::Visibility::Visible => "visible",
                    gpui::Visibility::Hidden => "hidden",
                }
                .into(),
            );
        }
        for (name, axis) in [("overflow-x", cs.base.overflow.x), ("overflow-y", cs.base.overflow.y)] {
            if let Some(o) = axis {
                put(
                    name,
                    match o {
                        gpui::Overflow::Visible => "visible",
                        gpui::Overflow::Hidden => "hidden",
                        gpui::Overflow::Clip => "clip",
                        gpui::Overflow::Scroll => "scroll",
                    }
                    .into(),
                );
            }
        }
        if let Some(o) = cs.base.opacity {
            put("opacity", format!("{o}"));
        }
        if let Some(z) = cs.extras.z_index {
            put("z-index", format!("{z}"));
        }
        if cs.extras.pointer_events_none.is_some() {
            put(
                "pointer-events",
                if cs.extras.pointer_events_none == Some(true) { "none" } else { "auto" }.into(),
            );
        }
        put(
            "transform",
            match cs.extras.translate {
                Some((x, y)) => format!("translate({}, {})", css_len(x), css_len(y)),
                None => "none".into(),
            },
        );
        // The transition list, in the shape runtime-dom's `<Transition>` reads
        // it: a comma-joined value per longhand, one entry per list item.
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
        // The USED size, i.e. the painted rectangle — the only place a real
        // number exists. Absent (rather than "0px") when the node has not been
        // painted, which is the honest answer.
        if let Some((_, _, w, h)) = crate::render::bounds_of(id) {
            put("width", format!("{w}px"));
            put("height", format!("{h}px"));
        }
        // gpui lays out left-to-right unconditionally; floating-ui asks.
        put("direction", "ltr".into());
        for (name, value) in cs.vars.iter() {
            put(name, value.to_string());
        }
        Some(out)
    })
}

fn secs(d: std::time::Duration) -> String {
    format!("{}s", d.as_secs_f32())
}

fn css_len(v: vn_css::TranslateLen) -> String {
    match v {
        vn_css::TranslateLen::Px(p) => format!("{p}px"),
        vn_css::TranslateLen::Percent(f) => format!("{}%", f * 100.0),
    }
}
