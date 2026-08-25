//! The retained tree: a slab of nodes keyed by dense `u32` ids (0 = none),
//! mutated only from the VM thread via the `__vn_*` host functions.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;

pub const EV_CLICK: u32 = 1;

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
    /// Bitmask of EV_* the JS side listens for.
    pub listeners: u32,
    pub(crate) inline_dirty: bool,
    pub(crate) inline_cache: Option<std::rc::Rc<gpui::StyleRefinement>>,
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
            listeners: 0,
            inline_dirty: false,
            inline_cache: None,
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
    pub fn restyle(&mut self, engine: Option<&vn_css::Engine>) {
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
            let parsed = (!body.is_empty()).then(|| std::rc::Rc::new(vn_css::parse_inline(&body, &mut diags)));
            for d in diags {
                eprintln!("[css inline] {d}");
            }
            let n = self.node_mut(id).unwrap();
            n.inline_cache = parsed;
            n.inline_dirty = false;
        }

        // 2. Cascade (immutable walk), then store.
        let computed: Vec<(u32, vn_css::ComputedStates)> = {
            let this: &Tree = self;
            (1..this.nodes.len() as u32)
                .filter_map(|id| {
                    let n = this.node(id)?;
                    if n.kind != Kind::Element {
                        return None;
                    }
                    let inline = n.inline_cache.as_deref();
                    let states = match engine {
                        Some(e) => e.compute(NodeRef { tree: this, id }, inline),
                        None => vn_css::ComputedStates {
                            base: inline
                                .map(|r| std::rc::Rc::new(r.clone()))
                                .unwrap_or_default(),
                            hover: None,
                            active: None,
                        },
                    };
                    Some((id, states))
                })
                .collect()
        };
        for (id, states) in computed {
            if let Some(n) = self.node_mut(id) {
                n.computed = Some(states);
            }
        }
    }
}

// ---------------------------------------------------------------------------
// cascade wiring
// ---------------------------------------------------------------------------

thread_local! {
    static ENGINE: RefCell<Option<vn_css::Engine>> = const { RefCell::new(None) };
}

pub fn set_engine(engine: vn_css::Engine) {
    for d in &engine.diagnostics {
        eprintln!("[css] {d}");
    }
    ENGINE.with(|c| *c.borrow_mut() = Some(engine));
}

/// Restyles under the current engine (or inline-only when none is set).
pub fn restyle_all() {
    ENGINE.with(|eng| {
        let eng = eng.borrow();
        with_tree(|t| t.restyle(eng.as_ref()));
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
    fn is_empty(&self) -> bool {
        self.node().children.is_empty() && self.node().text.is_empty()
    }
    fn is_root(&self) -> bool {
        self.node().parent == 0
    }
}
