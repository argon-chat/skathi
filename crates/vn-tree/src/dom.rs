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

/// `getBoundingClientRect()` — `(x, y, width, height)` in CSS px, from the last
/// frame the node was painted in, relative to **the app root**.
///
/// The app root is JS's viewport: it is the box `position: fixed` resolves
/// against (see the mapper), it is what `document.documentElement` reports, and
/// it is where a `<Teleport to="body">` lands. Reporting window-client
/// coordinates instead would leave @floating-ui measuring elements in one space
/// and clipping them against another, offset by whatever chrome the host draws
/// around the root — and every popover pinned to the left edge by the `shift`
/// middleware, which is exactly how that failure looks.
///
/// (Devtools keeps the WINDOW-relative numbers — see [`crate::render::bounds_of`]
/// — because a screenshot is a window, not a root. The two differ by the root's
/// origin and nothing else.)
///
/// `None` = never painted this frame: detached, `display: none`, or parked in a
/// `<KeepAlive>` cache container. A browser answers an all-zero rect for all
/// three, which is what the JS side substitutes.
pub fn bounds(id: u32) -> Option<(f32, f32, f32, f32)> {
    let (x, y, w, h) = crate::render::bounds_of(id)?;
    let (ox, oy) = crate::render::bounds_of(tree::read_tree(|t| t.root))
        .map(|(rx, ry, _, _)| (rx, ry))
        .unwrap_or((0.0, 0.0));
    Some((x - ox, y - oy, w, h))
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
        // A node that exists but has not been cascaded yet (created earlier in
        // this same JS wave) computes to the initial values, not to nothing —
        // a browser has no "not styled yet" state to report.
        let default = vn_css::ComputedStates::default();
        let cs = t.node(id)?.computed.as_ref().unwrap_or(&default);
        let mut out = String::new();
        let mut put = |k: &str, v: String| {
            out.push_str(k);
            out.push('=');
            out.push_str(&v);
            out.push('\n');
        };

        // Every property in the published set ALWAYS has a value: a browser's
        // computed style has no "absent", and a component that reads `display`
        // expects a keyword, not "". So an undeclared property reports its CSS
        // initial value and only a property OUTSIDE the set reads "" (with the
        // advisory the JS side prints).
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
        // `fixed` is laid out as absolute (see the mapper) but the AUTHORED
        // value is what a component reads back, and @floating-ui branches on
        // exactly this string.
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
        put(
            "visibility",
            match cs.base.visibility.unwrap_or(gpui::Visibility::Visible) {
                gpui::Visibility::Visible => "visible",
                gpui::Visibility::Hidden => "hidden",
            }
            .into(),
        );
        let overflow_name = |o: Option<gpui::Overflow>| match o.unwrap_or(gpui::Overflow::Visible) {
            gpui::Overflow::Visible => "visible",
            gpui::Overflow::Hidden => "hidden",
            gpui::Overflow::Clip => "clip",
            gpui::Overflow::Scroll => "scroll",
        };
        put("overflow-x", overflow_name(cs.base.overflow.x).into());
        put("overflow-y", overflow_name(cs.base.overflow.y).into());
        // The shorthand, which is what `isOverflowElement` in @floating-ui reads.
        put(
            "overflow",
            format!("{} {}", overflow_name(cs.base.overflow.x), overflow_name(cs.base.overflow.y)),
        );
        put("opacity", format!("{}", cs.base.opacity.unwrap_or(1.0)));
        put(
            "z-index",
            cs.extras.z_index.map(|z| z.to_string()).unwrap_or_else(|| "auto".into()),
        );
        put(
            "pointer-events",
            if cs.extras.pointer_events_none == Some(true) { "none" } else { "auto" }.into(),
        );
        // The FULL transform now — translate, rotate and scale all reach the
        // paint (m13 item 2), so all three are reported. The serialisation is
        // the function list rather than the `matrix()` a browser resolves to,
        // because that is what the cascade actually holds; floating-ui's
        // `isContainingBlock` only tests it against `"none"`.
        put("transform", transform_text(cs.base.vn_transform));
        // The containing-block probe every floating-ui build runs. These are
        // the INDIVIDUAL transform properties, which vue-native does not map —
        // `transform` above is the shorthand and the only one that applies —
        // plus three that are genuinely not implemented. Reporting the initial
        // value is honest: it IS what happens.
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
        // The animation list. reka-ui's `Presence` reads `animationName` to
        // decide whether to wait for an `animationend` at all — `none` means
        // "unmount now", anything else means "wait for the event" — so this has
        // to be the truth, or an exit animation either never plays or never
        // finishes.
        if cs.animations.is_empty() {
            put("animation-name", "none".into());
            put("animation-duration", "0s".into());
            put("animation-delay", "0s".into());
            put("animation-fill-mode", "none".into());
        } else {
            let join = |f: &dyn Fn(&vn_css::AnimationSpec) -> String| {
                cs.animations.iter().map(f).collect::<Vec<_>>().join(", ")
            };
            put("animation-name", join(&|s| s.name.to_string()));
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
        // number exists. A node that has not been painted reports `0px`, which
        // is what a browser reports for a `display: none` one.
        let (w, h) = crate::render::bounds_of(id).map(|(_, _, w, h)| (w, h)).unwrap_or((0.0, 0.0));
        put("width", format!("{w}px"));
        put("height", format!("{h}px"));
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

fn css_len(v: gpui::TransformLength) -> String {
    match v {
        gpui::TransformLength::Pixels(p) => format!("{p}px"),
        gpui::TransformLength::Fraction(f) => format!("{}%", f * 100.0),
    }
}

/// `getComputedStyle().transform`, as the function list the cascade computed.
///
/// A browser resolves the shorthand to a `matrix(...)`; we serialise the parts,
/// which is strictly more informative and matches what every consumer in the
/// reused-component set actually does with it (compare against `"none"`).
fn transform_text(t: Option<gpui::TransformSpec>) -> String {
    let Some(t) = t.filter(|t| !t.is_identity()) else {
        return "none".into();
    };
    let mut parts: Vec<String> = Vec::new();
    if !t.translate.0.is_zero() || !t.translate.1.is_zero() {
        parts.push(format!(
            "translate({}, {})",
            css_len(t.translate.0),
            css_len(t.translate.1)
        ));
    }
    if t.rotate != 0.0 {
        parts.push(format!("rotate({}deg)", t.rotate.to_degrees()));
    }
    if t.scale != (1.0, 1.0) {
        parts.push(format!("scale({}, {})", t.scale.0, t.scale.1));
    }
    parts.join(" ")
}
