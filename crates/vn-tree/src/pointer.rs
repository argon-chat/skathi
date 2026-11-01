//! Which element a mouse event is FOR: the topmost one under the pointer.
//!
//! A browser hit-tests once — the topmost element under the pointer that
//! `pointer-events` lets be hit — and the event goes to it and bubbles through
//! its ancestors. gpui instead hands a mouse event to EVERY hitbox under the
//! pointer, and gpui gives a hitbox only to an element that is interactive.
//! Together that made the rule "the innermost box that LISTENS takes it": a
//! box that listens for nothing let a press fall through to whatever listening
//! box lay under it, a sibling or an unrelated overlay included, and
//! `e.target` was the listening box, never the element pressed (E-2 in
//! docs/RUNTIME.md, now retired). A context menu over its dismissal shield
//! closed on its own items' presses (research/reports/m26/text-selection-1.md
//! step 0); a `@click.self` backdrop closed on a click at the picture in it.
//!
//! The browser's rule, in three parts:
//!
//! * **Every box can be hit.** gpui records a PICK for each element that gets
//!   no hitbox (`Window::set_vn_pick_every_box`, fork item 39): its box,
//!   content mask, transform and node tag, in the tagged list only. A pick is
//!   one small record per element per painted frame, replayed with a cached
//!   view's prepaint, and gpui's per-move hit test never looks at it. (A host
//!   with devtools gives every element a real hitbox anyway, for the
//!   inspector's `pick`; the picks are what a `no-devtools` build has.)
//! * **The target is resolved once per press, release and wheel**, before any
//!   listener runs ([`refine`], the fork's hit refiner): the topmost tagged box
//!   under the pointer ([`gpui::Window::vn_tags_at`], in paint order) whose
//!   element is not `pointer-events: none` and not inside a
//!   `visibility: hidden` subtree. Nothing hittable: the window's root.
//! * **Only the target's path hears it.** The refiner narrows gpui's hovered
//!   set for the event to the hitboxes of the target and its ancestors, and
//!   adds the ancestors' boxes that do not contain the pointer. Everything
//!   downstream keys off that set, so it all follows at once: the listeners
//!   [`crate::render`] binds per element, a text field's own caret placement,
//!   gpui's focus-on-press, the click tracking, the scroll containers a wheel
//!   moves. The renderer then dispatches the DOM event AT the target
//!   ([`target_or`]); a `click` goes to the nearest common ancestor of the
//!   press's target and the release's ([`click_target_or`]), as in Chromium.
//!
//! `mousemove` is not refined (moves are frequent, and resolving one is a walk
//! over every box): its one dispatch per frame resolves its own target
//! ([`target_at`]). **Hover stays geometric** — `:hover`, `:active`,
//! `mouseenter` and `mouseleave` still come from gpui's own hit test, so an
//! element under a cover that listens for nothing stays hovered (H-1 in
//! docs/RUNTIME.md). A press on such a cover says so once ([`note_covered`]).

use gpui::{App, Pixels, Point, VnHitKind, Window};

use crate::tree::{self, Kind, Tree};
use crate::windows;

/// The pointer events a covered listener would have taken: what the dev log
/// line names.
const PRESS_BITS: u64 = (1 << tree::EV_CLICK)
    | (1 << tree::EV_MOUSEDOWN)
    | (1 << tree::EV_MOUSEUP)
    | (1 << tree::EV_CONTEXTMENU);

/// The listeners whose element shows a hover of its own.
const HOVER_BITS: u64 = (1 << tree::EV_MOUSEENTER) | (1 << tree::EV_MOUSELEAVE);

/// Turns the targeting on for this window: picks for every element, and the
/// refiner. Called once per build (`render::arm_pick_hitboxes`); both are
/// window flags and cost one read when already set.
pub(crate) fn arm(window: &mut Window) {
    if !window.vn_picks_every_box() {
        window.set_vn_pick_every_box(true);
    }
    if !window.has_vn_hit_refiner() {
        window.set_vn_hit_refiner(Some(std::rc::Rc::new(refine)));
    }
}

/// The node a window paints as its root: the document root in the primary
/// window, the popped-out node in a pop-out.
fn window_root(t: &Tree, window: &Window) -> u32 {
    match windows::root_of(windows::key(window)) {
        0 => t.root,
        popped => popped,
    }
}

/// `id` and its ancestors up to `root`, target first — `None` when `id` is not
/// an element this window can target: gone, not an element, outside `root`'s
/// subtree, `pointer-events: none`, or inside a `visibility: hidden` subtree
/// (gpui paints none of one, so it is not there to be hit).
///
/// `pointer-events` is inherited and the cascade has already folded it, so the
/// element's own computed value is the answer; `auto` under a `none` parent
/// is hittable. The root is the exception `render::pointer_mask` makes too:
/// `body { pointer-events: none }` is how a modal layer says "nothing else",
/// and the root then stands in for the document a browser would target.
fn path_if_hittable(t: &Tree, id: u32, root: u32) -> Option<Vec<u32>> {
    let node = t.node(id)?;
    if node.kind != Kind::Element {
        return None;
    }
    let inert = node.computed.as_ref().is_some_and(|cs| cs.extras.pointer_events_none == Some(true));
    if inert && id != root {
        return None;
    }
    let mut path = Vec::with_capacity(16);
    let mut cur = id;
    for _ in 0..4096 {
        let n = t.node(cur)?;
        if n.computed.as_ref().is_some_and(|cs| cs.base.visibility == Some(gpui::Visibility::Hidden)) {
            return None;
        }
        path.push(cur);
        if cur == root {
            return Some(path);
        }
        if n.parent == 0 {
            return None;
        }
        cur = n.parent;
    }
    None
}

/// What the resolution found: the target, its path (target first, the root
/// last), and the tags under the pointer it came from, topmost first.
struct Resolved {
    target: u32,
    path: Vec<u32>,
    tags: Vec<u32>,
    /// Index in `tags` of the target; `tags.len()` when it is the fallback.
    at: usize,
}

fn resolve(t: &Tree, tags: Vec<u32>, root: u32) -> Resolved {
    for (at, &tag) in tags.iter().enumerate() {
        if let Some(path) = path_if_hittable(t, tag, root) {
            return Resolved { target: tag, path, tags, at };
        }
    }
    let at = tags.len();
    Resolved { target: root, path: vec![root], tags, at }
}

/// The topmost element at `p` that a pointer event there is for, in this
/// window. `None` when nothing but the root is there (or the tree has none).
pub(crate) fn topmost(window: &Window, p: Point<Pixels>) -> Option<u32> {
    let tags = window.vn_tags_at(p);
    tree::read_tree(|t| {
        let root = window_root(t, window);
        tags.into_iter().find(|&tag| path_if_hittable(t, tag, root).is_some())
    })
}

/// [`topmost`], or the window's root: the target of a pointer event at `p`.
pub(crate) fn target_at(window: &Window, p: Point<Pixels>) -> u32 {
    let tags = window.vn_tags_at(p);
    tree::read_tree(|t| {
        let root = window_root(t, window);
        resolve(t, tags, root).target
    })
}

/// The fork's hit refiner: run at the start of every press, release and wheel
/// in a window, before any listener.
fn refine(kind: VnHitKind, p: Point<Pixels>, window: &mut Window, _cx: &mut App) {
    let tags = window.vn_tags_at(p);
    let resolved = tree::read_tree(|t| {
        let root = window_root(t, window);
        let r = resolve(t, tags, root);
        if kind == VnHitKind::Down {
            note_covered(t, &r, p);
        }
        r
    });
    let target = resolved.target;
    windows::here(window, |w| {
        w.hit = Some(target);
        if kind == VnHitKind::Down {
            w.down_hit = Some(target);
        }
    });
    let mut path = resolved.path;
    path.sort_unstable();
    // A wheel scrolls the containers on the path that are UNDER the pointer:
    // one off to the side has nothing of the target in view to scroll.
    window.vn_restrict_mouse_hit(&|tag| path.binary_search(&tag).is_ok(), kind != VnHitKind::Scroll);
}

/// The element the mouse event being dispatched in this window is for, or
/// `fallback` outside such a dispatch (a window the refiner is not armed in).
pub(crate) fn target_or(window: &Window, fallback: u32) -> u32 {
    windows::here(window, |w| w.hit).unwrap_or(fallback)
}

/// Where a `click` goes: the nearest common inclusive ancestor of the press's
/// target and the release's (Chromium; the press and the release need not be
/// on the same element). `fallback` when either is unknown.
pub(crate) fn click_target_or(window: &Window, fallback: u32) -> u32 {
    let Some((down, up)) = windows::here(window, |w| w.down_hit.zip(w.hit)) else {
        return fallback;
    };
    tree::read_tree(|t| common_ancestor(t, down, up)).unwrap_or(fallback)
}

/// The nearest common inclusive ancestor of `a` and `b`.
fn common_ancestor(t: &Tree, a: u32, b: u32) -> Option<u32> {
    let mut seen = Vec::with_capacity(16);
    let mut cur = a;
    for _ in 0..4096 {
        let n = t.node(cur)?;
        seen.push(cur);
        if n.parent == 0 {
            break;
        }
        cur = n.parent;
    }
    let mut cur = b;
    for _ in 0..4096 {
        if seen.contains(&cur) {
            return Some(cur);
        }
        let n = t.node(cur)?;
        if n.parent == 0 {
            return None;
        }
        cur = n.parent;
    }
    None
}

thread_local! {
    /// `(cover, listener)` pairs the dev log has already named.
    static NAMED: std::cell::RefCell<std::collections::HashSet<(u32, u32)>> =
        std::cell::RefCell::new(std::collections::HashSet::new());
}

/// A press whose target COVERS an element that is not on its path — a box
/// painted over it that is not one of its ancestors. That is the browser's
/// rule and nothing to correct, but it is new here, and a layout that differs
/// from Chromium's (a transparent box laid out bigger than it should be) now
/// takes presses it used to let through. So:
///
/// * a covered element that listens for presses gets one `dev` log line per
///   (cover, listener) pair — the lead for "my button stopped working";
/// * a covered element that shows a hover (a `:hover` rung, a `group-hover`
///   anchor, a `mouseenter`/`mouseleave` listener) gets the one-time `hover:covered`
///   advisory: it stays hovered under the cover, where a browser would not (H-1).
///
/// Only the elements gpui found under the pointer are looked at, so this is a
/// few node reads per press.
fn note_covered(t: &Tree, r: &Resolved, p: Point<Pixels>) {
    let mut listener = None;
    let mut hovered = None;
    for &tag in r.tags.iter().skip(r.at + 1) {
        if r.path.contains(&tag) || (listener.is_some() && hovered.is_some()) {
            continue;
        }
        let Some(node) = t.node(tag) else { continue };
        if node.kind != Kind::Element {
            continue;
        }
        let inert = node.computed.as_ref().is_some_and(|cs| cs.extras.pointer_events_none == Some(true));
        if inert {
            continue;
        }
        if listener.is_none() && node.listeners & PRESS_BITS != 0 {
            listener = Some(tag);
        }
        let shows_hover = node.listeners & HOVER_BITS != 0
            || node.computed.as_ref().is_some_and(|cs| cs.hover.is_some() || cs.group_hover.is_some())
            || t.is_group_anchor(tag);
        if hovered.is_none() && shows_hover {
            hovered = Some(tag);
        }
    }
    if let Some(l) = listener {
        let fresh = NAMED.with(|n| {
            let mut n = n.borrow_mut();
            if n.len() > 256 {
                n.clear();
            }
            n.insert((r.target, l))
        });
        if fresh {
            crate::log::record(
                "dev",
                format!(
                    "[hit] a press at ({:.0}, {:.0}) is for {}, painted over {} — which listens for it and, \
                     as in a browser, does not get it (the cover is not one of its descendants)",
                    f32::from(p.x),
                    f32::from(p.y),
                    describe(t, r.target),
                    describe(t, l),
                ),
            );
        }
    }
    if let Some(h) = hovered {
        crate::render::warn_compat_at(
            "hover:covered",
            &format!("{}: ", describe(t, h)),
            &format!(
                "is hovered under {}, which is painted over it — a press there goes to the cover, \
                 as in a browser, but `:hover`, `:active` and `mouseenter`/`mouseleave` still follow \
                 the pointer's geometry here, so the element under it keeps its hover. Give the cover \
                 `pointer-events: none` if it should not take the pointer",
                describe(t, r.target),
            ),
        );
    }
}

/// `node 42 (<div id="x" class="a b">)`, for the log line and the advisory.
fn describe(t: &Tree, id: u32) -> String {
    let Some(node) = t.node(id) else { return format!("node {id}") };
    let mut out = format!("node {id} (<{}", t.name(node.tag));
    if let Some(el_id) = t.prop(id, "id").filter(|v| !v.is_empty()) {
        out.push_str(&format!(" id=\"{el_id}\""));
    }
    let classes: Vec<&str> = node.classes.iter().map(String::as_str).take(3).collect();
    if !classes.is_empty() {
        out.push_str(&format!(" class=\"{}\"", classes.join(" ")));
    }
    out.push_str(">)");
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tree_of(build: impl FnOnce(&mut Tree) -> Vec<u32>) -> Vec<u32> {
        tree::with_tree(build)
    }

    /// The cascade's answers, set by hand as the selection tests do:
    /// `pointer-events` already folded down the tree, `visibility` as declared.
    fn set(t: &mut Tree, id: u32, pointer_none: Option<bool>, hidden: bool) {
        let mut cs = vn_css::ComputedStates::default();
        cs.extras.pointer_events_none = pointer_none;
        if hidden {
            let mut base = gpui::StyleRefinement::default();
            base.visibility = Some(gpui::Visibility::Hidden);
            cs.base = std::rc::Rc::new(base);
        }
        t.node_mut(id).unwrap().computed = Some(cs);
    }

    #[test]
    fn the_common_ancestor_of_a_press_and_its_release() {
        let ids = tree_of(|t| {
            let root = t.create_root();
            let div = t.intern("div");
            let list = t.create_element(div);
            let a = t.create_element(div);
            let b = t.create_element(div);
            let a1 = t.create_element(div);
            t.insert(list, root, 0);
            t.insert(a, list, 0);
            t.insert(b, list, 1);
            t.insert(a1, a, 0);
            vec![root, list, a, b, a1]
        });
        let [root, list, a, b, a1] = ids[..] else { unreachable!() };
        tree::read_tree(|t| {
            assert_eq!(common_ancestor(t, a1, a1), Some(a1), "the same element");
            assert_eq!(common_ancestor(t, a1, a), Some(a), "an ancestor of the other");
            assert_eq!(common_ancestor(t, a1, b), Some(list), "two branches");
            assert_eq!(common_ancestor(t, root, b), Some(root));
        });
    }

    #[test]
    fn pointer_events_none_and_hidden_subtrees_are_not_targets() {
        let ids = tree_of(|t| {
            let root = t.create_root();
            let div = t.intern("div");
            let decor = t.create_element(div);
            let inside = t.create_element(div);
            let hidden = t.create_element(div);
            let under_hidden = t.create_element(div);
            let plain = t.create_element(div);
            t.insert(decor, root, 0);
            t.insert(inside, decor, 0);
            t.insert(hidden, root, 1);
            t.insert(under_hidden, hidden, 0);
            t.insert(plain, root, 2);
            set(t, decor, Some(true), false);
            set(t, inside, Some(false), false);
            set(t, hidden, None, true);
            set(t, under_hidden, None, false);
            vec![root, decor, inside, hidden, under_hidden, plain]
        });
        let [root, decor, inside, hidden, under_hidden, plain] = ids[..] else { unreachable!() };
        tree::read_tree(|t| {
            assert!(path_if_hittable(t, decor, root).is_none(), "pointer-events: none");
            assert_eq!(
                path_if_hittable(t, inside, root),
                Some(vec![inside, decor, root]),
                "auto under none is hit, and its path runs through the none parent"
            );
            assert!(path_if_hittable(t, hidden, root).is_none(), "visibility: hidden");
            assert!(path_if_hittable(t, under_hidden, root).is_none(), "inside a hidden subtree");
            assert_eq!(path_if_hittable(t, plain, root), Some(vec![plain, root]));
            // Topmost first: the decoration and the hidden box are passed over.
            let r = resolve(t, vec![decor, under_hidden, plain, root], root);
            assert_eq!((r.target, r.at), (plain, 2));
            let r = resolve(t, vec![decor], root);
            assert_eq!(r.target, root, "nothing hittable: the root");
            // A node outside the window's root is not this window's.
            assert!(path_if_hittable(t, plain, decor).is_none());
        });
    }
}
