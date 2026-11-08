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
        // Filtered by the live interaction state, as `document.querySelectorAll`.
        let live = crate::live_state::LiveStates::take(t);
        let candidates: Vec<crate::live_state::LiveRef<'_>> =
            ids.iter().map(|&id| crate::live_state::LiveRef::new(t, id, &live)).collect();
        let result = vn_css::query_all(selector, &candidates)?;
        for w in &result.warnings {
            crate::log::record("css", format!("[query] {w}"));
        }
        Ok((result.matched.iter().map(|&i| ids[i]).collect(), result.warnings))
    })
}

/// How many live nodes the retained tree holds — the one number that says
/// whether a slow frame is a big tree or a slow cascade, and the cheapest
/// thing the Performance panel can put beside its frame times.
pub fn node_count() -> u32 {
    tree::read_tree(|t| t.live_count())
}

/// `(element-tree builds, paint-only presents)` since the process started, in
/// any window. A `<video>` or moving `<canvas>` frame is a present, not a
/// build (`crate::live`); a test reads both to tell the two apart.
pub fn frame_counts() -> (u64, u64) {
    (crate::render::build_count(), crate::live::present_count())
}

pub use crate::resample::ImageStats;
pub use crate::tree::TreeMem;

/// The retained tree's memory, estimated from its live structures
/// (`Tree::mem_estimate`) — devtools `mem`.
pub fn tree_mem() -> TreeMem {
    tree::read_tree(|t| t.mem_estimate())
}

/// The painted-image registry and the downscaled-copy cache (`crate::resample`).
pub fn image_stats() -> ImageStats {
    crate::resample::image_stats()
}

pub use crate::img::{KeptFull, ResidencyStats};

/// What the image residency sweep did since boot: decodes reduced to the copy
/// they are drawn from, restored, evicted (`crate::img`, "residency").
pub fn residency_stats() -> ResidencyStats {
    crate::img::residency_stats()
}

/// Arms the painted-image registry (`VN_MEMSTATS=1`).
pub fn set_image_tracking(on: bool) {
    crate::resample::set_tracking(on);
}

/// Everything else vn-tree holds that is worth a line in `mem`, as named
/// counters: `(name, value)`. Byte counts end in `Bytes`.
pub fn side_stats() -> Vec<(&'static str, u64)> {
    let (memos, src_bytes, images, image_bytes, backgrounds) = crate::img::mem_stats();
    let (canvases, canvas_bytes) = crate::render::canvas::mem_stats();
    let (svg_docs, svg_rasters, svg_bytes) = crate::svg::mem_stats();
    let (log_entries, log_bytes) = crate::log::mem_stats();
    let (declared, waiting, waiting_bytes, registered, registered_bytes) = crate::fonts::mem_stats();
    let (boundaries, entities, cached_ids) = crate::cache::mem_stats();
    let (focus, inputs, scrolls, bounds) = crate::render::side_state_counts();
    let (stickers, sticker_tiles, sticker_tile_bytes) = crate::render::sticker::mem_stats();
    let u = |v: usize| v as u64;
    vec![
        ("imgMemos", u(memos)),
        ("imgSrcBytes", u(src_bytes)),
        ("imgEncodedInMemory", u(images)),
        ("imgEncodedInMemoryBytes", u(image_bytes)),
        ("cssBackgroundLayers", u(backgrounds)),
        ("canvases", u(canvases)),
        ("canvasHostCopyBytes", u(canvas_bytes)),
        ("svgDocs", u(svg_docs)),
        ("svgColourRasters", u(svg_rasters)),
        ("svgColourRasterBytes", u(svg_bytes)),
        ("videos", u(crate::video::count())),
        ("videosAudible", u(crate::video::audible_count())),
        ("lotties", u(crate::lottie::count())),
        ("stickers", u(stickers)),
        ("stickerTiles", u(sticker_tiles)),
        ("stickerTileBytes", u(sticker_tile_bytes)),
        ("logRingEntries", u(log_entries)),
        ("logRingBytes", u(log_bytes)),
        ("fontFacesDeclared", u(declared)),
        ("fontFacesLoadedNotRegistered", u(waiting)),
        ("fontFacesLoadedNotRegisteredBytes", u(waiting_bytes)),
        ("fontFacesRegistered", registered),
        ("fontFacesRegisteredBytes", registered_bytes),
        ("cacheBoundaries", u(boundaries)),
        ("cacheBoundaryEntities", u(entities)),
        ("cacheBoundaryNodeIds", u(cached_ids)),
        ("focusHandles", u(focus)),
        ("inputStates", u(inputs)),
        ("scrollHandles", u(scrolls)),
        ("boundsEntries", u(bounds)),
    ]
}

/// How many scroll containers registered with the wheel router in the frame
/// just painted.
///
/// A one-number window onto the one thing a retained-render replay cannot
/// rebuild for itself. `vn_elements::scrollbar`'s registry is filled by the
/// PAINT walk, and a replayed subtree does not walk — so before m23 this
/// number silently dropped every container inside a cache boundary, which is
/// why a boundary refused to cover one at all. `crate::cache::Boundary` now
/// re-registers the span it recorded, and this is how a spec asserts that from
/// outside the process: cache on or off, the count is the same.
pub fn scroll_chain_count() -> u32 {
    vn_elements::scrollbar::chain_count() as u32
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

/// A text field's IME, as the platform drives the field's input handler:
/// `text` becomes the composition's pre-edit (`""` cancels it), or with
/// `commit` its committed text. The devtools `ime` command; a test composes
/// with it where no IME is installed.
pub fn ime(id: u32, text: &str, commit: bool, window: &mut Window, cx: &mut App) -> Result<(), String> {
    crate::render::ime(id, text, commit, window, cx)
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

/// The deepest ELEMENT gpui would hit at `(x, y)`, as the path from the root
/// down to it (empty when the point is over nothing).
///
/// **This IS gpui's hit test.** It used to be ours: a walk over the prepaint
/// bounds map taking the deepest containing rectangle, because gpui decides
/// hits from hitboxes, only interactive elements have one, and there was no way
/// to ask what the frame would hit at an arbitrary point without moving the
/// cursor. Both halves of that are gone. gpui now gives every element a hitbox
/// tagged with its node id while a dev host is running (`render::pick_hitbox`),
/// and upstream's `Hitbox::is_hovered_at` answers the question against the
/// RENDERED frame — so the two divergences the hand-rolled version documented are closed
/// by construction:
///
/// * **transforms.** A rotated or scaled element keeps its untransformed
///   `bounds`; gpui hit-tests by mapping the pointer BACKWARDS through the
///   hitbox's matrix, which is exact for rotation and scale alike and correctly
///   answers "not hittable" for `scale(0)`. Comparing rectangles could only
///   ever be right for a translation.
/// * **paint order.** An element lifted out of document order by a deferred
///   paint (`position: fixed`, a `z-index`) is hit-tested in the order it
///   paints (fork item 37), so it is in FRONT — which is what the picker has
///   to say, and what "deeper or later in document order" only sometimes
///   agreed with.
///
/// Content masks come along too: a node scrolled out of its clipping container
/// is no longer hit, exactly as in a browser.
///
/// What is left is the tie-break between the several hitboxes a point is inside
/// (a node and all of its ancestors): the DEEPEST wins, and among equals the
/// last in document order, which is paint order. Across two overlays over one
/// point that is not paint order: a node nested deeper in the one painted
/// under can win the pick (docs/DEVTOOLS.md).
pub fn hit_test(x: f32, y: f32, window: &Window) -> Vec<u32> {
    hit_test_in(0, x, y, window)
}

/// [`hit_test`], restricted to the window that paints `root`.
///
/// `root` is the popped-out node a second window paints (`0` = the primary
/// window). Hitboxes live in the frame of the window that painted them, so a
/// point is only answerable once a window is named: without this, hovering a
/// pop-out at (40, 40) could pick the primary window's node at (40, 40), and
/// the picker would confidently outline the wrong thing in the wrong window
/// (docs/MULTIWINDOW.md §5.6).
///
/// The PATH is still root-first from the document root, not from `root` — the
/// tree panel expands ancestors, and a node's ancestry does not stop at the
/// window boundary. Only the search does.
pub fn hit_test_in(root: u32, x: f32, y: f32, window: &Window) -> Vec<u32> {
    let point = gpui::Point::new(gpui::px(x), gpui::px(y));
    tree::read_tree(|t| {
        fn walk(
            t: &Tree,
            window_root: u32,
            id: u32,
            depth: usize,
            point: gpui::Point<gpui::Pixels>,
            window: &Window,
            best: &mut Option<(usize, u32)>,
        ) {
            let Some(node) = t.node(id) else { return };
            if node.kind == Kind::Element
                && crate::render::pick_hitbox(window, id)
                    .is_some_and(|h| h.is_hovered_at(point, window))
                // `>=` on the depth: the LAST node found at the deepest
                // level wins, and document order is paint order.
                && best.is_none_or(|(d, _)| depth >= d)
            {
                *best = Some((depth, id));
            }
            for &c in &node.children {
                // A popped-out subtree is painted by a DIFFERENT window; its
                // hitboxes are not in this window's frame, so descending into
                // it can only waste time.
                if window_root == 0 && crate::render::popped_out(c) {
                    continue;
                }
                walk(t, window_root, c, depth + 1, point, window, best);
            }
        }
        let mut best = None;
        let start = if root == 0 { t.root } else { root };
        if start != 0 && t.node(start).is_some() {
            walk(t, root, start, 0, point, window, &mut best);
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

/// The window point to aim a real pointer gesture at, for a node addressed by
/// SELECTOR rather than by coordinate — the centre of the box as it was
/// actually painted.
///
/// A node's recorded `bounds` are the UNTRANSFORMED rectangle (that is the
/// frame layout, `getBoundingClientRect` and the outline overlay all think in),
/// so on a `rotate()`d or `translate()`d element its centre is not where the
/// element is on screen. Mapping that centre FORWARD through the same matrix
/// gpui hit-tests backwards through puts the point back on the element — and
/// the result is then confirmed against gpui's own hit test, so a caller is
/// told when the box is unreachable (covered by an overlay, clipped out of its
/// scroll container, `scale(0)`) instead of clicking a point that belongs to
/// something else.
///
/// `None` = the node was not painted in the last frame BY THIS WINDOW, or the
/// point that would be aimed at is not the node's. Both halves are asked of
/// `window` and only of `window`: the rectangle comes out of that window's own
/// bounds map (`render::bounds_here`) and the hitbox out of that window's own
/// frame, so a node painted by a pop-out answers `None` to the primary window
/// rather than handing back a coordinate from the other window's client space
/// (docs/MULTIWINDOW.md §5.6).
pub fn aim_point(id: u32, window: &Window) -> Option<(f32, f32)> {
    let (x, y, w, h) = crate::render::bounds_here(window, id)?;
    let centre = gpui::Point::new(gpui::px(x + w / 2.0), gpui::px(y + h / 2.0));
    let hitbox = crate::render::pick_hitbox(window, id)?;
    let scale = window.scale_factor();
    let point = if hitbox.transformation.is_unit() {
        centre
    } else {
        let d = hitbox.transformation.apply(gpui::Point::new(
            gpui::px(f32::from(centre.x) * scale),
            gpui::px(f32::from(centre.y) * scale),
        ));
        gpui::Point::new(gpui::px(f32::from(d.x) / scale), gpui::px(f32::from(d.y) / scale))
    };
    hitbox
        .is_hovered_at(point, window)
        .then(|| (f32::from(point.x), f32::from(point.y)))
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

#[cfg(test)]
mod tests {
    use super::*;

    /// `tag: 0` means "this node has no tag" — `create_text` and
    /// `create_anchor` both set it. It must NOT resolve to whatever name the
    /// interner happened to hand out first.
    ///
    /// It did: the interner started empty, `create_root` interned `"root"`
    /// into slot 0, and from then on every text node and every Vue fragment
    /// anchor reported `tag: "root"` on the wire — which the inspector's
    /// Elements tree drew as runs of `<root>` rows between the real elements.
    #[test]
    fn text_and_anchor_nodes_have_no_tag_name() {
        let (root, div, text, anchor) = tree::with_tree(|t| {
            let root = t.create_root();
            let div_tag = t.intern("div");
            let div = t.create_element(div_tag);
            let text = t.create_text();
            let anchor = t.create_anchor();
            t.insert(div, root, 0);
            t.insert(text, div, 0);
            t.insert(anchor, div, 0);
            (root, div, text, anchor)
        });

        let info = |id: u32| node_info(id).expect("node exists");
        // The name table's slot 0 is reserved, so no real name can land there.
        assert_eq!(tree::read_tree(|t| t.name(0).to_string()), "");

        assert_eq!(info(root).kind, "element");
        assert_eq!(info(root).tag, "root");
        assert_eq!(info(div).kind, "element");
        assert_eq!(info(div).tag, "div");

        assert_eq!(info(text).kind, "text");
        assert_eq!(info(text).tag, "", "a text node has no tag name");
        assert_eq!(info(anchor).kind, "anchor");
        assert_eq!(info(anchor).tag, "", "a fragment anchor has no tag name");
    }
}

/// Native canvas atlas copies and exceptional CPU fallbacks, for the inspector.
#[cfg(feature = "webgpu")]
pub fn gpu_canvas_presentation_stats() -> (u64,u64) { crate::render::canvas::native_stats() }
