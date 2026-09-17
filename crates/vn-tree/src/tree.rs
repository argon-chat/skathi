//! The retained tree: a slab of nodes keyed by dense `u32` ids (0 = none),
//! mutated only from the VM thread via the `__vn_*` host functions.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;

// Event codes — the JS side (packages/vue-native/src/runtime.js EV_CODES)
// must stay in lockstep.
pub const EV_CLICK: u32 = 1;
pub const EV_MOUSEDOWN: u32 = 2;
pub const EV_MOUSEUP: u32 = 3;
pub const EV_WHEEL: u32 = 4;
pub const EV_KEYDOWN: u32 = 5;
pub const EV_KEYUP: u32 = 6;
pub const EV_FOCUS: u32 = 7;
pub const EV_BLUR: u32 = 8;
pub const EV_INPUT: u32 = 9;
pub const EV_SCROLL: u32 = 10;
// `<lottie>` playback events, named after lottie-web's so a component ports
// from the browser unchanged (crates/vn-lottie/CONTRACT.md §2).
pub const EV_COMPLETE: u32 = 11;
pub const EV_LOOPCOMPLETE: u32 = 12;
pub const EV_ENTERFRAME: u32 = 13;
// The pointer family. Only the three the host can actually SOURCE have codes:
// gpui gives us a move-while-hovered listener and an enter/leave pair, and
// everything else in the W3C pointer family is derived from these (and from
// mousedown/mouseup) inside the JS runtime — `pointerdown` is `mousedown`,
// `pointerenter` is `mouseenter`, and so on. Adding a code per alias would put
// the same gpui listener on the element several times over.
pub const EV_MOUSEMOVE: u32 = 14;
/// Non-bubbling, per the UI Events spec: fires only on the element entered.
pub const EV_MOUSEENTER: u32 = 15;
pub const EV_MOUSELEAVE: u32 = 16;
// `<video>` — the HTML media event set (crates/vn-video/CONTRACT.md §4). Every
// one of these is also an `MF_MEDIA_ENGINE_EVENT` below 1000, which Microsoft
// documents as the HTML 5 media events, so the names are the spec's on both
// sides of the boundary and a component ports from the browser unchanged.
//
// This is the block the `u32 -> u64` widening existed for: it takes the highest
// used code from 16 to 34.
pub const EV_LOADSTART: u32 = 17;
pub const EV_LOADEDMETADATA: u32 = 18;
pub const EV_LOADEDDATA: u32 = 19;
pub const EV_CANPLAY: u32 = 20;
pub const EV_CANPLAYTHROUGH: u32 = 21;
pub const EV_PLAY: u32 = 22;
pub const EV_PLAYING: u32 = 23;
pub const EV_PAUSE: u32 = 24;
pub const EV_SEEKING: u32 = 25;
pub const EV_SEEKED: u32 = 26;
/// Throttled to the spec's "no more often than every 250 ms" — see
/// `vn_video::VideoState`. High frequency by media standards, so like
/// `enterFrame` it is gated on the listener bit before the payload is built.
pub const EV_TIMEUPDATE: u32 = 27;
pub const EV_DURATIONCHANGE: u32 = 28;
pub const EV_RATECHANGE: u32 = 29;
pub const EV_VOLUMECHANGE: u32 = 30;
pub const EV_WAITING: u32 = 31;
pub const EV_ENDED: u32 = 32;
/// Carries `MediaError.code` in `a` and the advisory text in `key`.
pub const EV_ERROR: u32 = 33;
pub const EV_EMPTIED: u32 = 34;

// The CSS animation/transition events, sourced by vn-css's animator rather than
// by gpui. `EventPayload::a` carries `elapsedTime` (seconds) and `key` carries
// `propertyName` / `animationName`.
//
// These four were 17-20 on the compat track and were RENUMBERED to 35-38 at the
// m12 integration merge, because the `<video>` track had independently taken
// 17-34 for the media block. `Node::listeners` is a `u64`, so the space is
// there; bits 39-63 remain free.
pub const EV_TRANSITIONEND: u32 = 35;
pub const EV_ANIMATIONSTART: u32 = 36;
pub const EV_ANIMATIONEND: u32 = 37;
pub const EV_ANIMATIONCANCEL: u32 = 38;

/// Every host event code with the name JS knows it by — the ONE table, so a
/// listing (devtools) can never lag the codes again. It did: `event_names` in
/// `devtools.rs` stopped at code 10 and never learned the three `<lottie>`
/// events, which made a `@complete` handler invisible to `vn_tree`.
///
/// Derived names (`pointerdown` from `mousedown`, `focusin` from `focus`) are
/// deliberately absent: they have no code of their own — the JS runtime
/// synthesises them from the source event.
pub const EVENT_NAMES: &[(u32, &str)] = &[
    (EV_CLICK, "click"),
    (EV_MOUSEDOWN, "mousedown"),
    (EV_MOUSEUP, "mouseup"),
    (EV_WHEEL, "wheel"),
    (EV_KEYDOWN, "keydown"),
    (EV_KEYUP, "keyup"),
    (EV_FOCUS, "focus"),
    (EV_BLUR, "blur"),
    (EV_INPUT, "input"),
    (EV_SCROLL, "scroll"),
    (EV_COMPLETE, "complete"),
    (EV_LOOPCOMPLETE, "loopComplete"),
    (EV_ENTERFRAME, "enterFrame"),
    (EV_MOUSEMOVE, "mousemove"),
    (EV_MOUSEENTER, "mouseenter"),
    (EV_MOUSELEAVE, "mouseleave"),
    (EV_LOADSTART, "loadstart"),
    (EV_LOADEDMETADATA, "loadedmetadata"),
    (EV_LOADEDDATA, "loadeddata"),
    (EV_CANPLAY, "canplay"),
    (EV_CANPLAYTHROUGH, "canplaythrough"),
    (EV_PLAY, "play"),
    (EV_PLAYING, "playing"),
    (EV_PAUSE, "pause"),
    (EV_SEEKING, "seeking"),
    (EV_SEEKED, "seeked"),
    (EV_TIMEUPDATE, "timeupdate"),
    (EV_DURATIONCHANGE, "durationchange"),
    (EV_RATECHANGE, "ratechange"),
    (EV_VOLUMECHANGE, "volumechange"),
    (EV_WAITING, "waiting"),
    (EV_ENDED, "ended"),
    (EV_ERROR, "error"),
    (EV_EMPTIED, "emptied"),
    (EV_TRANSITIONEND, "transitionend"),
    (EV_ANIMATIONSTART, "animationstart"),
    (EV_ANIMATIONEND, "animationend"),
    (EV_ANIMATIONCANCEL, "animationcancel"),
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Element,
    Text,
    /// Zero-size structural marker (fragment/Teleport anchors, comments).
    Anchor,
}

pub struct Node {
    pub kind: Kind,
    pub tag: u32,
    pub parent: u32,
    pub children: Vec<u32>,
    /// Text content: node text for `Kind::Text`, inline text (exclusive with
    /// children) for `Kind::Element` via `setElementText`.
    pub text: String,
    /// (interned kebab-case property, raw value) pairs, last write wins —
    /// the inline-style transport; parsed lazily into `inline_cache`.
    pub style: Vec<(u32, String)>,
    pub classes: Vec<String>,
    /// Generic element properties/attributes (interned name, string value,
    /// write counter) — the DOM-compat channel (`value`, `placeholder`,
    /// `disabled`, ...). Native widgets (text input etc.) read what they
    /// understand.
    ///
    /// The counter ticks on *every* assignment, including one that stores the
    /// same string again. In the DOM `el.value = x` always assigns; a native
    /// widget that dedupes on the string alone cannot tell a controlled input
    /// being re-forced to a value the user has since typed over from a prop
    /// nobody touched. See [`Tree::prop_write`].
    pub props: Vec<(u32, String, u32)>,
    /// Vue `<style scoped>` hashes (`data-v-...`); a child component's root
    /// carries two — its own and its parent's.
    pub scopes: Vec<String>,
    /// Bitmask of EV_* the JS side listens for.
    ///
    /// **`u64`, not `u32`.** The transport is already per-code
    /// (`__vn_set_listener(id, code, on)`), so the width is purely an internal
    /// storage decision — but it is a decision with a ceiling, and the `<video>`
    /// element alone needs 13 codes for its v1 event set and 22 for the full
    /// HTML media set. At 16 codes used the old `u32` left no room for that, for
    /// `<audio>`, or for the `<img>` `@load`/`@error` pair the m10 images report
    /// flagged. Widened as a standalone change so it is never a merge conflict
    /// with the element work that needs it (research/reports/m11/video.md §9.4).
    pub listeners: u64,
    pub(crate) inline_dirty: bool,
    /// Everything the inline `style` body contributes: the mapped refinement,
    /// its transitions and `object-fit`, plus the `--x` declarations and
    /// `var()`-composed values that can only be resolved once the node's place
    /// in the tree is known.
    pub(crate) inline: Option<std::rc::Rc<vn_css::InlineStyle>>,
    /// The folded cascade result, filled by the restyle pass.
    pub computed: Option<vn_css::ComputedStates>,
}

/// Above this many separately-marked nodes, scoping stops paying: the
/// bookkeeping (ancestor checks, per-root walks, repeated visits where two
/// marks overlap) costs more than one straight pass over the document. A Vue
/// wave that touches a quarter of the tree is a page change, not a hover.
const SCOPED_RESTYLE_LIMIT: usize = 64;

#[derive(Default)]
pub struct Tree {
    nodes: Vec<Option<Node>>,
    free: Vec<u32>,
    pub root: u32,
    dirty: bool,
    /// Nodes whose own cascade inputs changed since the last restyle — the
    /// roots of the subtrees the next pass has to recompute. See
    /// [`Tree::mark_restyle`].
    dirty_nodes: Vec<u32>,
    /// "Everything": a new stylesheet or a changed `@media` environment can
    /// flip any rule in the sheet, so no subtree is safe.
    dirty_all: bool,
    /// Nodes whose inline `style` text changed and must be re-parsed. Tracked
    /// rather than rediscovered, so the pass never scans the whole slab.
    inline_dirty_nodes: Vec<u32>,
    /// A FORCED style flush (`getComputedStyle`, see
    /// [`flush_pending_style`]) already ran this wave's restyle. The wave's
    /// `flush_changes` must still report a change so the root view repaints,
    /// but it must not run the cascade a second time — and neither must the
    /// next `getComputedStyle` in the same wave.
    style_flushed: bool,
    /// Every `<style>` element in the tree, by node id.
    ///
    /// Kept as a set rather than rediscovered, because [`sync_runtime_sheets`]
    /// runs on every flush that dirties the tree and a scan of the whole slab
    /// per flush is exactly the cost m11 spent a milestone removing. Almost
    /// every app leaves it empty and pays one `is_empty()`.
    style_nodes: std::collections::BTreeSet<u32>,
    /// Elements some OTHER element's `group-hover:` rule watches — the ones the
    /// renderer has to give gpui's `.group(name)` to. Recomputed at the end of
    /// a restyle pass, and only for a sheet that has such a rule at all: a
    /// scoped restyle can recompute the DESCENDANT without touching the
    /// ancestor, so an incremental set would go stale in exactly the case the
    /// scoped pass exists for.
    group_anchors: std::collections::HashSet<u32>,
    /// Anchor → the elements whose `group-hover:` rung has to be ANIMATED when
    /// that anchor's pointer state flips (i.e. the ones that also declare a
    /// `transition`). gpui resolves the rung itself at paint time and needs no
    /// list; the animator needs one, because a flip on the anchor is the only
    /// signal a descendant ever gets. Built in the same pass as
    /// [`Tree::group_anchors`] and, like it, only for a sheet that uses the
    /// variant at all — so the m11 invariant holds: a hover walks this list,
    /// never the tree.
    group_hover_deps: HashMap<u32, Vec<u32>>,
    names: Vec<String>,
    name_ids: HashMap<String, u32>,
}

thread_local! {
    static TREE: RefCell<Tree> = RefCell::new(Tree::new());
    static DISPATCHER: Cell<u64> = const { Cell::new(0) };
}

/// Runs `f` with the tree. Panics (→ caught by `host_fn!`, poisons) if the
/// tree is already borrowed — i.e. if JS mutates during a draw walk (I6/I8).
pub fn with_tree<R>(f: impl FnOnce(&mut Tree) -> R) -> R {
    TREE.with(|t| f(&mut t.borrow_mut()))
}

/// Read-only access for the render walk.
pub fn read_tree<R>(f: impl FnOnce(&Tree) -> R) -> R {
    TREE.with(|t| f(&t.borrow()))
}

/// One line per distinct CSS advisory, however many elements hit it — and
/// however many times the sheet is recompiled, since a recompile of the same
/// source produces the same diagnostics (see [`set_stylesheet`]).
fn warn_css(msg: &str) {
    // Normalised BEFORE the dedupe, so two spellings of the same message are
    // one line and not two (see [`crate::log::one_line`]).
    let msg = crate::log::one_line(msg);
    let fresh = CSS_WARNED.with(|w| w.borrow_mut().insert(msg.clone()));
    if fresh {
        eprintln!("{msg}");
        crate::log::record("css", msg);
    }
}

pub(crate) fn set_dispatcher(f: u64) {
    DISPATCHER.with(|d| d.set(f));
}

pub(crate) fn dispatcher() -> u64 {
    DISPATCHER.with(|d| d.get())
}

impl Tree {
    fn new() -> Self {
        Tree {
            // Slot 0 is the reserved "none" id.
            nodes: vec![None],
            // NAME 0 is reserved the same way, and for a bug that shipped:
            // `create_text` and `create_anchor` give their nodes `tag: 0`
            // meaning "no tag", but the interner used to hand id 0 to the
            // FIRST name anybody interned — which is `create_root`'s "root".
            // So every text node and every Vue fragment anchor reported
            // `tag: "root"` to the devtools, and the inspector's Elements tree
            // drew runs of `<root>` rows between the real elements. Reserving
            // the empty name makes `name(0)` mean what `tag: 0` says.
            names: vec![String::new()],
            name_ids: HashMap::from([(String::new(), 0)]),
            ..Default::default()
        }
    }

    pub fn intern(&mut self, name: &str) -> u32 {
        if let Some(&id) = self.name_ids.get(name) {
            return id;
        }
        let id = self.names.len() as u32;
        self.names.push(name.to_string());
        self.name_ids.insert(name.to_string(), id);
        id
    }

    pub fn name(&self, id: u32) -> &str {
        self.names.get(id as usize).map(String::as_str).unwrap_or("")
    }

    /// Does some descendant's `group-hover:` rule watch this element? See
    /// [`Tree::group_anchors`].
    pub fn is_group_anchor(&self, id: u32) -> bool {
        self.group_anchors.contains(&id)
    }

    /// …and does at least one of those descendants need the ANIMATOR told when
    /// this element's pointer state flips? See [`Tree::group_hover_deps`].
    pub fn has_group_hover_deps(&self, id: u32) -> bool {
        self.group_hover_deps.contains_key(&id)
    }

    fn alloc(&mut self, node: Node) -> u32 {
        if let Some(id) = self.free.pop() {
            self.nodes[id as usize] = Some(node);
            id
        } else {
            self.nodes.push(Some(node));
            (self.nodes.len() - 1) as u32
        }
    }

    pub fn node(&self, id: u32) -> Option<&Node> {
        self.nodes.get(id as usize).and_then(Option::as_ref)
    }

    pub(crate) fn node_mut(&mut self, id: u32) -> Option<&mut Node> {
        self.nodes.get_mut(id as usize).and_then(Option::as_mut)
    }

    fn fresh(&mut self, kind: Kind, tag: u32) -> u32 {
        self.dirty = true;
        let id = self.alloc(Node {
            kind,
            tag,
            parent: 0,
            children: Vec::new(),
            text: String::new(),
            style: Vec::new(),
            classes: Vec::new(),
            props: Vec::new(),
            scopes: Vec::new(),
            listeners: 0,
            inline_dirty: false,
            inline: None,
            computed: None,
        });
        self.mark_restyle(id);
        id
    }

    pub fn create_element(&mut self, tag: u32) -> u32 {
        let id = self.fresh(Kind::Element, tag);
        // A `<style>` carries CSS, not a box (`sync_runtime_sheets`).
        if self.name(tag).eq_ignore_ascii_case("style") {
            self.style_nodes.insert(id);
        }
        id
    }

    pub fn create_text(&mut self) -> u32 {
        self.fresh(Kind::Text, 0)
    }

    pub fn create_anchor(&mut self) -> u32 {
        self.fresh(Kind::Anchor, 0)
    }

    pub fn create_root(&mut self) -> u32 {
        let tag = self.intern("root");
        let id = self.fresh(Kind::Element, tag);
        self.root = id;
        id
    }

    pub fn set_text(&mut self, id: u32, text: String) {
        if let Some(n) = self.node_mut(id) {
            n.text = text;
            self.dirty = true;
            self.mark_restyle(id);
        }
    }

    /// "This element's entire child content is now exactly this string":
    /// destroy remaining children, set inline text.
    pub fn set_element_text(&mut self, id: u32, text: String) {
        let children = match self.node_mut(id) {
            Some(n) => std::mem::take(&mut n.children),
            None => return,
        };
        for c in children {
            self.free_subtree(c);
        }
        if let Some(n) = self.node_mut(id) {
            n.text = text;
        }
        self.dirty = true;
        self.mark_restyle(id);
    }

    /// Insert-or-move `child` under `parent`, before `anchor` (0 = append).
    pub fn insert(&mut self, child: u32, parent: u32, anchor: u32) {
        if child == 0 || parent == 0 || child == anchor {
            return;
        }
        // Both ends: the old parent loses a child, the new one gains one, and
        // marking a parent covers every sibling AND the moved subtree itself.
        let old_parent = self.node(child).map(|n| n.parent).unwrap_or(0);
        if old_parent != 0 {
            self.mark_restyle(old_parent);
        }
        self.mark_restyle(parent);
        self.detach(child);
        // An element with inline text gaining a real child: the inline text
        // is stale Vue state (setElementText always precedes) — drop it.
        let idx = {
            let Some(p) = self.node_mut(parent) else { return };
            p.text.clear();
            match p.children.iter().position(|&c| c == anchor) {
                Some(i) => i,
                None => p.children.len(),
            }
        };
        if let Some(p) = self.node_mut(parent) {
            p.children.insert(idx, child);
        }
        if let Some(c) = self.node_mut(child) {
            c.parent = parent;
        }
        self.dirty = true;
    }

    fn detach(&mut self, id: u32) {
        let parent = self.node(id).map(|n| n.parent).unwrap_or(0);
        if parent != 0 {
            if let Some(p) = self.node_mut(parent) {
                p.children.retain(|&c| c != id);
            }
            if let Some(n) = self.node_mut(id) {
                n.parent = 0;
            }
        }
    }

    /// Vue calls remove exactly once per unmounted subtree root: recursively
    /// destroy native state and recycle every id.
    pub fn remove(&mut self, id: u32) {
        let parent = self.node(id).map(|n| n.parent).unwrap_or(0);
        if parent != 0 {
            self.mark_restyle(parent);
        }
        self.detach(id);
        self.free_subtree(id);
        self.dirty = true;
    }

    fn free_subtree(&mut self, id: u32) {
        let Some(node) = self.nodes.get_mut(id as usize).and_then(Option::take) else {
            return;
        };
        self.style_nodes.remove(&id);
        for c in node.children {
            self.free_subtree(c);
        }
        if self.root == id {
            self.root = 0;
        }
        self.free.push(id);
    }

    pub fn set_style(&mut self, id: u32, key: u32, value: String) {
        if let Some(n) = self.node_mut(id) {
            if value.is_empty() {
                n.style.retain(|(k, _)| *k != key);
            } else if let Some(slot) = n.style.iter_mut().find(|(k, _)| *k == key) {
                slot.1 = value;
            } else {
                n.style.push((key, value));
            }
            n.inline_dirty = true;
            self.dirty = true;
            self.inline_dirty_nodes.push(id);
            self.mark_restyle(id);
        }
    }

    pub fn set_prop(&mut self, id: u32, key: u32, value: Option<String>) {
        if let Some(n) = self.node_mut(id) {
            match value {
                None => n.props.retain(|(k, _, _)| *k != key),
                Some(v) => {
                    if let Some(slot) = n.props.iter_mut().find(|(k, _, _)| *k == key) {
                        slot.1 = v;
                        slot.2 = slot.2.wrapping_add(1);
                    } else {
                        n.props.push((key, v, 0));
                    }
                }
            }
            self.dirty = true;
            self.mark_restyle(id);
        }
    }

    pub fn prop<'a>(&'a self, id: u32, name: &str) -> Option<&'a str> {
        self.prop_write(id, name).map(|(v, _)| v)
    }

    /// The property value together with its write counter — see [`Node::props`].
    /// Consumers that must react to *assignments* rather than to value changes
    /// (a controlled `<input>`) key off the counter.
    pub fn prop_write<'a>(&'a self, id: u32, name: &str) -> Option<(&'a str, u32)> {
        let key = *self.name_ids.get(name)?;
        self.node(id)?
            .props
            .iter()
            .find(|(k, _, _)| *k == key)
            .map(|(_, v, write)| (v.as_str(), *write))
    }

    pub fn add_scope(&mut self, id: u32, scope: String) {
        if let Some(n) = self.node_mut(id) {
            if !n.scopes.contains(&scope) {
                n.scopes.push(scope);
                self.dirty = true;
                self.mark_restyle(id);
            }
        }
    }

    pub fn set_class(&mut self, id: u32, classes: &str) {
        if let Some(n) = self.node_mut(id) {
            n.classes = classes.split_whitespace().map(str::to_string).collect();
            self.dirty = true;
            self.mark_restyle(id);
        }
    }

    pub fn set_listener(&mut self, id: u32, event: u32, on: bool) {
        if let Some(n) = self.node_mut(id) {
            // `event` is a code, not a bit index the caller can be trusted with:
            // a JS mirror that has drifted ahead of this build would otherwise
            // shift past the width and panic in debug / wrap in release.
            if event >= u64::BITS {
                return;
            }
            let bit = 1u64 << event;
            if on {
                n.listeners |= bit;
            } else {
                n.listeners &= !bit;
            }
            self.dirty = true;
        }
    }

    /// [`take_dirty`](Self::take_dirty) without taking it: is there tree state
    /// the host has not folded and painted yet? Read by the synchronous-layout
    /// path, which must not consume the flag the frame is going to act on.
    pub fn is_dirty(&self) -> bool {
        self.dirty || self.style_flushed
    }

    pub fn take_dirty(&mut self) -> bool {
        // A forced flush already did the cascade, but the frame still has to be
        // told something changed, or the wave paints the old pixels.
        let flushed = std::mem::take(&mut self.style_flushed);
        std::mem::take(&mut self.dirty) || flushed
    }

    /// "Something changed that no subtree is safe from": a new stylesheet, or a
    /// changed `@media` environment. Both can flip any rule in the sheet.
    /// Live nodes — allocated slots that still hold a node. `nodes.len()`
    /// counts freed slots too, which would make a tree that churned once
    /// look permanently large.
    pub fn live_count(&self) -> u32 {
        self.nodes.iter().filter(|n| n.is_some()).count() as u32
    }

    /// Marks one subtree for a re-cascade — the scoped half of [`mark_dirty`].
    /// A pop-out resizing changes the `@media` answers for the subtree IT
    /// paints and for nothing else (docs/MULTIWINDOW.md §6.3).
    pub fn mark_restyle_subtree(&mut self, id: u32) {
        self.dirty = true;
        self.mark_restyle(id);
    }

    pub fn mark_dirty(&mut self) {
        self.dirty = true;
        self.dirty_all = true;
    }

    /// "This element's own cascade inputs changed" — its classes, attributes,
    /// scope hashes, inline style, text, or child list.
    ///
    /// The mark is a SUBTREE root, not a node: a descendant combinator lets any
    /// rule read every ancestor, so `.card .title` starts matching the moment
    /// `card` appears three levels up. Descendants are therefore always
    /// recomputed; ancestors never are (nothing in the supported selector set
    /// reads downwards — `:has()` is rejected at sheet-compile time, and the
    /// structural pseudos that read the child list are handled by marking the
    /// PARENT wherever the child list changes). Siblings are covered only when
    /// the sheet actually contains `+` / `~`; see [`Tree::restyle_roots`].
    fn mark_restyle(&mut self, id: u32) {
        if self.dirty_all || id == 0 {
            return;
        }
        if self.dirty_nodes.len() >= SCOPED_RESTYLE_LIMIT {
            // Past the limit the list stops being worth keeping — one straight
            // document pass is cheaper than the bookkeeping.
            self.dirty_nodes.clear();
            self.dirty_all = true;
            return;
        }
        if !self.dirty_nodes.contains(&id) {
            self.dirty_nodes.push(id);
        }
    }

    /// The restyle pass: re-parse dirty inline styles, then run the cascade
    /// for every element node. Runs once per JS wave that touched the tree —
    /// never during a draw.
    pub fn restyle(&mut self, engine: Option<&vn_css::Engine>, env: &vn_css::Env) {
        match engine {
            Some(e) => self.restyle_with(e, env),
            // Before the first `__vn_set_stylesheet` there is no sheet, but the
            // cascade still has work to do (inline styles, and the custom
            // properties an inline `style` declares), so it runs against an
            // empty one rather than a second, divergent code path.
            None => NO_SHEET.with(|e| self.restyle_with(e, env)),
        }
    }

    /// The subtree roots this pass has to walk, in an order where an ancestor
    /// is always computed before anything under it.
    ///
    /// `dirty_all` (a new sheet, a changed `@media` environment, or simply too
    /// many separate marks) answers "the whole document": the mounted root plus
    /// every parentless node, since a `<KeepAlive>` cache container is never
    /// inserted and its contents still need styles for the frame they are
    /// re-attached in.
    ///
    /// Otherwise the marks are expanded and pruned:
    ///
    /// * a mark whose ancestor is also marked is dropped — the ancestor's walk
    ///   already covers it;
    /// * a mark is promoted upwards past any ancestor that has never been
    ///   cascaded, so a freshly mounted subtree is never entered with a parent
    ///   map that does not exist yet;
    /// * with `+` / `~` anywhere in the sheet, every FOLLOWING element sibling
    ///   of a mark joins it — that is exactly how far a sibling combinator can
    ///   see, and sheets without one pay nothing;
    /// * the result is ordered shallowest-first, because custom properties
    ///   inherit and a child must never be computed before its parent.
    fn restyle_roots(&mut self, engine: &vn_css::Engine) -> Vec<u32> {
        let marks = std::mem::take(&mut self.dirty_nodes);
        if std::mem::take(&mut self.dirty_all) {
            let mut roots: Vec<u32> = Vec::new();
            if self.root != 0 && self.node(self.root).is_some() {
                roots.push(self.root);
            }
            roots.extend(
                (1..self.nodes.len() as u32)
                    .filter(|&id| id != self.root)
                    .filter(|&id| self.node(id).is_some_and(|n| n.parent == 0)),
            );
            return roots;
        }

        let mut marked: Vec<u32> = marks
            .into_iter()
            .filter(|&id| self.node(id).is_some())
            .collect();
        if marked.is_empty() {
            return Vec::new();
        }
        if engine.uses_sibling_combinators() {
            let mut extra: Vec<u32> = Vec::new();
            for &id in &marked {
                let Some(parent) = self.node(id).map(|n| n.parent).filter(|&p| p != 0) else {
                    continue;
                };
                let Some(siblings) = self.node(parent).map(|n| &n.children) else { continue };
                let Some(pos) = siblings.iter().position(|&c| c == id) else { continue };
                extra.extend(siblings[pos + 1..].iter().copied());
            }
            marked.extend(extra);
        }
        let set: std::collections::HashSet<u32> = marked.iter().copied().collect();

        let mut roots: Vec<u32> = Vec::new();
        for &id in &set {
            if self.ancestors(id).any(|a| set.contains(&a)) {
                continue; // an ancestor's walk already covers this one
            }
            // Promote past ancestors that have never been cascaded — their maps
            // are what a scoped walk would inherit from.
            let mut root = id;
            while let Some(parent) = self.node(root).map(|n| n.parent).filter(|&p| p != 0) {
                match self.node(parent) {
                    Some(p) if p.computed.is_none() => root = parent,
                    _ => break,
                }
            }
            roots.push(root);
        }
        roots.sort_unstable();
        roots.dedup();
        // Promotion can put one root above another; drop the covered ones so no
        // subtree is walked twice, then order parents before children.
        let promoted: std::collections::HashSet<u32> = roots.iter().copied().collect();
        roots.retain(|&id| !self.ancestors(id).any(|a| promoted.contains(&a)));
        roots.sort_by_key(|&id| self.depth(id));
        roots
    }

    /// This node's ancestors, nearest first. Bounded, so a corrupt parent chain
    /// cannot hang the pump.
    fn ancestors(&self, id: u32) -> impl Iterator<Item = u32> + '_ {
        let mut cursor = self.node(id).map(|n| n.parent).unwrap_or(0);
        let mut budget = 1024;
        std::iter::from_fn(move || {
            if cursor == 0 || budget == 0 {
                return None;
            }
            budget -= 1;
            let out = cursor;
            cursor = self.node(cursor).map(|n| n.parent).unwrap_or(0);
            Some(out)
        })
    }

    fn depth(&self, id: u32) -> u32 {
        self.ancestors(id).count() as u32
    }

    /// The root element's computed `font-size` in px, resolved **without
    /// committing a cascade**.
    ///
    /// `rem` is flattened to px when the sheet is COMPILED, so the basis has to
    /// be settled before the sheet the document cascades against is chosen
    /// ([`crate::restyle_all`]). Learning it used to cost a full document pass
    /// per attempt, and a full pass commits `node.computed` — which made the
    /// pass after the recompile look like an author's style change, so every
    /// element with a `transition` animated across it. Invisible on screen (no
    /// frame ran in between) and very visible to `onMounted`, which measures
    /// through `sync_layout`'s forced frame: `apps/meet-probe`'s join card read
    /// its panel at t≈0 of that transition and pinned 293 px where the settled
    /// layout says 278 (research/reports/m23/tails.md §3).
    ///
    /// The root is the one node whose computed style needs no walk: it has no
    /// parent, so nothing is inherited into it, and no siblings, so no
    /// combinator can reach it. One [`vn_css::Engine::compute_node`] is the
    /// whole answer.
    ///
    /// The root's inline style is parsed here when it is dirty — `@argon/theme`
    /// sets `documentElement.style.fontSize` at boot, so the inline style IS
    /// the interesting case — but `inline_dirty` is deliberately left standing:
    /// the pass that commits owns that list, and this is a probe.
    pub(crate) fn probe_root_font_size(
        &self,
        engine: Option<&vn_css::Engine>,
        env: &vn_css::Env,
    ) -> Option<f32> {
        let engine = engine?;
        let n = self.node(self.root)?;
        let parsed;
        let inline: Option<&vn_css::InlineStyle> = if n.inline_dirty {
            let body = n
                .style
                .iter()
                .map(|(k, v)| format!("{}: {}", self.name(*k), v))
                .collect::<Vec<_>>()
                .join("; ");
            parsed = (!body.is_empty())
                .then(|| vn_css::parse_inline_style(&body, &mut Vec::new()))
                .filter(|s| !s.is_empty());
            parsed.as_ref()
        } else {
            n.inline.as_deref()
        };
        let states = engine.compute_node(
            NodeRef { tree: self, id: self.root },
            inline,
            &std::rc::Rc::new(vn_css::Vars::default()),
            None,
            env,
            &mut Vec::new(),
        );
        match states.base.text.font_size? {
            gpui::AbsoluteLength::Pixels(p) => Some(f32::from(p)),
            // `Rems` cannot reach here (the mapper flattens), but if it ever
            // does, resolving it against the current basis would be a fixed
            // point that never moves — take the value at face value instead.
            gpui::AbsoluteLength::Rems(r) => Some(r.0 * vn_css::rem_basis()),
        }
    }

    fn restyle_with(&mut self, engine: &vn_css::Engine, env: &vn_css::Env) {
        let pass = crate::perf::Span::start();
        let phase = crate::perf::Span::start();
        // 1. Re-parse changed inline styles. The list is kept by `set_style`
        // rather than rediscovered — a scan of the whole slab per pass is the
        // one thing a scoped restyle must not do.
        let dirty_inline = std::mem::take(&mut self.inline_dirty_nodes);
        for id in dirty_inline {
            if !self.node(id).is_some_and(|n| n.inline_dirty) {
                continue; // freed, or already re-parsed (the list can repeat).
            }
            let body = {
                let n = self.node(id).unwrap();
                n.style
                    .iter()
                    .map(|(k, v)| format!("{}: {}", self.name(*k), v))
                    .collect::<Vec<_>>()
                    .join("; ")
            };
            let mut diags = Vec::new();
            let parsed = if body.is_empty() {
                None
            } else {
                let style = vn_css::parse_inline_style(&body, &mut diags);
                (!style.is_empty()).then(|| std::rc::Rc::new(style))
            };
            for d in diags {
                warn_css(&format!("[css inline] {d}"));
            }
            let n = self.node_mut(id).unwrap();
            n.inline = parsed;
            n.inline_dirty = false;
        }

        let inline_ms = phase.ms();
        let phase = crate::perf::Span::start();

        // 2. Cascade (immutable walk), then store.
        //
        // The walk is DOCUMENT ORDER, not id order, and that is a requirement
        // rather than a nicety: CSS custom properties inherit, so a node's map
        // is its parent's plus its own declarations, and the parent must
        // therefore be computed first. Every parentless node is a walk root —
        // the mounted root plus any detached subtree (a `<KeepAlive>` cache
        // container is never inserted, and its contents still need styles for
        // the frame they are re-attached in).
        let mut diagnostics: Vec<String> = Vec::new();
        let roots = self.restyle_roots(engine);
        let cascaded_anything = !roots.is_empty();
        let computed: Vec<(u32, vn_css::ComputedStates)> = {
            let this: &Tree = self;
            let root_vars = std::rc::Rc::new(vn_css::Vars::default());
            let mut out = Vec::new();
            // `(node, inherited custom properties, is an ancestor `pointer-events:
            // none`?)`. `pointer-events` is an INHERITED property in CSS, and it
            // is the whole mechanism behind the "invisible panel on top" idiom:
            // a tab panel that is `absolute inset-x-0 top-0 opacity-0
            // pointer-events-none` sits exactly over the visible one, and if the
            // inertness stops at the panel itself, every control INSIDE it is
            // still live and — being painted later — on top. That is what made
            // the meet app's two `Your name` fields fight over the same click
            // (research/reports/m18/live-test-round2.md §4).
            // `(node, inherited custom properties, inherited `pointer-events:
            // none`?, inherited `color`)`.
            //
            // The last element is the `@media` ENVIRONMENT this node answers
            // against. It rides the walk rather than being resolved per node
            // because it changes at exactly one kind of node — a painted root —
            // and `windows::env_at_root` is the O(1) test for that, where
            // `env_for_node`'s ancestor walk is O(depth).
            let mut stack: Vec<(
                u32,
                std::rc::Rc<vn_css::Vars>,
                bool,
                Option<gpui::Hsla>,
                vn_css::Env,
            )> = Vec::new();
            // Free for a single-window app: `ROOTS` is empty until a pop-out
            // actually paints, so the per-child lookup below is never reached.
            let multiwindow = crate::windows::any_secondary();
            for r in roots {
                // **Which window's `@media` environment does this subtree
                // answer against?** (docs/MULTIWINDOW.md §6.) An element knows
                // its window implicitly: it hangs under exactly one painted
                // root, and each painted root belongs to exactly one window. So
                // the walk root resolves it once for the whole subtree — the
                // conditions (`min-width`, `prefers-color-scheme`,
                // `display-mode`, `resolution`, `env(titlebar-area-*)`) are
                // already per-`Env` PARAMETERS inside `compute_node`, so this
                // costs one map lookup per walk root and nothing per node.
                //
                // Free when there is no pop-out: `window_env` short-circuits to
                // the published env the moment no window has a painted root.
                let root_env = crate::windows::env_for_node(this, r, env);
                // A scoped root inherits from its parent's already-computed map;
                // `restyle_roots` guarantees the parent has one (it promotes the
                // root upwards until that holds), so this cannot read a stale
                // ancestor.
                let inherited = this
                    .node(r)
                    .map(|n| n.parent)
                    .filter(|&p| p != 0)
                    .and_then(|p| this.node(p))
                    .and_then(|p| p.computed.as_ref())
                    .map(|c| c.vars.clone())
                    .unwrap_or_else(|| root_vars.clone());
                let parent_computed = this
                    .node(r)
                    .map(|n| n.parent)
                    .filter(|&p| p != 0)
                    .and_then(|p| this.node(p))
                    .and_then(|p| p.computed.as_ref());
                let inherited_inert =
                    parent_computed.is_some_and(|c| c.extras.pointer_events_none == Some(true));
                // `currentColor` resolves against the element's computed
                // `color`, which is inherited when it declares none — so the
                // effective colour rides down the walk beside the vars map.
                let inherited_color = parent_computed.and_then(|c| c.color);
                stack.push((r, inherited, inherited_inert, inherited_color, root_env));
                while let Some((id, parent_vars, parent_inert, parent_color, here_env)) =
                    stack.pop()
                {
                    let Some(n) = this.node(id) else { continue };
                    // Text and anchor nodes have no cascade of their own; they
                    // simply pass the map down to whatever hangs under them.
                    let (child_vars, child_inert, child_color) = if n.kind == Kind::Element {
                        let mut states = engine.compute_node(
                            NodeRef { tree: this, id },
                            n.inline.as_deref(),
                            &parent_vars,
                            parent_color,
                            &here_env,
                            &mut diagnostics,
                        );
                        // `None` = the element declared nothing, so it takes the
                        // parent's value; an explicit `pointer-events: auto`
                        // (`Some(false)`) is how a descendant opts back IN, which
                        // is what reka's `data-[state=open]:pointer-events-auto`
                        // overlay relies on.
                        let inert = states.extras.pointer_events_none.unwrap_or(parent_inert);
                        states.extras.pointer_events_none = Some(inert);
                        let vars = states.vars.clone();
                        let color = states.color;
                        out.push((id, states));
                        (vars, inert, color)
                    } else {
                        (parent_vars, parent_inert, parent_color)
                    };
                    for &c in n.children.iter().rev() {
                        // **The window boundary.** A popped-out node is painted
                        // by a different window, so its whole subtree answers
                        // that window's environment — and it must do so on a
                        // FULL-DOCUMENT pass too, not only when the subtree
                        // happens to be its own walk root. Resolving the env
                        // once per walk root was right for the pass that
                        // changes hands (`set_painted_root` scopes that one)
                        // and silently wrong for every pass afterwards: opening
                        // a THIRD window re-cascades the document from `t.root`
                        // and used to hand the pop-out the primary's `@media`
                        // answers back.
                        let child_env = if multiwindow {
                            crate::windows::env_at_root(c, env).unwrap_or(here_env)
                        } else {
                            here_env
                        };
                        stack.push((c, child_vars.clone(), child_inert, child_color, child_env));
                    }
                }
            }
            out
        };
        let cascade_ms = phase.ms();
        let cascaded = computed.len() as u64;
        let phase = crate::perf::Span::start();
        for d in diagnostics {
            warn_css(&format!("[css] {d}"));
        }
        // 3. Store + feed the animator. ONE Instant for the whole pass (tracks
        // started together must not drift), change-gated via
        // ComputedStates: PartialEq.
        let now = std::time::Instant::now();
        let has_keyframes = engine.has_keyframes();
        // `@keyframes` declarations composed with `var()` resolve against the
        // ANIMATING element, so the advisories they produce belong to the
        // element too — collected here and drained with the animator's own.
        let keyframe_diagnostics = std::cell::RefCell::new(Vec::<String>::new());
        // The cascade switches environments at a painted root and this loop does
        // not walk the tree, so the window has to be asked for again — but only
        // for a node that is actually about to start an animation, and only
        // when a pop-out exists at all. Both gates are free in the single-window
        // case: `any_secondary` is an `is_empty`, and a sheet with no
        // `@keyframes` never reaches the second one.
        let multiwindow = crate::windows::any_secondary();
        ANIMATOR.with(|a| {
            let mut animator = a.borrow_mut();
            for (id, states) in computed {
                // Before `node_mut` takes the tree: `env_for_node` reads it.
                let node_env = if multiwindow && (has_keyframes || !states.animations.is_empty()) {
                    crate::windows::env_for_node(self, id, env)
                } else {
                    *env
                };
                // **A node painted by a pop-out can need its animations
                // re-bound while its own computed style has not moved at all.**
                // The element's declarations are identical in both windows; it
                // is the `@keyframes` block its `animation-name` refers to that
                // is declared per window. The change-gate below therefore never
                // fires for it — and the run it started with, on the frame
                // before its window had painted and `ROOTS` was still empty,
                // was resolved against the PRIMARY's environment.
                let rebind = node_env != *env && !states.animations.is_empty();
                if let Some(n) = self.node_mut(id) {
                    if n.computed.as_ref() != Some(&states) {
                        animator.on_computed(id, n.computed.as_ref(), &states, now);
                        // `@keyframes` are document-scoped, so the animation
                        // half needs the sheet; a sheet without any pays
                        // nothing for the check.
                        //
                        // **Against the PAINTING window's environment**, like
                        // the rest of the cascade (docs/MULTIWINDOW.md §6). A
                        // `@keyframes` block inside an `@media` one is declared
                        // only where that query holds, and `env()` inside a
                        // keyframe resolves against the window too — so the
                        // same `animation-name` can be a different animation in
                        // a 380px pop-out and in the document that opened it.
                        // This used to take the pass's fallback environment,
                        // which is the PRIMARY window's
                        // (m22/integration.md §6 item 4).
                        if has_keyframes || !states.animations.is_empty() {
                            animator.on_computed_animations(id, &states, now, &|name| {
                                engine.keyframes_for(
                                    name,
                                    &states.vars,
                                    &node_env,
                                    &mut keyframe_diagnostics.borrow_mut(),
                                )
                            });
                        }
                        // A restyle under a stationary pointer: the flip already
                        // happened (or never will), so the animator would keep
                        // aiming at the OLD fold. Re-aim it — but only for a
                        // node that is actually in a hovered state, or that is
                        // still holding one, so an ordinary restyle pays a set
                        // lookup and nothing else.
                        if states.hover_transitions() {
                            let target = fold_hover_target(id, &states);
                            if target.is_some() || animator.holds_state(id) {
                                animator.on_state(id, &states, target.as_ref(), now);
                            }
                        }
                        n.computed = Some(states);
                    } else if rebind {
                        // Same style, possibly a different animation. The run
                        // is kept (the name has not changed) and only the block
                        // behind it is re-resolved, which is what a browser
                        // does when a media query flips under a running
                        // animation.
                        animator.on_computed_animations(id, &states, now, &|name| {
                            engine.keyframes_for(
                                name,
                                &states.vars,
                                &node_env,
                                &mut keyframe_diagnostics.borrow_mut(),
                            )
                        });
                    }
                }
            }
            for d in animator.take_diagnostics() {
                warn_css(&format!("[css transition] {d}"));
            }
        });
        for d in keyframe_diagnostics.into_inner() {
            warn_css(&format!("[css] {d}"));
        }
        // The set of elements that have to carry gpui's `.group(name)`. Costs
        // one pass over the slab, and only for a sheet that has a
        // `group-hover:` rule in it at all — and only for a pass that actually
        // recomputed something. The second half is what keeps
        // `flush_pending_style` cheap: `getComputedStyle` is a flush point now,
        // so a mount that reads it a dozen times runs a dozen passes, and all
        // but the first find no dirty root. No node's `ComputedStates` changed,
        // so neither set can have.
        if !cascaded_anything {
            crate::perf::note_restyle(pass.ms(), inline_ms, cascade_ms, phase.ms(), cascaded);
            return;
        }
        self.group_anchors.clear();
        self.group_hover_deps.clear();
        if engine.uses_group_states() {
            for (i, slot) in self.nodes.iter().enumerate() {
                let Some(slot) = slot else { continue };
                if let Some(cs) = &slot.computed {
                    if let Some((key, _)) = &cs.group_hover {
                        self.group_anchors.insert(*key as u32);
                        // Only the ones with a `transition` need the animator's
                        // flip signal; the rest are pure paint-ladder and cost
                        // nothing here.
                        if cs.hover_transitions() {
                            self.group_hover_deps
                                .entry(*key as u32)
                                .or_default()
                                .push(i as u32);
                        }
                    }
                    if let Some((key, _)) = &cs.group_active {
                        self.group_anchors.insert(*key as u32);
                    }
                }
            }
        }
        crate::perf::note_restyle(pass.ms(), inline_ms, cascade_ms, phase.ms(), cascaded);
    }
}

// ---------------------------------------------------------------------------
// cascade wiring
// ---------------------------------------------------------------------------

thread_local! {
    /// The cascade used before any stylesheet is installed — an empty sheet, so
    /// inline styles and their custom properties take exactly the same path
    /// they will once a sheet arrives.
    static NO_SHEET: vn_css::Engine = vn_css::Engine::compile("");
    /// Runtime CSS advisories already printed. The cascade's compile-time
    /// diagnostics are deduped inside the engine; these come from computed-value
    /// time (an unresolvable `var()`, a reference cycle) and are therefore
    /// per-element, so a page with 200 buttons must not print 200 lines.
    /// The CSS every `<style>` element in the tree is carrying right now,
    /// concatenated in document order — see [`sync_runtime_sheets`].
    static RUNTIME_CSS: RefCell<String> = const { RefCell::new(String::new()) };
    static CSS_WARNED: RefCell<std::collections::HashSet<String>> =
        RefCell::new(std::collections::HashSet::new());
    static ENGINE: RefCell<Option<vn_css::Engine>> = const { RefCell::new(None) };
    /// The text the current engine was compiled from, kept so the sheet can be
    /// compiled AGAIN against a different `rem` basis — see [`restyle_all`].
    static SHEET_SOURCE: RefCell<String> = const { RefCell::new(String::new()) };
    static ANIMATOR: RefCell<vn_css::Animator> = RefCell::new(vn_css::Animator::new());
    /// The `@media` environment the cascade is evaluated against **for
    /// everything the PRIMARY window paints** — the native stand-in for a
    /// browser's viewport + OS preferences. The host refreshes it every frame
    /// from that window (see [`set_env`]).
    ///
    /// A pop-out's own environment lives in its `WindowState`
    /// (`crate::windows::set_env`); `restyle_with` carries the chosen one down
    /// the cascade walk and switches at a painted root
    /// (`crate::windows::env_at_root`), which is the only node where the window
    /// can change. This one stays a single slot because it is also the FALLBACK for
    /// a pop-out that has been requested but has not painted yet, and because
    /// `matchMedia` — a JS-realm global, and there is one realm — means it
    /// (docs/MULTIWINDOW.md §7.7, A-M04).
    static ENV: Cell<vn_css::Env> = Cell::new(vn_css::Env::default());
}

/// Publishes the current window environment. Returns `true` when it CHANGED,
/// which obliges the caller to mark the tree dirty: a resize or a theme flip
/// can turn any `@media` rule in the sheet on or off, so every node has to be
/// re-cascaded. Both events are rare enough that a full invalidation is the
/// right trade against tracking which rules a condition feeds.
pub fn set_env(env: vn_css::Env) -> bool {
    // `prefers-reduced-motion` is also what suppresses running transitions.
    // Pushed unconditionally (the animator only reacts to the false→true edge)
    // so it cannot be missed on the one frame where the rest of the env happens
    // to equal the default.
    //
    // **Deliberately only from the primary window** (docs/MULTIWINDOW.md §5.3):
    // `set_reduce_motion` is a whole-ANIMATOR flag and the animator is shared,
    // so a second window pushing its own value would stomp the first's. It is
    // an OS-wide preference — `App::reduce_motion()` answers the same in every
    // window — so taking it from one window is not an approximation.
    set_reduce_motion(env.reduce_motion);
    crate::windows::set_env(crate::windows::primary(), env);
    ENV.with(|e| {
        if e.get() == env {
            return false;
        }
        e.set(env);
        true
    })
}

/// Publishes ONE non-primary window's `@media` environment (§6). Returns `true`
/// when it changed, in which case the subtree that window paints is marked for
/// a scoped restyle — not the whole document, because nothing outside that
/// window's painted root can have answered against this env.
pub fn set_window_env(k: crate::windows::WinKey, env: vn_css::Env) -> bool {
    if !crate::windows::set_env(k, env) {
        return false;
    }
    let root = crate::windows::root_of(k);
    if root != 0 {
        with_tree(|t| t.mark_restyle_subtree(root));
    }
    true
}

pub fn env() -> vn_css::Env {
    ENV.with(|e| e.get())
}

/// The per-frame interpolated overlay for a node (None = idle). `base` is the
/// node's computed style — a `@keyframes` animation reads it for the implicit
/// 0 % / 100 % of any property its keyframes do not declare.
pub fn animation_overlay(
    id: u32,
    base: &gpui::StyleRefinement,
    now: std::time::Instant,
) -> Option<gpui::StyleRefinement> {
    ANIMATOR.with(|a| a.borrow().overlay(id, base, now))
}

thread_local! {
    /// Which elements the pointer is currently inside, as gpui's own hover
    /// edges report it. One set, not two: an element is "group-hovered" exactly
    /// when the anchor its `group-hover:` rung names is in here.
    ///
    /// This exists ONLY so that a restyle landing while the pointer is already
    /// over something can re-fold the target — the flip itself is an edge, and
    /// an edge alone cannot answer "what state am I in now?".
    static HOVERED: RefCell<std::collections::HashSet<u32>> =
        RefCell::new(std::collections::HashSet::new());
}

/// gpui saw the pointer cross this element's hitbox.
///
/// This is the whole state-transition story on the input side. In a browser a
/// `transition` on the BASE rule animates the changes a `:hover` rule makes;
/// here the hover rung is installed into gpui's paint-time ladder, which
/// refines it over the computed style with no clock at all — so a hover SNAPPED
/// (m7 recorded this as "state-rule transitions rejected by design", which was
/// about a `transition` declared ON the state rule; the base-rule case is the
/// one authors actually write and it was snapping too).
///
/// So: [`crate::render::apply_computed_style`] strips the transitioned
/// properties out of the rung before gpui ever sees it, and this routes the
/// flip into the animator instead. What the author did NOT name in `transition`
/// stays in the rung and still snaps — which is also what CSS says.
///
/// Cost per flip is one map lookup plus one `on_state` per element that
/// actually declares a transition, so m11's "no full-tree work per hover" holds
/// (`group_hover_deps` is a precomputed list, not a walk).
pub fn note_hover(id: u32, hovered: bool) {
    let changed = HOVERED.with(|h| {
        let mut h = h.borrow_mut();
        if hovered { h.insert(id) } else { h.remove(&id) }
    });
    if !changed {
        return;
    }
    let now = std::time::Instant::now();
    retarget_state(id, now);
    // The other half of `group-hover:`: this element may be the `.group` some
    // descendants watch, and a flip on it is the only signal they get.
    let deps = read_tree(|t| t.group_hover_deps.get(&id).cloned());
    for dep in deps.into_iter().flatten() {
        retarget_state(dep, now);
    }
}

/// Is the pointer inside `id`'s hitbox right now?
///
/// Only ever TRUE for a node the renderer asked gpui to report hover edges for
/// — see [`crate::render::wants_style_hover`]. A node with a `:hover` rule but
/// no `transition` is styled entirely inside gpui's paint-time ladder and never
/// reaches [`note_hover`], so this answers `false` for it. That is a real limit
/// of reading the state back rather than a bug; `live_style` says the same
/// thing and docs/TESTING.md writes it down.
pub fn is_hovered(id: u32) -> bool {
    HOVERED.with(|h| h.borrow().contains(&id))
}

/// The style that is IN EFFECT for `id` on this frame — what a browser would
/// answer for `getComputedStyle(el)`.
///
/// It reassembles, for a reader, exactly the composition
/// [`crate::render::apply_computed_style`] hands gpui:
///
///   base  →  the animator's in-flight overlay  →  the hover rung minus the
///            properties the animator took over (only while hovered)
///
/// …which is why it is the one bucket an assertion can use to prove a `:hover`
/// actually LANDED. The static `hover` bucket next to it says only "the cascade
/// produced this rung", which is true whether or not a pointer is anywhere near.
///
/// Two honest gaps, both structural: `:active` and the untransitioned half of a
/// hover rung are refined by gpui at paint time off a hitbox we cannot read
/// back, so an element whose `:hover` declares no `transition` reports its base
/// here. Introduced for `vn-test` (docs/TESTING.md); nothing on the paint path
/// calls it.
pub fn live_style(id: u32) -> Option<gpui::StyleRefinement> {
    use gpui::Refineable as _;
    let cs = read_tree(|t| t.node(id).and_then(|n| n.computed.clone()))?;
    let mut live = (*cs.base).clone();
    if let Some(overlay) = animation_overlay(id, &cs.base, std::time::Instant::now()) {
        live.refine(&overlay);
    }
    // `pointer-events: none` keeps the rungs off the element entirely, so it
    // keeps them out of here too — same condition, same place in the order.
    if cs.extras.pointer_events_none != Some(true) {
        if let Some(rung) = fold_hover_target(id, &cs) {
            match vn_css::strip_transitioned(&rung, &cs.transitions) {
                Some(stripped) => live.refine(&stripped),
                None => live.refine(&rung),
            }
        }
    }
    Some(live)
}

/// The refinement `id`'s ladder produces for the pointer state it is in RIGHT
/// NOW — `None` when it is in the plain base state. Reads [`HOVERED`] only, so
/// it is safe to call with the tree already borrowed.
fn fold_hover_target(id: u32, cs: &vn_css::ComputedStates) -> Option<gpui::StyleRefinement> {
    HOVERED.with(|h| {
        let h = h.borrow();
        let own = h.contains(&id);
        let group = cs
            .group_hover
            .as_ref()
            .is_some_and(|(key, _)| h.contains(&(*key as u32)));
        cs.hovered(own, group)
    })
}

/// Re-aims one node's state transition at whatever its ladder says right now.
fn retarget_state(id: u32, now: std::time::Instant) {
    let Some(cs) = read_tree(|t| t.node(id).and_then(|n| n.computed.clone())) else {
        return;
    };
    if !cs.hover_transitions() {
        return;
    }
    let target = fold_hover_target(id, &cs);
    ANIMATOR.with(|a| a.borrow_mut().on_state(id, &cs, target.as_ref(), now));
}

/// True while any transition runs — the host keeps repainting. Also drops
/// settled tracks (visually a no-op: a finished track equals the base).
pub fn animating() -> bool {
    let now = std::time::Instant::now();
    ANIMATOR.with(|a| {
        let a = a.borrow();
        a.any_active(now) || a.any_animation(now)
    })
}

/// Advances the animator's clock and queues whatever events crossing a boundary
/// produced. Must run EVERY frame, from [`crate::frame_tick`] — it used to live
/// inside [`animating`], which the host calls behind a `||` short-circuit, so a
/// frame that already had a reason to repaint skipped the clock entirely and no
/// `animationstart` was ever queued.
pub fn advance_animations() {
    let now = std::time::Instant::now();
    ANIMATOR.with(|a| {
        let mut a = a.borrow_mut();
        // `tick` turns a run crossing its delay or its last iteration into an
        // `animationstart`/`animationend`; it runs before `finish_settled`
        // drops the settled transition tracks (which is where `transitionend`
        // comes from).
        a.tick(now);
        a.finish_settled(now);
    });
}

/// Drains the `transitionend` / `animation*` events the animator queued.
pub fn take_anim_events() -> Vec<vn_css::AnimEvent> {
    ANIMATOR.with(|a| a.borrow_mut().take_events())
}

pub fn animator_gc() {
    ANIMATOR.with(|a| {
        a.borrow_mut()
            .gc(&|id| read_tree(|t| t.node(id).is_some()));
    });
    // A node unmounted while the pointer was over it never gets a leave edge
    // (gpui drops the element and with it the listener), so the set has to be
    // swept too — otherwise the id is "hovered" forever and a later node
    // reusing it would come up pre-hovered.
    HOVERED.with(|h| {
        let mut h = h.borrow_mut();
        if !h.is_empty() {
            read_tree(|t| h.retain(|&id| t.node(id).is_some()));
        }
    });
}

pub fn set_reduce_motion(on: bool) {
    ANIMATOR.with(|a| a.borrow_mut().set_reduce_motion(on));
}

/// Installs a compiled stylesheet, keeping its source for
/// [`recompile_for_rem_basis`].
/// Everything the engine compiles: the build's sheet, then whatever the running
/// app has put in `<style>` elements.
///
/// The runtime half comes LAST, so at equal specificity it wins — which is
/// where a `<style>` appended to the document sits in a browser's cascade too.
fn full_source() -> String {
    let base = SHEET_SOURCE.with(|s| s.borrow().clone());
    RUNTIME_CSS.with(|r| {
        let r = r.borrow();
        if r.is_empty() { base } else { format!("{base}\n{r}") }
    })
}

/// Collects every `<style>` element's text and, if it has changed, recompiles.
///
/// **A `<style>` element in the tree now works.** It used to render nothing and
/// install nothing — a real gap, and one every component library walks into:
/// reka-ui appends one at runtime to hide a scroll area's scrollbars,
/// `vite-plugin-css-injected-by-js` ships a whole bundle that way, and
/// `document.createElement("style")` is how half the ecosystem does anything
/// dynamic with CSS. The element still paints nothing, exactly as in a browser
/// (it is metadata content, `vn_tree::render::is_metadata`); what changed is
/// that its CSS reaches the cascade.
///
/// Called from [`restyle_all`], i.e. once per flush that dirtied the tree —
/// which is the only moment a `<style>`'s text can have changed. The comparison
/// is against the installed string, so a flush that did not touch one costs a
/// walk of the (usually empty) style-node set and nothing else, and a recompile
/// cannot re-trigger itself.
fn sync_runtime_sheets() {
    let next = with_tree(|t| {
        let mut ids: Vec<u32> = t.style_nodes.iter().copied().collect();
        // Creation order stands in for document order: Vue creates a node
        // before it inserts it, and a sheet appended later has to win.
        ids.sort_unstable();
        let mut out = String::new();
        for id in ids {
            if t.node(id).is_none() {
                continue;
            }
            fn walk(t: &Tree, id: u32, out: &mut String) {
                let Some(n) = t.node(id) else { return };
                out.push_str(&n.text);
                for &c in &n.children {
                    walk(t, c, out);
                }
            }
            walk(t, id, &mut out);
            out.push('\n');
        }
        out
    });
    if RUNTIME_CSS.with(|r| *r.borrow() == next) {
        return;
    }
    RUNTIME_CSS.with(|r| *r.borrow_mut() = next);
    // The dedupe set is deliberately NOT cleared. A `<style>` arrives in pieces
    // — created, then filled, then sometimes refilled — and each step is a
    // recompile; clearing would make every advisory the BUILD's sheet already
    // gave re-print once per step, which is the exact noise `set_stylesheet`'s
    // freshness check exists to prevent. Whatever the new text says that has
    // not been said is a new string and speaks on its own.
    recompile_sheet("a <style> element in the tree changed — recompiling the stylesheet");
}

pub fn set_stylesheet(css: String) {
    // A genuinely NEW sheet is a new set of gaps: an advisory suppressed
    // against the old one must be free to speak again (this is the HMR path).
    // Recompiling the SAME text under a new rem/viewport basis is not that —
    // see [`recompile_sheet`], which is why the clear lives here and not in
    // [`set_engine`].
    let fresh = SHEET_SOURCE.with(|s| *s.borrow() != css);
    if fresh {
        CSS_WARNED.with(|w| w.borrow_mut().clear());
    }
    SHEET_SOURCE.with(|s| *s.borrow_mut() = css);
    set_engine(vn_css::Engine::compile(&full_source()));
}

/// Installs a compiled engine WITHOUT recording a source to recompile from.
/// [`set_stylesheet`] is the path the host uses; this one is for tests and for
/// the recompile inside [`restyle_all`], which already has the source.
pub(crate) fn set_engine(engine: vn_css::Engine) {
    // Every running `@keyframes` run holds the block it started with, and a new
    // sheet can redefine or delete any of them.
    ANIMATOR.with(|a| a.borrow_mut().clear());
    // Through `warn_css`, so a recompile under a new rem/viewport basis does
    // not re-print what the same source already said. Compiling the sheet three
    // times at boot — once cold, once for the rem basis, once for the client
    // size — is normal and used to TRIPLE the advisory stream the developer
    // sees. `set_stylesheet` clears the dedupe set when the text itself
    // changes, which is the case that has to speak again.
    for d in &engine.diagnostics {
        warn_css(&format!("[css] {d}"));
    }
    ENGINE.with(|c| *c.borrow_mut() = Some(engine));
}

/// Reads the installed stylesheet. `None` = no sheet (inline styles only).
///
/// The one reader outside the restyle pass is devtools' rule provenance
/// (`crate::devtools::matched_rules`), which asks the engine which rules match
/// a node — the same question the cascade asks, without folding the answer.
pub(crate) fn with_engine<R>(f: impl FnOnce(Option<&vn_css::Engine>) -> R) -> R {
    ENGINE.with(|e| f(e.borrow().as_ref()))
}

/// Restyles under the current engine (or inline-only when none is set), then
/// re-runs the whole thing if the ROOT element turned out to define a different
/// `rem`.
///
/// `rem` is the root element's `font-size`, and vn-css flattens it to px at
/// SHEET COMPILE time (gpui's style fields are `Pixels`; carrying `Rems`
/// through every one of them is a different project — see
/// [`vn_css::set_rem_basis`]). So a root font-size the app sets from JS cannot
/// move the sheet that is already compiled; the sheet has to be compiled again.
///
/// Which is what this does, and the loop is the whole subtlety: the root's own
/// `font-size` may itself be written in `rem` or `em`, so recompiling can
/// change the very value that triggered the recompile. CSS resolves that by
/// defining the root's own `rem` against the INITIAL font size; we get the same
/// answer by iterating to a fixed point and capping it, which also terminates
/// on the pathological `html { font-size: 2rem }`.
///
/// In practice this fires at most once in an app's life — `@argon/theme` sets
/// `documentElement.style.fontSize` once at boot — and never at all for an app
/// that leaves the root font-size alone, which is the case the `changed` test
/// keeps free.
pub fn restyle_all() {
    sync_runtime_sheets();
    let env = env();
    // Viewport units are flattened to px at sheet-compile time, exactly like
    // `rem` — so a resize has to recompile, and only when the sheet actually
    // contains a `vh`/`vw`. Almost none do; the ones that do cannot do without
    // it (`max-h-[85vh]` is the only thing keeping reka's dialog inside the
    // window, and dropping it turned the dialog into a full-window black slab).
    // **A-M05, the phase-1 guard (docs/MULTIWINDOW.md §6.2 option C).** `vw` /
    // `vh` / `rem` are flattened to px at SHEET-COMPILE time, into one
    // process-wide basis, so two differently-sized windows sharing a
    // `vh`-using sheet would recompile the single `Engine` on every frame —
    // and `set_engine` clears the animator, so every running transition would
    // be cancelled while doing it. The basis therefore stays the PRIMARY
    // window's, and we say so once.
    //
    // The gate is free for the apps that use no viewport unit, which is most of
    // them: `uses_viewport_units()` is false and per-window `@media` is
    // completely correct today. When it is true and a pop-out is open, the
    // conditions are still per-window (above); only the LENGTHS come from the
    // primary window's viewport.
    if vn_css::set_viewport_basis(env.viewport_width, env.viewport_height)
        && vn_css::uses_viewport_units()
    {
        recompile_sheet(&format!(
            "viewport basis is now {}x{}px — recompiling the stylesheet (it uses vh/vw units)",
            env.viewport_width, env.viewport_height
        ));
    }
    if vn_css::uses_viewport_units() && crate::windows::any_secondary() {
        crate::render::warn_compat(
            "multiwindow-viewport-units",
            "A-M05: this stylesheet uses `vw`/`vh`/`rem`, which vue-native flattens to px when \
             the sheet is COMPILED — so those lengths resolve against the PRIMARY window's \
             viewport in every window, including pop-outs. `@media` conditions, `env()` and \
             `prefers-color-scheme` are already per-window. See docs/MULTIWINDOW.md §6.2.",
        );
    }
    // **The rem basis is settled BEFORE the document is cascaded, not by
    // cascading it.** `rem` is flattened at sheet-compile time, so the basis
    // has to be known before the sheet the document cascades against is
    // chosen. This used to iterate a FULL cascade per pass — and a full
    // cascade COMMITS `node.computed`, so the pass that followed the recompile
    // was, to the animator, an author's style change: every element carrying a
    // `transition` animated from the 16px-basis geometry to the real one.
    //
    // Nothing painted in between, so nothing showed it — except a layout read.
    // `apps/meet-probe`'s join card measures its panel from `onMounted`, which
    // is `sync_layout`'s forced frame, which is t≈0 of that transition: the
    // panel answered 293 where the settled layout (and a browser) says 278, and
    // the app wrote that number back as a height for the life of the process
    // (research/reports/m23/tails.md §3).
    //
    // The root has no parent and no siblings, so its own computed style is one
    // `compute_node`: the basis can be resolved from it alone, and the document
    // is cascaded exactly ONCE, under the settled basis. Which is also what a
    // browser does — `rem` is resolved at computed-value time, so an
    // intermediate computed style never exists to transition from.
    for _ in 0..REM_BASIS_PASSES {
        let Some(root_font_size) = ENGINE.with(|eng| {
            let eng = eng.borrow();
            read_tree(|t| t.probe_root_font_size(eng.as_ref(), &env))
        }) else {
            break;
        };
        if !vn_css::set_rem_basis(root_font_size) {
            break;
        }
        if !recompile_sheet(&format!(
            "rem basis is now {root_font_size}px (the root element's font-size) — recompiling \n             the stylesheet"
        )) {
            break;
        }
    }
    ENGINE.with(|eng| {
        let eng = eng.borrow();
        with_tree(|t| t.restyle(eng.as_ref(), &env));
    });
}

/// A FORCED STYLE FLUSH: run the pending restyle now, because JS is about to
/// read a computed value.
///
/// In a browser, `getComputedStyle` is a flush point — CSSOM §5 makes it return
/// the value the DOM currently in memory produces, so a component may mutate an
/// attribute and read the result back on the very next line. Ours cascaded once
/// per JS wave instead, so such a read answered with the PREVIOUS style.
///
/// That is not an academic difference. reka-ui's `<Presence>` is built on it:
/// when an element stops being present it does
///
/// ```js
/// await nextTick();                                    // data-state="closed"
/// const name = getComputedStyle(node).animationName;   // must be "exit"
/// if (name === "none" || name === prevAnimationName) unmount();
/// ```
///
/// With a stale read the name was still `enter`, so `prevAnimationName ===
/// currentAnimationName`, so every dialog, popover, drawer and tooltip in the
/// library unmounted INSTANTLY instead of playing its exit animation.
///
/// Only the CASCADE is flushed, not layout: `getBoundingClientRect` still
/// reports the last painted rectangle (m18), because a box's size is decided by
/// gpui's layout pass and there is no way to run one outside a frame.
pub fn flush_pending_style() {
    let pending = with_tree(|t| {
        if !t.dirty {
            return false;
        }
        t.dirty = false;
        t.style_flushed = true;
        true
    });
    if pending {
        restyle_all();
        crate::render::gc_node_state();
        animator_gc();
    }
}

/// Recompiles the retained stylesheet source under whatever basis just changed,
/// and marks the tree for a full restyle. `false` = there was no source to
/// recompile (before the first `__vn_set_stylesheet`), so the caller should
/// stop.
fn recompile_sheet(why: &str) -> bool {
    // `dev`, not `css`. Nothing here describes a divergence from a browser:
    // it says the engine flattened `rem`/`vh` at compile time and has just
    // re-flattened them against a number that moved. A developer counting CSS
    // warnings in the Console panel was being handed three of these per boot
    // (cold, the root font-size, the first real client size) as if they were
    // gaps, which is exactly the noise m19's rule — an advisory has to name a
    // real divergence — was written to keep out. The line still prints and is
    // still in the ring; it is on the channel that says "state trace".
    crate::log::record("dev", format!("[css] {why}"));
    let css = full_source();
    if css.trim().is_empty() {
        return false;
    }
    set_engine(vn_css::Engine::compile(&css));
    with_tree(|t| t.mark_dirty());
    true
}

/// One recompile to adopt the root's font-size, and one more in case that
/// font-size was itself relative. A third would be a cycle.
const REM_BASIS_PASSES: usize = 3;


/// The matcher's view of one element node.
#[derive(Clone, Copy)]
pub struct NodeRef<'t> {
    pub tree: &'t Tree,
    pub id: u32,
}

impl<'t> NodeRef<'t> {
    fn node(&self) -> &'t Node {
        self.tree.node(self.id).expect("NodeRef points at a live node")
    }

    fn element_sibling(&self, forward: bool) -> Option<NodeRef<'t>> {
        let parent = self.node().parent;
        if parent == 0 {
            return None;
        }
        let siblings = &self.tree.node(parent)?.children;
        let pos = siblings.iter().position(|&c| c == self.id)?;
        let range: Box<dyn Iterator<Item = u32>> = if forward {
            Box::new(siblings[pos + 1..].iter().copied())
        } else {
            Box::new(siblings[..pos].iter().rev().copied())
        };
        for id in range {
            if self.tree.node(id).is_some_and(|n| n.kind == Kind::Element) {
                return Some(NodeRef { tree: self.tree, id });
            }
        }
        None
    }
}

impl<'t> vn_css::MatchDom for NodeRef<'t> {
    fn key(&self) -> u64 {
        self.id as u64
    }
    fn parent(&self) -> Option<Self> {
        let p = self.node().parent;
        (p != 0).then(|| NodeRef { tree: self.tree, id: p })
    }
    fn prev_sibling(&self) -> Option<Self> {
        self.element_sibling(false)
    }
    fn next_sibling(&self) -> Option<Self> {
        self.element_sibling(true)
    }
    /// Every ELEMENT child, for `:has()`.
    ///
    /// Anchor nodes are Vue fragments: not elements, but they DO carry element
    /// children, so the walk flattens through them (and through text nodes,
    /// which have none) rather than stopping. Without that a `:has()` would
    /// miss everything under a `v-if` or a `<template>`.
    fn each_child(&self, f: &mut dyn FnMut(Self)) {
        fn walk<'t>(tree: &'t Tree, id: u32, f: &mut dyn FnMut(NodeRef<'t>)) {
            let Some(node) = tree.node(id) else { return };
            for &child in &node.children {
                match tree.node(child).map(|n| n.kind) {
                    Some(Kind::Element) => f(NodeRef { tree, id: child }),
                    Some(_) => walk(tree, child, f),
                    None => {}
                }
            }
        }
        walk(self.tree, self.id, f);
    }
    fn has_tag(&self, name: &str) -> bool {
        // The `html` / `body` aliasing for the document root lives in vn-css
        // (`element.rs`), so every `MatchDom` gets it — the query matcher and
        // the engine's own tests included.
        self.tree.name(self.node().tag) == name
    }
    fn tag_eq(&self, other: &Self) -> bool {
        self.node().tag == other.node().tag
    }
    fn has_class(&self, name: &str) -> bool {
        self.node().classes.iter().any(|c| c == name)
    }
    fn tag_name(&self) -> &str {
        self.tree.name(self.node().tag)
    }
    fn each_class(&self, f: &mut dyn FnMut(&str)) {
        for c in &self.node().classes {
            f(c);
        }
    }
    fn has_scope(&self, name: &str) -> bool {
        self.node().scopes.iter().any(|s| s == name)
    }
    /// The DOM attribute channel, straight off `Node::props`. Names are
    /// interned once per sheet compile's worth of lookups; an attribute a
    /// selector asks for that JS has never written has no id at all, which is
    /// the cheap "absent" answer.
    fn attr(&self, name: &str) -> Option<&str> {
        let key = *self.tree.name_ids.get(name)?;
        self.node()
            .props
            .iter()
            .find(|(k, _, _)| *k == key)
            .map(|(_, v, _)| v.as_str())
    }
    fn is_empty(&self) -> bool {
        self.node().children.is_empty() && self.node().text.is_empty()
    }
    fn is_root(&self) -> bool {
        self.node().parent == 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A `<style>` element in the tree used to render nothing AND install
    /// nothing. Only the first of those is the browser's behaviour: `<style>`
    /// is metadata content, so it generates no box — but its CSS reaches the
    /// cascade, and every component library counts on that. reka-ui appends one
    /// at runtime to hide a scroll area's scrollbars,
    /// `vite-plugin-css-injected-by-js` ships a whole bundle that way, and
    /// `document.createElement("style")` is how half the ecosystem does
    /// anything dynamic with CSS (engine-round.md §4.7).
    ///
    /// The runtime half is concatenated AFTER the build's sheet, so at equal
    /// specificity it wins — which is where a `<style>` appended to the
    /// document sits in a browser's cascade — and a change to its TEXT
    /// recompiles, because that is the only moment the text can have moved.
    #[test]
    fn a_runtime_style_element_reaches_the_cascade_and_recompiles_when_its_text_moves() {
        let opacity = |id: u32| {
            read_tree(|t| t.node(id).and_then(|n| n.computed.as_ref()).and_then(|c| c.base.opacity))
        };

        // The build's own sheet: what a `<style>` has to be able to override.
        set_stylesheet(".probe { opacity: 0.25 }".into());
        let (probe, sheet) = with_tree(|t| {
            let root = t.create_root();
            let div = t.intern("div");
            let probe = t.create_element(div);
            t.set_class(probe, "probe");
            t.insert(probe, root, 0);
            let style_tag = t.intern("style");
            let sheet = t.create_element(style_tag);
            t.insert(sheet, root, 1);
            (probe, sheet)
        });
        restyle_all();
        assert_eq!(opacity(probe), Some(0.25), "the build's sheet, with an empty <style>");

        // The `<style>` arrives in pieces — created, then filled — which is
        // exactly how Vue and reka do it.
        with_tree(|t| t.set_element_text(sheet, ".probe { opacity: 0.5 }".into()));
        restyle_all();
        assert_eq!(opacity(probe), Some(0.5), "a runtime <style> installs its rules");

        // Refilled: the sheet has to be recompiled, not remembered.
        with_tree(|t| t.set_element_text(sheet, ".probe { opacity: 0.75 }".into()));
        restyle_all();
        assert_eq!(opacity(probe), Some(0.75), "changed text re-installs");

        // A restyle that did NOT touch the text must not change the answer (and
        // costs a walk of the style-node set and nothing else).
        restyle_all();
        assert_eq!(opacity(probe), Some(0.75), "an unchanged sheet is not recompiled away");

        // And emptying it hands the cascade back to the build's own sheet.
        with_tree(|t| t.set_element_text(sheet, String::new()));
        restyle_all();
        assert_eq!(opacity(probe), Some(0.25), "removing the rules restores the build's sheet");
    }

    /// **A `rem` recompile is the engine correcting its own basis, not the
    /// author changing a declaration — so it must not start a transition.**
    ///
    /// `rem` is flattened to px when the sheet is COMPILED, so an app that sets
    /// its own root font-size (every Tailwind app with a 14 px base) makes the
    /// engine compile twice. [`restyle_all`] used to learn the basis by
    /// CASCADING the document, and a cascade commits `node.computed`: the pass
    /// after the recompile was, to the animator, a style change, and every
    /// element carrying a `transition` animated from the 16 px-basis geometry
    /// to the real one.
    ///
    /// No frame ran between the two passes, so nothing on screen showed it —
    /// but `onMounted` measures through `sync_layout`'s forced frame, which is
    /// t=0 of that animation. `apps/meet-probe`'s join card measured its panel
    /// at 293 px where the settled layout says 278 and pinned it as its own
    /// height (research/reports/m23/tails.md §3).
    #[test]
    fn a_rem_recompile_at_boot_starts_no_transition() {
        set_stylesheet(".card { padding: 1.5rem; transition: all 300ms }".into());
        let card = with_tree(|t| {
            let root = t.create_root();
            // EXACTLY `@argon/theme`'s line, and the reason the cold compile
            // cannot have known the basis: an INLINE root font-size, written
            // before the app mounts.
            let font_size = t.intern("font-size");
            t.set_style(root, font_size, "14px".into());
            let div = t.intern("div");
            let card = t.create_element(div);
            t.set_class(card, "card");
            t.insert(card, root, 0);
            card
        });

        // The basis is resolved from the ROOT ALONE, and resolving it commits
        // nothing: no node has a computed style until the one cascade runs.
        let probed = ENGINE.with(|eng| {
            let eng = eng.borrow();
            read_tree(|t| t.probe_root_font_size(eng.as_ref(), &env()))
        });
        assert_eq!(probed, Some(14.0), "the root's own inline font-size, without a cascade");
        assert!(
            read_tree(|t| t.node(t.root).is_some_and(|n| n.computed.is_none())),
            "probing the basis must not commit a computed style"
        );

        restyle_all();
        assert_eq!(vn_css::rem_basis(), 14.0, "the sheet was recompiled against the root");

        // `1.5rem` at 14 px is 21, not the 24 the cold sheet flattened.
        let padding = read_tree(|t| {
            t.node(card).and_then(|n| n.computed.as_ref()).and_then(|c| c.base.padding.top)
        });
        assert_eq!(
            padding,
            Some(gpui::DefiniteLength::Absolute(gpui::AbsoluteLength::Pixels(gpui::px(21.0)))),
            "the ONE committed cascade ran under the settled basis"
        );

        // …and the animator has nothing to say about this node. Before the fix
        // it held a 300 ms `padding` track from 24 px to 21 px, and a layout
        // read at t=0 answered 24.
        let base = read_tree(|t| {
            t.node(card).and_then(|n| n.computed.as_ref()).map(|c| c.base.clone())
        })
        .expect("the card is cascaded");
        assert!(
            animation_overlay(card, &base, std::time::Instant::now()).is_none(),
            "a basis recompile is not a style change, so no transition runs across it"
        );
    }

    /// Two `<style>` elements concatenate in CREATION order, which stands in for
    /// document order — Vue creates a node before it inserts it, and a sheet
    /// appended later has to win at equal specificity.
    #[test]
    fn several_style_elements_concatenate_in_creation_order() {
        let opacity = |id: u32| {
            read_tree(|t| t.node(id).and_then(|n| n.computed.as_ref()).and_then(|c| c.base.opacity))
        };
        set_stylesheet(String::new());
        let probe = with_tree(|t| {
            let root = t.create_root();
            let div = t.intern("div");
            let probe = t.create_element(div);
            t.set_class(probe, "probe");
            t.insert(probe, root, 0);
            let style_tag = t.intern("style");
            for (i, css) in [".probe { opacity: 0.1 }", ".probe { opacity: 0.9 }"]
                .into_iter()
                .enumerate()
            {
                let sheet = t.create_element(style_tag);
                t.set_element_text(sheet, css.into());
                t.insert(sheet, root, 1 + i as u32);
            }
            probe
        });
        restyle_all();
        assert_eq!(opacity(probe), Some(0.9), "the later sheet wins at equal specificity");
    }
}
