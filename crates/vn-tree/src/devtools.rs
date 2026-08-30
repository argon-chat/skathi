//! The read/drive surface `vn-devtools` sits on: selector queries against the
//! REAL cascade matcher, node dumps off the retained tree, and synthesized
//! input through the ordinary dispatch path.
//!
//! It lives in vn-tree rather than in vn-app because everything it needs —
//! the node slab, the computed styles, the focus/scroll/input side maps — is
//! private to this crate. vn-app owns the transport; this owns the semantics.
//!
//! Nothing here is dev-gated: it is ~300 lines of plain reads, and vn-tree has
//! no `prod` feature to gate on. What IS gated is the only thing that matters —
//! vn-app compiles its listener, its command dispatch and this module's callers
//! out entirely under `--features prod`, so a prod build has no way to reach it.

use gpui::{App, Window};

use crate::tree::{self, Kind, NodeRef, Tree};
use crate::EventPayload;

/// One node, as the wire sees it.
#[derive(Debug, Clone)]
pub struct NodeInfo {
    pub id: u32,
    /// `element` | `text` | `anchor`.
    pub kind: &'static str,
    pub tag: String,
    pub classes: Vec<String>,
    /// Vue `<style scoped>` hashes (`data-v-...`).
    pub scopes: Vec<String>,
    /// The DOM-compat attribute channel (`value`, `placeholder`, `data-*`, …).
    pub props: Vec<(String, String)>,
    /// This node's OWN text (a text node's content, or the inline text an
    /// element carries) — not the subtree's. Use [`text_content`] for that.
    pub text: String,
    pub parent: u32,
    pub children: Vec<u32>,
    /// The `EV_*` bits JS listens for, as event names.
    pub listeners: Vec<&'static str>,
    /// `(x, y, width, height)` in CSS px, window-relative, from the last frame
    /// the node was painted in. `None` = not painted (detached, or parked in a
    /// `<KeepAlive>` cache container).
    pub bounds: Option<(f32, f32, f32, f32)>,
    /// Some descendant's `group-hover:` rule watches this element, so the
    /// renderer gave it gpui's `.group(name)` (and therefore a hitbox).
    pub group_anchor: bool,
}

/// The listener bits as names, off [`tree::EVENT_NAMES`] — the single table, so
/// this listing cannot drift behind the codes the way it did before. (It did:
/// a hand-kept copy here is how the missing `@animationstart` registration was
/// found on the compat track.)
fn event_names(mask: u64) -> Vec<&'static str> {
    tree::EVENT_NAMES
        .iter()
        .filter(|(ev, _)| mask & (1u64 << ev) != 0)
        .map(|(_, n)| *n)
        .collect()
}

/// The mounted root's node id (0 = nothing mounted yet).
pub fn root_id() -> u32 {
    tree::read_tree(|t| t.root)
}

/// Every ELEMENT node reachable from the root, in document order (pre-order —
/// the order `querySelectorAll` returns).
fn elements_in_document_order(t: &Tree) -> Vec<u32> {
    let mut out = Vec::new();
    fn walk(t: &Tree, id: u32, out: &mut Vec<u32>) {
        let Some(node) = t.node(id) else { return };
        if node.kind == Kind::Element {
            out.push(id);
        }
        for &c in &node.children {
            walk(t, c, out);
        }
    }
    if t.root != 0 {
        walk(t, t.root, &mut out);
    }
    out
}

/// `querySelectorAll`, through the cascade's own matcher.
///
/// Returns `(matched node ids in document order, advisories)`. `Err` carries
/// the SAME text the sheet compiler prints for an unsupported selector — the
/// selector audit applies here exactly as it applies to a rule, so
/// `query(":has(.x)")` explains itself instead of reporting zero matches.
pub fn query(selector: &str) -> Result<(Vec<u32>, Vec<String>), String> {
    tree::read_tree(|t| {
        let ids = elements_in_document_order(t);
        let candidates: Vec<NodeRef<'_>> = ids.iter().map(|&id| NodeRef { tree: t, id }).collect();
        let result = vn_css::query_all(selector, &candidates)?;
        for w in &result.warnings {
            crate::log::record("css", format!("[query] {w}"));
        }
        Ok((result.matched.iter().map(|&i| ids[i]).collect(), result.warnings))
    })
}

pub fn node_info(id: u32) -> Option<NodeInfo> {
    tree::read_tree(|t| {
        let n = t.node(id)?;
        Some(NodeInfo {
            id,
            kind: match n.kind {
                Kind::Element => "element",
                Kind::Text => "text",
                Kind::Anchor => "anchor",
            },
            tag: t.name(n.tag).to_string(),
            classes: n.classes.clone(),
            scopes: n.scopes.clone(),
            props: n
                .props
                .iter()
                .map(|(k, v, _)| (t.name(*k).to_string(), v.clone()))
                .collect(),
            text: n.text.clone(),
            parent: n.parent,
            children: n.children.clone(),
            listeners: event_names(n.listeners),
            bounds: crate::render::bounds_of(id),
            group_anchor: t.is_group_anchor(id),
        })
    })
}

/// `textContent` of the subtree, concatenated in document order.
pub fn text_content(id: u32) -> Option<String> {
    tree::read_tree(|t| {
        t.node(id)?;
        let mut out = String::new();
        fn walk(t: &Tree, id: u32, out: &mut String) {
            let Some(n) = t.node(id) else { return };
            // An element's own `text` is set by `setElementText`, which is
            // exclusive with children — so this never double-counts.
            out.push_str(&n.text);
            for &c in &n.children {
                walk(t, c, out);
            }
        }
        walk(t, id, &mut out);
        Some(out)
    })
}

/// The live buffer of a native `<input>` (what the widget shows right now),
/// which can be ahead of the `value` prop between an edit and Vue's echo.
pub fn input_value(id: u32, cx: &App) -> Option<String> {
    crate::render::input_value(id, cx)
}

/// Is the pointer inside this node's hitbox right now? See [`tree::is_hovered`]
/// for the one case that answers `false` while a browser would say `true`.
pub fn is_hovered(id: u32) -> bool {
    tree::is_hovered(id)
}

/// The node's folded [`vn_css::ComputedStates`], rendered as `(bucket, dump)`
/// pairs. The dump is `StyleRefinement`'s own `Debug`, minus every field that
/// is `None` — a refinement has ~90 optional fields and the two or three a rule
/// actually set are the entire answer.
///
/// The `live` bucket is the odd one out and the important one: every other
/// bucket is a rung the CASCADE produced ("this is what a hover would look
/// like"), while `live` is the style in effect on this frame ("this is what a
/// hover IS doing"). See [`tree::live_style`].
pub fn computed_styles(id: u32) -> Option<Vec<(&'static str, String)>> {
    // Hoisted out of the closure below: `live_style` reads the tree itself, and
    // a reader that only works while nothing else is reading is a trap.
    let live = tree::live_style(id);
    tree::read_tree(|t| {
        let cs = t.node(id)?.computed.as_ref()?;
        let mut out = vec![("base", prune(&format!("{:#?}", cs.base)))];
        if let Some(live) = &live {
            out.push(("live", prune(&format!("{live:#?}"))));
        }
        for (name, slot) in [
            ("hover", &cs.hover),
            ("active", &cs.active),
            ("focus", &cs.focus),
            ("focus-visible", &cs.focus_visible),
        ] {
            if let Some(r) = slot {
                out.push((name, prune(&format!("{r:#?}"))));
            }
        }
        // `.group:hover .this` — the ancestor is named, because "which element
        // am I watching" is the half of a group rule that can be wrong.
        for (name, slot) in
            [("group-hover", &cs.group_hover), ("group-active", &cs.group_active)]
        {
            if let Some((anchor, r)) = slot {
                out.push((
                    name,
                    format!("watching node {anchor}\n{}", prune(&format!("{r:#?}"))),
                ));
            }
        }
        if !cs.transitions.is_empty() {
            out.push(("transitions", format!("{:#?}", cs.transitions)));
        }
        if !cs.animations.is_empty() {
            out.push(("animations", format!("{:#?}", cs.animations)));
        }
        if cs.extras != vn_css::Extras::default() {
            out.push(("extras", format!("{:#?}", cs.extras)));
        }
        Some(out)
    })
}

/// Every rule in the installed sheet that matches this node, in cascade order —
/// the provenance behind [`computed_styles`].
///
/// `None` when the node is gone or no stylesheet is installed. An empty vec is
/// a real answer ("the sheet has nothing to say about this element").
///
/// The `declarations` on each rule are only populated when the host turned
/// `vn_css::set_capture_rule_text` on BEFORE the sheet was compiled — see
/// docs/DEVTOOLS-UI.md §"Rule provenance".
pub fn matched_rules(id: u32) -> Option<Vec<vn_css::MatchedRule>> {
    let env = crate::env();
    tree::read_tree(|t| {
        t.node(id)?;
        tree::with_engine(|engine| Some(engine?.matched_rules(NodeRef { tree: t, id }, &env)))
    })
}

/// The node's rectangle as seen from inside a BUILD — what the inspector's
/// outline overlay is positioned from. See [`crate::render::painted_bounds`]:
/// during a build the live map is empty, so this falls back to the frame that
/// was actually painted.
pub fn overlay_bounds(id: u32) -> Option<(f32, f32, f32, f32)> {
    crate::render::painted_bounds(id)
}

/// The deepest ELEMENT whose painted rectangle contains `(x, y)`, as the path
/// from the root down to it (empty when the point is over nothing).
///
/// This is the picker's hit test, and it is deliberately NOT gpui's: gpui
/// decides hits from hitboxes, which only interactive elements have, and its
/// answer cannot be read back off a frame. So this asks the prepaint bounds map
/// instead — every painted node is in it — and takes the deepest containing
/// rectangle, later siblings winning a tie because they paint on top.
///
/// The divergence that leaves: an element lifted out of document order by a
/// deferred paint (`position: fixed`, positive `z-index`) is over the elements
/// it covers on screen, but this only prefers it if it is also DEEPER or LATER.
/// Good enough to point at a node; `mouse` is still what proves a control is
/// reachable.
pub fn hit_test(x: f32, y: f32) -> Vec<u32> {
    tree::read_tree(|t| {
        fn walk(t: &Tree, id: u32, depth: usize, x: f32, y: f32, best: &mut Option<(usize, u32)>) {
            let Some(node) = t.node(id) else { return };
            if node.kind == Kind::Element {
                if let Some((bx, by, bw, bh)) = crate::render::bounds_of(id) {
                    let inside = x >= bx && x < bx + bw && y >= by && y < by + bh;
                    // `>=` on the depth: the LAST node found at the deepest
                    // level wins, and document order is paint order.
                    if inside && best.is_none_or(|(d, _)| depth >= d) {
                        *best = Some((depth, id));
                    }
                }
            }
            for &c in &node.children {
                walk(t, c, depth + 1, x, y, best);
            }
        }
        let mut best = None;
        if t.root != 0 {
            walk(t, t.root, 0, x, y, &mut best);
        }
        let Some((_, mut id)) = best else { return Vec::new() };
        let mut path = vec![id];
        for _ in 0..1024 {
            let Some(node) = t.node(id) else { break };
            if node.parent == 0 {
                break;
            }
            id = node.parent;
            path.push(id);
        }
        path.reverse();
        path
    })
}

/// Drops `None` fields (and the blocks that become empty because of it) from a
/// pretty `Debug` dump. Purely cosmetic; every value that survives is verbatim.
fn prune(dump: &str) -> String {
    let mut lines: Vec<String> = dump
        .lines()
        .filter(|l| !l.trim_end().ends_with(": None,") && l.trim() != "None,")
        .map(str::to_string)
        .collect();
    // Collapse `Foo {` immediately followed by its own `}` — repeatedly, so a
    // nest of now-empty refinements folds all the way up.
    loop {
        let mut removed = false;
        let mut i = 0;
        while i + 1 < lines.len() {
            let open = lines[i].trim_end().ends_with('{');
            let close = matches!(lines[i + 1].trim(), "}" | "}," | ")" | "),");
            if open && close {
                lines.drain(i..=i + 1);
                removed = true;
            } else {
                i += 1;
            }
        }
        if !removed {
            break;
        }
    }
    lines.join("\n")
}

// ---------------------------------------------------------------------------
// driving
// ---------------------------------------------------------------------------

/// `el.focus()`. See [`crate::render::focus_node`] — the `focus`/`blur` events
/// come from the gpui focus observers, so this reports exactly like a click.
pub fn focus(id: u32, window: &mut Window, cx: &mut App) -> Result<(), String> {
    crate::render::focus_node(id, window, cx)
}

pub fn is_focused(id: u32, window: &Window, cx: &App) -> bool {
    crate::render::is_node_focused(id, window, cx)
}

/// The node that owns keyboard focus right now, asked of GPUI rather than of
/// our own bookkeeping.
///
/// The distinction matters: `focus`/`blur` events (and therefore
/// `FOCUSED_NODE`) come from gpui's focus observers, and those report EMPTY
/// focus paths while the window is not the active one — so an inactive window
/// can be perfectly focused internally and say nothing about it. Every
/// keystroke still routes by `window.focus`, which is what this reads.
pub fn focused_node(window: &Window, cx: &App) -> Option<u32> {
    crate::render::focused_node(window, cx)
}

/// Scrolls a container by `(dx, dy)` CSS px and fires `scroll`. Returns the new
/// `(scrollLeft, scrollTop)`.
pub fn scroll(
    id: u32,
    dx: f32,
    dy: f32,
    window: &mut Window,
    cx: &mut App,
) -> Result<(f32, f32), String> {
    crate::render::scroll_node(id, dx, dy, window, cx)
}

/// A synthesized mouse sequence at `(x, y)`: `mousedown` → `mouseup` →
/// `click`, each through the ordinary dispatch path (capture/bubble over the
/// mirror tree, `preventDefault()` honored). Returns whether `click` survived
/// `preventDefault()`.
///
/// HONEST LIMITATION: this is OUR dispatch, not gpui's mouse. `:hover` and
/// `:active` ride gpui's real hitboxes and are therefore NOT exercised — a
/// state-styling test still needs a real pointer.
pub fn click(
    id: u32,
    x: f32,
    y: f32,
    button: f64,
    window: &mut Window,
    cx: &mut App,
) -> bool {
    let payload = |b: f64| EventPayload { a: x as f64, b: y as f64, c: b, ..Default::default() };
    crate::render::synth_event(id, tree::EV_MOUSEDOWN, &payload(button), window, cx);
    crate::render::synth_event(id, tree::EV_MOUSEUP, &payload(button), window, cx);
    // A browser's `click` carries button 0 whatever `MouseEvent.button` the
    // press had (a right press produces `contextmenu`, never `click`).
    let proceed = crate::render::synth_event(id, tree::EV_CLICK, &payload(0.0), window, cx);
    window.refresh();
    proceed
}
