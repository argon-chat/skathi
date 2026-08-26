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
    pub listeners: u32,
    pub(crate) inline_dirty: bool,
    /// Everything the inline `style` body contributes: the mapped refinement,
    /// its transitions and `object-fit`, plus the `--x` declarations and
    /// `var()`-composed values that can only be resolved once the node's place
    /// in the tree is known.
    pub(crate) inline: Option<std::rc::Rc<vn_css::InlineStyle>>,
    /// The folded cascade result, filled by the restyle pass.
    pub computed: Option<vn_css::ComputedStates>,
}

#[derive(Default)]
pub struct Tree {
    nodes: Vec<Option<Node>>,
    free: Vec<u32>,
    pub root: u32,
    dirty: bool,
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

/// One line per distinct CSS advisory, whatever how many elements hit it.
fn warn_css(msg: &str) {
    let fresh = CSS_WARNED.with(|w| w.borrow_mut().insert(msg.to_string()));
    if fresh {
        eprintln!("{msg}");
        crate::log::record("css", msg.to_string());
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

    fn node_mut(&mut self, id: u32) -> Option<&mut Node> {
        self.nodes.get_mut(id as usize).and_then(Option::as_mut)
    }

    fn fresh(&mut self, kind: Kind, tag: u32) -> u32 {
        self.dirty = true;
        self.alloc(Node {
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
        })
    }

    pub fn create_element(&mut self, tag: u32) -> u32 {
        self.fresh(Kind::Element, tag)
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
    }

    /// Insert-or-move `child` under `parent`, before `anchor` (0 = append).
    pub fn insert(&mut self, child: u32, parent: u32, anchor: u32) {
        if child == 0 || parent == 0 || child == anchor {
            return;
        }
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
        self.detach(id);
        self.free_subtree(id);
        self.dirty = true;
    }

    fn free_subtree(&mut self, id: u32) {
        let Some(node) = self.nodes.get_mut(id as usize).and_then(Option::take) else {
            return;
        };
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
            }
        }
    }

    pub fn set_class(&mut self, id: u32, classes: &str) {
        if let Some(n) = self.node_mut(id) {
            n.classes = classes.split_whitespace().map(str::to_string).collect();
            self.dirty = true;
        }
    }

    pub fn set_listener(&mut self, id: u32, event: u32, on: bool) {
        if let Some(n) = self.node_mut(id) {
            let bit = 1u32 << event;
            if on {
                n.listeners |= bit;
            } else {
                n.listeners &= !bit;
            }
            self.dirty = true;
        }
    }

    pub fn take_dirty(&mut self) -> bool {
        std::mem::take(&mut self.dirty)
    }

    pub fn mark_dirty(&mut self) {
        self.dirty = true;
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

    fn restyle_with(&mut self, engine: &vn_css::Engine, env: &vn_css::Env) {
        // 1. Re-parse changed inline styles.
        let dirty_inline: Vec<u32> = (1..self.nodes.len() as u32)
            .filter(|&id| self.node(id).is_some_and(|n| n.inline_dirty))
            .collect();
        for id in dirty_inline {
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
        let computed: Vec<(u32, vn_css::ComputedStates)> = {
            let this: &Tree = self;
            let root_vars = std::rc::Rc::new(vn_css::Vars::default());
            let mut out = Vec::new();
            let mut roots: Vec<u32> = Vec::new();
            if this.root != 0 && this.node(this.root).is_some() {
                roots.push(this.root);
            }
            roots.extend(
                (1..this.nodes.len() as u32)
                    .filter(|&id| id != this.root)
                    .filter(|&id| this.node(id).is_some_and(|n| n.parent == 0)),
            );
            let mut stack: Vec<(u32, std::rc::Rc<vn_css::Vars>)> = Vec::new();
            for r in roots {
                stack.push((r, root_vars.clone()));
                while let Some((id, parent_vars)) = stack.pop() {
                    let Some(n) = this.node(id) else { continue };
                    // Text and anchor nodes have no cascade of their own; they
                    // simply pass the map down to whatever hangs under them.
                    let child_vars = if n.kind == Kind::Element {
                        let states = engine.compute_node(
                            NodeRef { tree: this, id },
                            n.inline.as_deref(),
                            &parent_vars,
                            env,
                            &mut diagnostics,
                        );
                        let vars = states.vars.clone();
                        out.push((id, states));
                        vars
                    } else {
                        parent_vars
                    };
                    for &c in n.children.iter().rev() {
                        stack.push((c, child_vars.clone()));
                    }
                }
            }
            out
        };
        for d in diagnostics {
            warn_css(&format!("[css] {d}"));
        }
        // 3. Store + feed the transition animator. ONE Instant for the whole
        // pass (tracks started together must not drift), change-gated via
        // ComputedStates: PartialEq.
        let now = std::time::Instant::now();
        ANIMATOR.with(|a| {
            let mut animator = a.borrow_mut();
            for (id, states) in computed {
                if let Some(n) = self.node_mut(id) {
                    if n.computed.as_ref() != Some(&states) {
                        animator.on_computed(id, n.computed.as_ref(), &states, now);
                        n.computed = Some(states);
                    }
                }
            }
            for d in animator.take_diagnostics() {
                eprintln!("[css transition] {d}");
                crate::log::record("css", format!("[css transition] {d}"));
            }
        });
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
    static CSS_WARNED: RefCell<std::collections::HashSet<String>> =
        RefCell::new(std::collections::HashSet::new());
    static ENGINE: RefCell<Option<vn_css::Engine>> = const { RefCell::new(None) };
    static ANIMATOR: RefCell<vn_css::Animator> = RefCell::new(vn_css::Animator::new());
    /// The `@media` environment the cascade is currently evaluated against —
    /// the native stand-in for a browser's viewport + OS preferences. The host
    /// refreshes it every frame from the window (see [`set_env`]).
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
    set_reduce_motion(env.reduce_motion);
    ENV.with(|e| {
        if e.get() == env {
            return false;
        }
        e.set(env);
        true
    })
}

pub fn env() -> vn_css::Env {
    ENV.with(|e| e.get())
}

/// The per-frame interpolated overlay for a node (None = idle).
pub fn animation_overlay(id: u32, now: std::time::Instant) -> Option<gpui::StyleRefinement> {
    ANIMATOR.with(|a| a.borrow().overlay(id, now))
}

/// True while any transition runs — the host keeps repainting. Also drops
/// settled tracks (visually a no-op: a finished track equals the base).
pub fn animating() -> bool {
    let now = std::time::Instant::now();
    ANIMATOR.with(|a| {
        let mut a = a.borrow_mut();
        a.finish_settled(now);
        a.any_active(now)
    })
}

pub fn animator_gc() {
    ANIMATOR.with(|a| {
        a.borrow_mut()
            .gc(&|id| read_tree(|t| t.node(id).is_some()));
    });
}

pub fn set_reduce_motion(on: bool) {
    ANIMATOR.with(|a| a.borrow_mut().set_reduce_motion(on));
}

pub fn set_engine(engine: vn_css::Engine) {
    // A new sheet is a new set of gaps: an advisory suppressed against the old
    // one must be free to speak again (this is the HMR path).
    CSS_WARNED.with(|w| w.borrow_mut().clear());
    for d in &engine.diagnostics {
        eprintln!("[css] {d}");
        crate::log::record("css", format!("[css] {d}"));
    }
    ENGINE.with(|c| *c.borrow_mut() = Some(engine));
}

/// Restyles under the current engine (or inline-only when none is set).
pub fn restyle_all() {
    let env = env();
    ENGINE.with(|eng| {
        let eng = eng.borrow();
        with_tree(|t| t.restyle(eng.as_ref(), &env));
    });
}

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
    fn has_tag(&self, name: &str) -> bool {
        self.tree.name(self.node().tag) == name
    }
    fn tag_eq(&self, other: &Self) -> bool {
        self.node().tag == other.node().tag
    }
    fn has_class(&self, name: &str) -> bool {
        self.node().classes.iter().any(|c| c == name)
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
