//! The retained tree: a slab of nodes keyed by dense `u32` ids (0 = none), mutated only from the VM
//! thread via the `__sk_*` host functions.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;

// Event codes — the JS side (EV_CODES in packages/runtime/src/runtime/event-names.ts) must stay in lockstep.
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
// `<lottie>` events, named after lottie-web's.
pub const EV_COMPLETE: u32 = 11;
pub const EV_LOOPCOMPLETE: u32 = 12;
pub const EV_ENTERFRAME: u32 = 13;
pub const EV_MOUSEMOVE: u32 = 14;
/// Non-bubbling, per the UI Events spec: fires only on the element entered.
pub const EV_MOUSEENTER: u32 = 15;
pub const EV_MOUSELEAVE: u32 = 16;
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
/// Throttled to the spec's 250 ms.
pub const EV_TIMEUPDATE: u32 = 27;
pub const EV_DURATIONCHANGE: u32 = 28;
pub const EV_RATECHANGE: u32 = 29;
pub const EV_VOLUMECHANGE: u32 = 30;
pub const EV_WAITING: u32 = 31;
pub const EV_ENDED: u32 = 32;
/// Carries `MediaError.code` in `a` and the advisory text in `key`.
pub const EV_ERROR: u32 = 33;
pub const EV_EMPTIED: u32 = 34;

pub const EV_TRANSITIONEND: u32 = 35;
pub const EV_ANIMATIONSTART: u32 = 36;
pub const EV_ANIMATIONEND: u32 = 37;
pub const EV_ANIMATIONCANCEL: u32 = 38;

pub const EV_PASTE: u32 = 39;
pub const EV_COPY: u32 = 40;
pub const EV_CUT: u32 = 41;

// (`dblclick` has no code: the JS runtime derives it from a `click` with `detail` 2.)
/// A MouseEvent; also from the Menu key / Shift+F10 at the focused element.
pub const EV_CONTEXTMENU: u32 = 42;
/// `a`/`b` = naturalWidth/naturalHeight, `key` = the `src`. A failure is [`EV_ERROR`] with the same
/// `key`.
pub const EV_LOAD: u32 = 43;
/// The window lost activation mid-press: only the press's `mousedown` target hears it.
pub const EV_POINTERCANCEL: u32 = 44;
/// `selectstart` fires at the element under a press that starts a selection (cancelable, bubbles);
/// every change owes `document` one `selectionchange`, delivered from the frame tick.
pub const EV_SELECTSTART: u32 = 45;
pub const EV_SELECTIONCHANGE: u32 = 46;
/// Middle or right button pressed and released over one element; the target is the nearest common
/// ancestor.
pub const EV_AUXCLICK: u32 = 47;
/// Before each edit, cancelable (except the IME's): a cancelled one makes no edit and no `input`.
pub const EV_BEFOREINPUT: u32 = 48;
/// The IME composition's stages (CompositionEvents, `data` as above): start, each pre-edit, and the
/// end, which follows the commit's `input`.
pub const EV_COMPOSITIONSTART: u32 = 49;
pub const EV_COMPOSITIONUPDATE: u32 = 50;
pub const EV_COMPOSITIONEND: u32 = 51;
/// An `<iframe>`'s document posted to its parent (`crate::iframe`): at the iframe's node, `key` =
/// `{"o": origin, "d": wire}`. The runtime fires it at `window`.
pub const EV_MESSAGE: u32 = 52;
/// The document's pointer lock changed (`a` = the request it settles, 0: none) or a request failed (`key` =
/// "Name: message"). Delivered at node 0; the runtime fires both at `document`.
pub const EV_POINTERLOCKCHANGE: u32 = 53;
pub const EV_POINTERLOCKERROR: u32 = 54;
/// `<sk-surface>` (`crate::surface`): its painted size changed (`a` × `b` physical px, `c` scale),
/// it showed its first frame (`a` × `b` the frame), the frame went (`key` = "cleared" | "device"),
/// or its display changed (`a` headroom, `b` potential headroom, `c` refresh Hz, `d` SDR white
/// nits, 0 unknown; `key` = "hdr"? then "vrr" | "fixed"?, space-separated).
pub const EV_SURFACERESIZE: u32 = 55;
pub const EV_SURFACEREADY: u32 = 56;
pub const EV_SURFACELOST: u32 = 57;
pub const EV_SURFACEDISPLAYCHANGE: u32 = 58;
/// The pointer moved onto (`over`) or off (`out`) the deepest element under it; they bubble. `key` is
/// the other element's node id (`relatedTarget`), empty for none.
pub const EV_MOUSEOVER: u32 = 59;
pub const EV_MOUSEOUT: u32 = 60;
pub const EV_GESTURESTART: u32 = 61;
pub const EV_GESTURECHANGE: u32 = 62;
pub const EV_GESTUREEND: u32 = 63;

/// Every host event code with the name JS knows it by; devtools lists from here.
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
    (EV_PASTE, "paste"),
    (EV_COPY, "copy"),
    (EV_CUT, "cut"),
    (EV_CONTEXTMENU, "contextmenu"),
    (EV_LOAD, "load"),
    (EV_POINTERCANCEL, "pointercancel"),
    (EV_SELECTSTART, "selectstart"),
    (EV_SELECTIONCHANGE, "selectionchange"),
    (EV_AUXCLICK, "auxclick"),
    (EV_BEFOREINPUT, "beforeinput"),
    (EV_COMPOSITIONSTART, "compositionstart"),
    (EV_COMPOSITIONUPDATE, "compositionupdate"),
    (EV_COMPOSITIONEND, "compositionend"),
    (EV_MESSAGE, "message"),
    (EV_POINTERLOCKCHANGE, "pointerlockchange"),
    (EV_POINTERLOCKERROR, "pointerlockerror"),
    (EV_SURFACERESIZE, "surfaceresize"),
    (EV_SURFACEREADY, "surfaceready"),
    (EV_SURFACELOST, "surfacelost"),
    (EV_SURFACEDISPLAYCHANGE, "surfacedisplaychange"),
    (EV_MOUSEOVER, "mouseover"),
    (EV_MOUSEOUT, "mouseout"),
    (EV_GESTURESTART, "gesturestart"),
    (EV_GESTURECHANGE, "gesturechange"),
    (EV_GESTUREEND, "gestureend"),
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Element,
    Text,
    Anchor,
}

pub struct Node {
    pub kind: Kind,
    pub tag: u32,
    pub parent: u32,
    pub children: Vec<u32>,
    /// Node text, or inline text (exclusive with children) for an element via `setElementText`.
    pub text: String,
    /// (interned property, raw value), last write wins; parsed lazily into `inline_cache`.
    pub style: Vec<(u32, String)>,
    pub classes: Vec<String>,
    /// As written, for attribute selectors; `classes` is the token list.
    pub class_attr: String,
    /// (interned name, value, write counter).
    /// The counter ticks on every assignment, even of the same string, so a native widget can tell
    /// a re-forced controlled input from an untouched prop ([`Tree::prop_write`]).
    pub props: Vec<(u32, String, u32)>,
    /// Vue `<style scoped>` hashes; a child component's root carries its own and its parent's.
    pub scopes: Vec<String>,
    pub listeners: u64,
    pub(crate) inline_dirty: bool,
    /// Whether the element has a `style` attribute: set by any inline write, cleared only by removing it.
    pub style_present: bool,
    /// The attribute text as `setAttribute("style", …)` wrote it, until the next inline write.
    pub style_raw: Option<String>,
    /// Bumped on the node and every ancestor by [`Tree::touch`]. Upwards only: a mutation cannot
    /// reach outside its subtree except through the cascade, which touches changed nodes itself.
    pub(crate) sub_rev: u64,
    /// Restyle generation in which this node's own cascade inputs changed; see
    /// [`Tree::mark_restyle`].
    pub(crate) mark_gen: u32,
    pub(crate) desc_gen: u32,
    /// Which other nodes decide how this one and its children match ([`sel`]).
    pub(crate) sel_flags: Cell<u8>,
    pub(crate) inline: Option<std::rc::Rc<sk_css::InlineStyle>>,
    pub computed: Option<sk_css::ComputedStates>,
}

/// [`Node::sel_flags`]: dependencies the cascade records while it matches, like Servo's
/// `ElementSelectorFlags`.
/// Learned from the matcher's questions, not the sheet. Sibling flags live on the parent. A
/// question about the cascaded element is an EDGE dependence (`:first-child`, `+`); about another
/// node, a WALK (`:nth-child()`, `~`) that restyles the whole run.
pub(crate) mod sel {
    pub const PREV_EDGE: u8 = 1;
    pub const PREV_WALK: u8 = 2;
    pub const NEXT_EDGE: u8 = 4;
    pub const NEXT_WALK: u8 = 8;
    pub const SIBLINGS: u8 = PREV_EDGE | PREV_WALK | NEXT_EDGE | NEXT_WALK;
    pub const EMPTY: u8 = 16;
    pub const HAS_SELF: u8 = 32;
    pub const HAS_DESC: u8 = 64;
    pub const HAS: u8 = HAS_SELF | HAS_DESC;
    pub const HAS_NEXT: u8 = 128;
}

thread_local! {
    static CASCADE_SUBJECT: Cell<u32> = const { Cell::new(0) };
    /// Depth inside [`NodeRef`]'s `each_child`: the outermost call is the `:has()` anchor's
    /// question.
    static HAS_DEPTH: Cell<u32> = const { Cell::new(0) };
    static READ_ON_OTHERS: RefCell<ReadOnOthers> = RefCell::new(ReadOnOthers::default());
    /// Names [`READ_ON_OTHERS`] already holds, keyed by the address and length of the selector's
    /// string: a hash per ask would cost more than the restyles it saves.
    static READ_CACHE: [Cell<(usize, usize, u32)>; 128] = [const { Cell::new((0, 0, 0)) }; 128];
}

/// Every class, attribute and scope name the cascade has tested on an element other than the one
/// being cascaded, with the tags of the askers; likewise an ancestor's place among its siblings
/// ([`NameKind::Position`]).
/// Only a change to one of these can move another element's match, so a change restyles narrowly.
/// Growing only.
#[derive(Default)]
struct ReadOnOthers {
    names: [HashMap<Box<str>, Readers>; 4],
}

#[derive(Default)]
struct Readers {
    tags: Vec<u32>,
    all: bool,
}

/// Beyond this many reader tags a narrow walk stops paying.
const MAX_READER_TAGS: usize = 8;

#[derive(Clone, Copy)]
enum NameKind {
    Class = 0,
    Attr = 1,
    Scope = 2,
    /// An element's place among siblings, asked of one that is neither the cascaded element nor its
    /// sibling: `.row:first-child .cell`.
    Position = 3,
}

const POSITION: &str = "position";

#[derive(Debug, PartialEq)]
enum Reach {
    Own,
    /// Elements with these tags in the node's subtree (and, with `+` / `~`, its later siblings').
    Tags(Vec<u32>),
    Subtree,
}

fn note_name_read(asked: u32, subject: u32, subject_tag: u32, name: &str, kind: NameKind) {
    if subject == 0 || subject == asked {
        return;
    }
    let key = (
        name.as_ptr() as usize,
        name.len() * 4 + kind as usize,
        subject_tag,
    );
    let slot = ((key.0 >> 3) ^ key.2 as usize) & 127;
    if READ_CACHE.with(|c| c[slot].get()) == key {
        return;
    }
    READ_ON_OTHERS.with(|r| {
        let mut r = r.borrow_mut();
        let map = &mut r.names[kind as usize];
        let readers = match map.get_mut(name) {
            Some(readers) => readers,
            None => map.entry(name.into()).or_default(),
        };
        if !readers.all && !readers.tags.contains(&subject_tag) {
            readers.tags.push(subject_tag);
            if readers.tags.len() > MAX_READER_TAGS {
                readers.all = true;
                readers.tags = Vec::new();
            }
        }
    });
    READ_CACHE.with(|c| c[slot].set(key));
}

fn reach_of<'a>(names: impl IntoIterator<Item = (&'a str, NameKind)>) -> Reach {
    READ_ON_OTHERS.with(|r| {
        let r = r.borrow();
        let mut tags: Vec<u32> = Vec::new();
        for (name, kind) in names {
            let Some(readers) = r.names[kind as usize].get(name) else {
                continue;
            };
            if readers.all {
                return Reach::Subtree;
            }
            for &t in &readers.tags {
                if !tags.contains(&t) {
                    tags.push(t);
                }
            }
        }
        match tags.len() {
            0 => Reach::Own,
            n if n > MAX_READER_TAGS => Reach::Subtree,
            _ => Reach::Tags(tags),
        }
    })
}

#[derive(Default, Debug, Clone)]
pub struct TreeMem {
    pub slots: usize,
    pub nodes: usize,
    pub elements: usize,
    pub texts: usize,
    pub anchors: usize,
    pub computed: usize,
    pub slot_bytes: usize,
    pub owned_bytes: usize,
    pub computed_bytes: usize,
    pub refinements: usize,
    pub refinement_bytes: usize,
    pub vars_maps: usize,
    pub var_entries: usize,
    pub vars_bytes: usize,
    pub inlines: usize,
    pub inline_bytes: usize,
    pub names: usize,
    pub total_bytes: usize,
}

#[derive(Default)]
pub struct Tree {
    nodes: Vec<Option<Node>>,
    free: Vec<u32>,
    freed: Vec<u32>,
    pub root: u32,
    dirty: bool,
    restyle_gen: u32,
    /// Parentless non-root nodes marked this generation: created but not yet inserted, or a
    /// `<KeepAlive>` cache container.
    detached_marks: Vec<u32>,
    has_anchors: Cell<bool>,
    self_marks: Vec<u32>,
    sibling_combinators: bool,
    sibling_positions: RefCell<HashMap<u32, HashMap<u32, u32>>>,
    /// One counter for the tree, not per slot: a removed-then-set prop would restart at 0 and the
    /// same string would read as unassigned.
    prop_writes: u32,
    dirty_all: bool,
    inline_dirty_nodes: Vec<u32>,
    style_flushed: bool,
    /// An inline `style` has used a `cq*` unit (sticky): the cascade then tracks query containers
    /// whatever the sheet says.
    inline_container_units: bool,
    /// A set because `sync_runtime_sheets` runs on every dirtying flush and a slab scan is too
    /// costly.
    style_nodes: std::collections::BTreeSet<u32>,
    /// Elements some other element's `group-hover:` rule watches; the renderer gives them gpui's
    /// `.group(name)`. Recomputed at the end of a pass: a scoped restyle can recompute the
    /// descendant without the ancestor.
    group_anchors: std::collections::HashSet<u32>,
    /// Anchor → elements whose `group-hover:` rung watches it. Rungs are folded at build time, so a
    /// flip on the anchor must re-render cached subtrees ([`Tree::touch_hover_styled`]).
    group_hover_deps: HashMap<u32, Vec<u32>>,
    /// Anchor → elements whose `.a:focus .b`-style rules watch it ([`Tree::touch_focus_watchers`]).
    group_focus_deps: HashMap<u32, Vec<u32>>,
    generated_hover: HashMap<u32, Vec<u32>>,
    generated_focus: HashMap<u32, Vec<u32>>,
    custom_watchers: HashMap<(u32, sk_css::Interaction), std::collections::HashSet<u32>>,
    custom_marks: std::collections::HashSet<u32>,
    names: Vec<String>,
    name_ids: HashMap<String, u32>,
}

thread_local! {
    static TREE: RefCell<Tree> = RefCell::new(Tree::new());
    static DISPATCHER: Cell<u64> = const { Cell::new(0) };
}

/// Runs `f` with the tree. Panics (caught by `host_fn!`, poisons) if it is already borrowed, i.e.
/// JS mutating during a draw walk.
pub fn with_tree<R>(f: impl FnOnce(&mut Tree) -> R) -> R {
    TREE.with(|t| f(&mut t.borrow_mut()))
}

pub fn read_tree<R>(f: impl FnOnce(&Tree) -> R) -> R {
    TREE.with(|t| f(&t.borrow()))
}

/// One line per distinct CSS advisory; a recompile of the same source repeats none.
fn warn_css(msg: &str) {
    // Normalised before the dedupe, so two spellings of one message are one line.
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
            nodes: vec![None],
            // Name 0 is reserved for `tag: 0` (text, anchors); otherwise the interner gave it to
            // "root" and devtools showed every text node as `<root>`.
            names: vec![String::new()],
            name_ids: HashMap::from([(String::new(), 0)]),
            restyle_gen: 1,
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
        self.names
            .get(id as usize)
            .map(String::as_str)
            .unwrap_or("")
    }

    pub fn is_group_anchor(&self, id: u32) -> bool {
        self.group_anchors.contains(&id)
    }

    fn update_custom_watchers(
        &mut self,
        id: u32,
        old: &[(u64, sk_css::Interaction)],
        new: &[(u64, sk_css::Interaction)],
    ) {
        if old == new {
            return;
        }
        for &(anchor, state) in old {
            let key = (anchor as u32, state);
            if let Some(watchers) = self.custom_watchers.get_mut(&key) {
                watchers.remove(&id);
                if watchers.is_empty() {
                    self.custom_watchers.remove(&key);
                }
            }
        }
        for &(anchor, state) in new {
            self.custom_watchers
                .entry((anchor as u32, state))
                .or_default()
                .insert(id);
        }
    }

    fn mark_custom_watchers(&mut self, anchor: u32, state: sk_css::Interaction) -> bool {
        if let Some(watchers) = self.custom_watchers.get(&(anchor, state)) {
            self.custom_marks.extend(watchers);
            self.dirty = true;
            return true;
        }
        false
    }

    pub(crate) fn mark_custom_path(&mut self, mut id: u32, root: u32, state: sk_css::Interaction) {
        if self.custom_watchers.is_empty() {
            return;
        }
        while id != 0 {
            self.mark_custom_watchers(id, state);
            if id == root {
                break;
            }
            id = self.node(id).map_or(0, |n| n.parent);
        }
    }

    /// `id` joined or left the hover path: touch its `:hover` rung and the `group-hover:` watchers
    /// so cached subtrees render again.
    pub(crate) fn touch_hover_styled(&mut self, id: u32) -> bool {
        let own = self
            .node(id)
            .and_then(|n| n.computed.as_ref())
            .is_some_and(|cs| cs.hover.is_some() || cs.control_state(1));
        if own {
            self.touch(id);
        }
        let deps = self.group_hover_deps.get(&id).cloned().unwrap_or_default();
        for &dep in &deps {
            self.touch(dep);
        }
        let generated = self.generated_hover.get(&id).cloned().unwrap_or_default();
        for &dep in &generated {
            self.mark_restyle_subtree(dep);
        }
        self.mark_custom_watchers(id, sk_css::Interaction::Hover)
            || own
            || !deps.is_empty()
            || !generated.is_empty()
    }

    /// Focus moved from `old` to `new`: re-render the elements whose ancestor-focus rules watch
    /// either one or, for `:focus-within`, an element above either, and the elements above either
    /// with a `:focus-within` rule of their own.
    pub(crate) fn touch_focus_watchers(&mut self, old: u32, new: u32) {
        for anchor in [old, new] {
            self.mark_custom_watchers(anchor, sk_css::Interaction::Focus);
            self.mark_custom_watchers(anchor, sk_css::Interaction::FocusVisible);
            self.mark_custom_path(anchor, 0, sk_css::Interaction::FocusWithin);
            if let Some(deps) = self.generated_focus.get(&anchor).cloned() {
                for dep in deps {
                    self.mark_restyle_subtree(dep);
                }
            }
        }
        let mut deps = Vec::new();
        for start in [old, new] {
            let mut at = start;
            for _ in 0..4096 {
                let Some(n) = self.node(at) else { break };
                if let Some(watchers) = self.group_focus_deps.get(&at) {
                    deps.extend_from_slice(watchers);
                }
                if n.computed
                    .as_ref()
                    .is_some_and(|cs| cs.focus_within.is_some())
                {
                    deps.push(at);
                }
                if n.parent == 0 || n.parent == at {
                    break;
                }
                at = n.parent;
            }
        }
        for dep in deps {
            self.touch(dep);
        }
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

    /// Whether `id` is in the document: its parent chain reaches the root.
    pub(crate) fn connected(&self, id: u32) -> bool {
        let mut cursor = id;
        // Bounded, so a corrupt parent chain cannot hang a frame.
        for _ in 0..4096 {
            if cursor == 0 {
                return false;
            }
            if cursor == self.root {
                return true;
            }
            match self.node(cursor) {
                Some(n) => cursor = n.parent,
                None => return false,
            }
        }
        false
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
            class_attr: String::new(),
            props: Vec::new(),
            scopes: Vec::new(),
            listeners: 0,
            inline_dirty: false,
            style_present: false,
            style_raw: None,
            sub_rev: 0,
            mark_gen: 0,
            desc_gen: 0,
            sel_flags: Cell::new(0),
            inline: None,
            computed: None,
        });
        self.mark_restyle(id);
        id
    }

    pub fn create_element(&mut self, tag: u32) -> u32 {
        let id = self.fresh(Kind::Element, tag);
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
            // `:empty` reads whether a text child has data, so a text node gaining its first or
            // losing its last character restyles its element, looking through anchors.
            let flips = n.kind == Kind::Text && n.text.is_empty() != text.is_empty();
            let mut parent = n.parent;
            n.text = text;
            self.dirty = true;
            self.mark_restyle(id);
            if flips {
                while let Some(p) = self.node(parent).filter(|p| p.kind == Kind::Anchor) {
                    parent = p.parent;
                }
                if self
                    .node(parent)
                    .is_some_and(|p| p.sel_flags.get() & sel::EMPTY != 0)
                {
                    self.mark_with_followers(parent);
                }
                self.touch_mutated(id, false);
                return;
            }
            self.touch(id);
        }
    }

    pub fn set_element_text(&mut self, id: u32, text: String) {
        let children = match self.node_mut(id) {
            Some(n) => std::mem::take(&mut n.children),
            None => return,
        };
        self.sibling_positions.get_mut().remove(&id);
        for c in children {
            self.free_subtree(c);
        }
        if let Some(n) = self.node_mut(id) {
            n.text = text;
        }
        self.dirty = true;
        if self
            .node(id)
            .is_some_and(|n| n.sel_flags.get() & sel::EMPTY != 0)
        {
            self.mark_with_followers(id);
        } else {
            self.mark_restyle(id);
        }
        self.touch_mutated(id, true);
    }

    /// Insert-or-move `child` under `parent`, before `anchor` (0 = append).
    /// Siblings are marked only if the cascade read the child list ([`Tree::child_list_changed`]).
    pub fn insert(&mut self, child: u32, parent: u32, anchor: u32) {
        if child == 0 || parent == 0 || child == anchor || self.node(parent).is_none() {
            return;
        }
        let old_parent = self.node(child).map(|n| n.parent).unwrap_or(0);
        if old_parent != 0 {
            let was_bare = self.bare_if_asked(old_parent);
            let at = self.detach(child);
            self.child_list_changed(old_parent, at, false, was_bare);
        }
        let was_bare = self.bare_if_asked(parent);
        // An element with inline text gaining a real child: the inline text is stale
        // (setElementText always precedes), so drop it.
        let idx = {
            let Some(p) = self.node_mut(parent) else {
                return;
            };
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
        self.mark_restyle(child);
        if self
            .node(child)
            .is_some_and(|c| c.desc_gen == self.restyle_gen)
        {
            self.flag_path(parent);
        }
        self.child_list_changed(parent, idx, true, was_bare);
        if self
            .node(child)
            .is_some_and(|node| node.kind == Kind::Element)
        {
            self.sibling_values_changed(parent);
            if old_parent != 0 && old_parent != parent {
                self.sibling_values_changed(old_parent);
            }
        }
        self.dirty = true;
        // Both chains: a cache boundary over either parent has a stale recording.
        if old_parent != 0 {
            self.touch_mutated(old_parent, true);
        }
        self.touch_mutated(child, false);
    }

    fn detach(&mut self, id: u32) -> usize {
        let parent = self.node(id).map(|n| n.parent).unwrap_or(0);
        let mut at = 0;
        if parent != 0 {
            if let Some(p) = self.node_mut(parent) {
                if let Some(i) = p.children.iter().position(|&c| c == id) {
                    p.children.remove(i);
                    at = i;
                }
            }
            if let Some(n) = self.node_mut(id) {
                n.parent = 0;
            }
        }
        at
    }

    /// Vue calls remove once per unmounted subtree root: destroys native state and recycles every
    /// id.
    pub fn remove(&mut self, id: u32) {
        let parent = self.node(id).map(|n| n.parent).unwrap_or(0);
        let element = self.node(id).is_some_and(|node| node.kind == Kind::Element);
        let was_bare = self.bare_if_asked(parent);
        let at = self.detach(id);
        self.free_subtree(id);
        if parent != 0 {
            self.child_list_changed(parent, at, false, was_bare);
            if element {
                self.sibling_values_changed(parent);
            }
        }
        self.dirty = true;
        if parent != 0 {
            self.touch_mutated(parent, true);
        }
    }

    fn element_children(&self, parent: u32) -> impl Iterator<Item = u32> + '_ {
        self.node(parent)
            .into_iter()
            .flat_map(|node| node.children.iter().copied())
            .filter(|&child| {
                self.node(child)
                    .is_some_and(|node| node.kind == Kind::Element)
            })
    }

    fn sibling_values_changed(&mut self, parent: u32) {
        if !self.sibling_positions.get_mut().contains_key(&parent) {
            return;
        }
        let positions: HashMap<_, _> = self
            .element_children(parent)
            .enumerate()
            .map(|(index, child)| (child, index as u32 + 1))
            .collect();
        let count = positions.len() as u32;
        let marks: Vec<_> = positions
            .iter()
            .filter_map(|(&child, &index)| {
                let values = self.node(child)?.computed.as_ref()?.sibling_values;
                (values.index.is_some_and(|old| old != index)
                    || values.count.is_some_and(|old| old != count))
                .then_some(child)
            })
            .collect();
        self.sibling_positions.get_mut().insert(parent, positions);
        for child in marks {
            self.mark_restyle(child);
        }
    }

    /// `parent`'s child list changed at `at`. Marks only what the cascade recorded as depending on
    /// it ([`Node::sel_flags`]), not whole sibling runs.
    fn child_list_changed(&mut self, parent: u32, at: usize, inserted: bool, was_bare: bool) {
        let Some(p) = self.node(parent) else { return };
        let flags = p.sel_flags.get();
        if flags & sel::EMPTY != 0 && self.is_bare(parent) != was_bare {
            self.mark_with_followers(parent);
        }
        if flags & sel::SIBLINGS == 0 {
            return;
        }
        let marks: Vec<u32> = {
            let children = &self.node(parent).map_or(&[][..], |p| &p.children[..]);
            let is_element = |c: &u32| self.node(*c).is_some_and(|n| n.kind == Kind::Element);
            let after = if inserted { at + 1 } else { at };
            let mut marks = Vec::new();
            if flags & sel::PREV_WALK != 0 {
                marks.extend(children.iter().skip(after).copied());
            } else if flags & sel::PREV_EDGE != 0 {
                marks.extend(children.iter().skip(after).find(|c| is_element(c)).copied());
            }
            if flags & sel::NEXT_WALK != 0 {
                marks.extend(children.iter().take(at).copied());
            } else if flags & sel::NEXT_EDGE != 0 {
                marks.extend(
                    children
                        .iter()
                        .take(at)
                        .rev()
                        .find(|c| is_element(c))
                        .copied(),
                );
            }
            marks
        };
        if marks.is_empty() {
            return;
        }
        let reach = reach_of([(POSITION, NameKind::Position)]);
        for c in marks {
            self.mark_reach(c, &reach, false);
        }
    }

    /// [`Tree::is_bare`] for a node whose cascade asked `:empty`, else `false`; spares a child-list
    /// walk on every insert.
    fn bare_if_asked(&self, id: u32) -> bool {
        self.node(id)
            .is_some_and(|n| n.sel_flags.get() & sel::EMPTY != 0)
            && self.is_bare(id)
    }

    fn is_bare(&self, id: u32) -> bool {
        let Some(n) = self.node(id) else { return true };
        n.text.is_empty()
            && n.children.iter().all(|&c| match self.node(c) {
                Some(child) => match child.kind {
                    Kind::Element => false,
                    Kind::Text => child.text.is_empty(),
                    Kind::Anchor => self.is_bare(c),
                },
                None => true,
            })
    }

    fn free_subtree(&mut self, id: u32) {
        self.sibling_positions.get_mut().remove(&id);
        let Some(node) = self.nodes.get_mut(id as usize).and_then(Option::take) else {
            return;
        };
        self.style_nodes.remove(&id);
        self.custom_marks.remove(&id);
        if let Some(deps) = node
            .computed
            .as_ref()
            .and_then(|c| c.custom_state_dependencies.as_deref())
        {
            self.update_custom_watchers(id, deps, &[]);
        }
        for c in node.children {
            self.free_subtree(c);
        }
        if self.root == id {
            self.root = 0;
        }
        self.freed.push(id);
    }

    pub(crate) fn hidden_above(&self, id: u32) -> bool {
        let mut cur = self.node(id).map_or(0, |n| n.parent);
        while cur != 0 {
            let Some(n) = self.node(cur) else {
                return false;
            };
            if n.computed
                .as_ref()
                .is_some_and(|c| c.base.display == Some(gpui::Display::None))
            {
                return true;
            }
            cur = n.parent;
        }
        false
    }

    /// Hands freed ids back to [`Tree::alloc`]. Not reused until the side tables keyed by node id
    /// are swept: a reallocated id inherited the old node's animator, rect and focus state.
    pub(crate) fn release_freed(&mut self) {
        self.free.append(&mut self.freed);
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
            self.mark_self(id);
            self.touch(id);
        }
    }

    pub fn set_prop(&mut self, id: u32, key: u32, value: Option<String>) {
        let reach = reach_of([(self.name(key), NameKind::Attr)]);
        self.prop_writes = self.prop_writes.wrapping_add(1);
        let write = self.prop_writes;
        if let Some(n) = self.node_mut(id) {
            match value {
                None => n.props.retain(|(k, _, _)| *k != key),
                Some(v) => {
                    if let Some(slot) = n.props.iter_mut().find(|(k, _, _)| *k == key) {
                        slot.1 = v;
                        slot.2 = write;
                    } else {
                        n.props.push((key, v, write));
                    }
                }
            }
            self.dirty = true;
            self.mark_own_inputs(id, reach);
            self.touch_mutated(id, false);
        }
    }

    pub fn prop<'a>(&'a self, id: u32, name: &str) -> Option<&'a str> {
        self.prop_write(id, name).map(|(v, _)| v)
    }

    /// The value with its write counter: consumers that react to assignments rather than value
    /// changes (a controlled `<input>`) key off the counter.
    pub fn prop_write<'a>(&'a self, id: u32, name: &str) -> Option<(&'a str, u32)> {
        let key = *self.name_ids.get(name)?;
        self.node(id)?
            .props
            .iter()
            .find(|(k, _, _)| *k == key)
            .map(|(_, v, write)| (v.as_str(), *write))
    }

    pub fn add_scope(&mut self, id: u32, scope: String) {
        let reach = reach_of([(scope.as_str(), NameKind::Scope)]);
        if let Some(n) = self.node_mut(id) {
            if !n.scopes.contains(&scope) {
                n.scopes.push(scope);
                self.dirty = true;
                self.mark_own_inputs(id, reach);
                self.touch_mutated(id, false);
            }
        }
    }

    /// `cloneNode()`: scoped-style ids are attributes in the DOM, so a copy carries them.
    pub fn copy_scopes(&mut self, from: u32, to: u32) {
        let scopes = self
            .node(from)
            .map(|n| n.scopes.clone())
            .unwrap_or_default();
        for scope in scopes {
            self.add_scope(to, scope);
        }
    }

    pub fn set_class(&mut self, id: u32, classes: &str) {
        if let Some(n) = self.node_mut(id) {
            if n.class_attr == classes {
                return; // `el.className = el.className`: nothing moved.
            }
            // An ordered set like `classList`; repeats are dropped (a duplicate would fold its
            // rules twice). The raw string stays in `class_attr`.
            let mut next: Vec<String> = Vec::new();
            for c in classes.split_whitespace() {
                if !next.iter().any(|x| x == c) {
                    next.push(c.to_string());
                }
            }
            let reach = {
                let gone = n.classes.iter().filter(|c| !next.contains(c));
                let came = next.iter().filter(|c| !n.classes.contains(c));
                let names = gone.chain(came).map(|c| (c.as_str(), NameKind::Class));
                reach_of(std::iter::once(("class", NameKind::Attr)).chain(names))
            };
            n.classes = next;
            n.class_attr = classes.to_string();
            self.dirty = true;
            self.mark_own_inputs(id, reach);
            self.touch_mutated(id, false);
        }
    }

    /// A class, attribute or scope of `id` changed; `reach` says whose match can have moved besides
    /// its own.
    fn mark_own_inputs(&mut self, id: u32, reach: Reach) {
        self.mark_reach(id, &reach, self.sibling_combinators);
    }

    fn mark_reach(&mut self, id: u32, reach: &Reach, later_siblings: bool) {
        let tags = match reach {
            Reach::Subtree if later_siblings => return self.mark_with_followers(id),
            Reach::Subtree => return self.mark_restyle(id),
            Reach::Own => return self.mark_self(id),
            Reach::Tags(tags) => tags,
        };
        self.mark_self(id);
        let mut stack: Vec<u32> = self
            .node(id)
            .map(|n| n.children.clone())
            .unwrap_or_default();
        if later_siblings {
            if let Some(siblings) = self
                .node(id)
                .and_then(|n| self.node(n.parent))
                .map(|p| &p.children)
            {
                if let Some(at) = siblings.iter().position(|&c| c == id) {
                    stack.extend_from_slice(&siblings[at + 1..]);
                }
            }
        }
        let mut hits: Vec<u32> = Vec::new();
        while let Some(c) = stack.pop() {
            let Some(n) = self.node(c) else { continue };
            if n.kind == Kind::Element && tags.contains(&n.tag) {
                hits.push(c);
                continue;
            }
            stack.extend_from_slice(&n.children);
        }
        for c in hits {
            self.mark_restyle(c);
        }
    }

    fn mark_self(&mut self, id: u32) {
        if self.dirty_all || id == 0 || self.node(id).is_none_or(|n| n.mark_gen == self.restyle_gen)
        {
            return;
        }
        self.self_marks.push(id);
    }

    pub fn set_listener(&mut self, id: u32, event: u32, on: bool) {
        if let Some(n) = self.node_mut(id) {
            // `event` is a code, not a bit index: a JS mirror ahead of this build would overflow
            // the shift.
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
            self.touch(id);
        }
    }

    pub fn is_dirty(&self) -> bool {
        self.dirty || self.style_flushed
    }

    pub fn take_dirty(&mut self) -> bool {
        // A forced flush already cascaded, but the frame must still be told something changed.
        let flushed = std::mem::take(&mut self.style_flushed);
        std::mem::take(&mut self.dirty) || flushed
    }

    /// Live nodes: allocated slots that still hold a node.
    pub fn live_count(&self) -> u32 {
        self.nodes.iter().filter(|n| n.is_some()).count() as u32
    }

    pub(crate) fn mem_estimate(&self) -> TreeMem {
        use std::collections::HashSet;
        use std::mem::size_of;
        let mut m = TreeMem {
            slots: self.nodes.len(),
            slot_bytes: self.nodes.capacity() * size_of::<Option<Node>>(),
            ..TreeMem::default()
        };
        let mut refinements: HashSet<*const gpui::StyleRefinement> = HashSet::new();
        let mut vars: HashSet<*const sk_css::Vars> = HashSet::new();
        let mut inlines: HashSet<*const sk_css::InlineStyle> = HashSet::new();
        let note_ref = |r: &std::rc::Rc<gpui::StyleRefinement>,
                        set: &mut HashSet<*const gpui::StyleRefinement>| {
            set.insert(std::rc::Rc::as_ptr(r));
        };
        for n in self.nodes.iter().flatten() {
            m.nodes += 1;
            match n.kind {
                Kind::Element => m.elements += 1,
                Kind::Text => m.texts += 1,
                Kind::Anchor => m.anchors += 1,
            }
            let mut own = n.children.capacity() * 4 + n.text.capacity() + n.class_attr.capacity();
            own += n.style.capacity() * size_of::<(u32, String)>()
                + n.style.iter().map(|(_, v)| v.capacity()).sum::<usize>();
            own += n.classes.capacity() * size_of::<String>()
                + n.classes.iter().map(|c| c.capacity()).sum::<usize>();
            own += n.props.capacity() * size_of::<(u32, String, u32)>()
                + n.props.iter().map(|(_, v, _)| v.capacity()).sum::<usize>();
            own += n.scopes.capacity() * size_of::<String>()
                + n.scopes.iter().map(|s| s.capacity()).sum::<usize>();
            m.owned_bytes += own;
            if let Some(i) = &n.inline {
                inlines.insert(std::rc::Rc::as_ptr(i));
            }
            if let Some(cs) = &n.computed {
                m.computed += 1;
                note_ref(&cs.base, &mut refinements);
                for r in [
                    &cs.hover,
                    &cs.active,
                    &cs.focus,
                    &cs.focus_visible,
                    &cs.focus_within,
                    &cs.placeholder,
                    &cs.selection,
                    &cs.file_selector_button,
                ]
                .into_iter()
                .flatten()
                {
                    note_ref(r, &mut refinements);
                }
                for (_, r) in [&cs.group_hover, &cs.group_active].into_iter().flatten() {
                    note_ref(r, &mut refinements);
                }
                if let Some(g) = &cs.group_focus {
                    for (_, r) in [&g.focus, &g.focus_visible, &g.focus_within]
                        .into_iter()
                        .flatten()
                    {
                        note_ref(r, &mut refinements);
                    }
                }
                if vars.insert(std::rc::Rc::as_ptr(&cs.vars)) {
                    m.var_entries += cs.vars.len();
                }
            }
        }
        m.computed_bytes = m.computed * size_of::<sk_css::ComputedStates>();
        m.refinements = refinements.len();
        m.refinement_bytes = refinements.len() * (size_of::<gpui::StyleRefinement>() + 16);
        m.vars_maps = vars.len();
        // Strings are shared down the tree and not counted.
        m.vars_bytes = vars.len() * (size_of::<sk_css::Vars>() + 16)
            + m.var_entries * 2 * size_of::<std::rc::Rc<str>>();
        m.inlines = inlines.len();
        m.inline_bytes = inlines.len() * (size_of::<sk_css::InlineStyle>() + 16);
        m.names = self.names.len();
        m.total_bytes = m.slot_bytes
            + m.owned_bytes
            + m.computed_bytes
            + m.refinement_bytes
            + m.vars_bytes
            + m.inline_bytes;
        m
    }

    /// Marks one subtree for re-cascade, the scoped [`mark_dirty`]. A pop-out resize changes
    /// `@media` answers only for the subtree it paints.
    pub fn mark_restyle_subtree(&mut self, id: u32) {
        self.dirty = true;
        self.mark_restyle(id);
    }

    pub fn mark_dirty(&mut self) {
        self.dirty = true;
        self.dirty_all = true;
    }

    /// Bumps [`Node::sub_rev`] on `id` and every ancestor. Not change-gated: a mutator that stored
    /// the same string still moved the DOM for `MutationObserver`.
    pub(crate) fn touch(&mut self, id: u32) {
        let mut cur = id;
        // A cycle cannot happen (`insert` detaches first), but a corrupt id must not hang the pump.
        for _ in 0..4096 {
            let Some(n) = self.node_mut(cur) else { return };
            n.sub_rev = n.sub_rev.wrapping_add(1);
            match n.parent {
                0 => return,
                p => cur = p,
            }
        }
    }

    /// [`Tree::touch`] plus a flush, for native state the tree does not hold (an editor's text).
    pub(crate) fn touch_repaint(&mut self, id: u32) {
        self.touch(id);
        self.dirty = true;
    }

    /// [`Tree::touch`] for a mutation that can change selector matching: the same walk also tells
    /// every `:has()` anchor above.
    /// One read by a descendant's selector ([`sel::HAS_DESC`]) is marked whole; one only its own
    /// match reads ([`sel::HAS_SELF`]) is rechecked alone. `id` counts as an anchor only for
    /// child-list changes.
    pub(crate) fn touch_mutated(&mut self, id: u32, include_self: bool) {
        if !self.has_anchors.get() {
            self.touch(id);
            return;
        }
        let mut whole: Vec<u32> = Vec::new();
        let mut cur = id;
        let mut first = true;
        for _ in 0..4096 {
            let Some(n) = self.node_mut(cur) else { break };
            n.sub_rev = n.sub_rev.wrapping_add(1);
            let flags = n.sel_flags.get();
            let parent = n.parent;
            if include_self || !first {
                if flags & sel::HAS_DESC != 0 {
                    whole.push(cur);
                } else if flags & sel::HAS_SELF != 0 {
                    self.self_marks.push(cur);
                }
            }
            first = false;
            // A `:has(+ …)` / `:has(~ …)` anchor among the earlier siblings reads this node and
            // what is under it.
            let earlier: Vec<u32> = match self.node(parent) {
                Some(p) if p.sel_flags.get() & sel::HAS_NEXT != 0 => p
                    .children
                    .iter()
                    .take_while(|&&c| c != cur)
                    .copied()
                    .collect(),
                _ => Vec::new(),
            };
            for sibling in earlier {
                let flags = self.node(sibling).map_or(0, |n| n.sel_flags.get());
                if flags & sel::HAS_DESC != 0 {
                    whole.push(sibling);
                } else if flags & sel::HAS_SELF != 0 {
                    self.self_marks.push(sibling);
                }
            }
            match parent {
                0 => break,
                p => cur = p,
            }
        }
        for a in whole {
            self.mark_with_followers(a);
        }
    }

    /// `0` for a missing node, which is also a never-touched node's revision: callers must hold a
    /// live node.
    pub fn sub_rev(&self, id: u32) -> u64 {
        self.node(id).map(|n| n.sub_rev).unwrap_or(0)
    }

    /// Walk this element's whole subtree next pass: it was inserted or moved, what it hands its
    /// children moved, or too many elements under it read a change.
    /// A mark is two numbers ([`Node::mark_gen`], and [`Node::desc_gen`] up the ancestors), so N
    /// marks cost O(N) plus one path to the root; there is no cap and no whole-document fallback.
    fn mark_restyle(&mut self, id: u32) {
        if self.dirty_all || id == 0 {
            return;
        }
        let gen = self.restyle_gen;
        match self.node_mut(id) {
            Some(n) if n.mark_gen != gen => n.mark_gen = gen,
            _ => return,
        }
        self.flag_path(id);
    }

    /// [`Tree::mark_restyle`] for `id` and, when the sheet has `+` / `~`, its later siblings: for
    /// changes the cascade does not say who read.
    fn mark_with_followers(&mut self, id: u32) {
        self.mark_restyle(id);
        if !self.sibling_combinators {
            return;
        }
        let later: Vec<u32> = self
            .node(id)
            .and_then(|n| self.node(n.parent))
            .map(|p| {
                p.children
                    .iter()
                    .skip_while(|&&c| c != id)
                    .skip(1)
                    .copied()
                    .collect()
            })
            .unwrap_or_default();
        for s in later {
            self.mark_restyle(s);
        }
    }

    /// Lays this generation's trail from `id` up, stopping at the first node that has it: every
    /// trail is laid all the way up ([`Tree::insert`] re-lays it on a move).
    fn flag_path(&mut self, id: u32) {
        let gen = self.restyle_gen;
        let root = self.root;
        let mut cur = id;
        // A cycle cannot happen (`insert` detaches first), but a corrupt id must not hang the pump.
        for _ in 0..4096 {
            let Some(n) = self.node_mut(cur) else { return };
            if n.desc_gen == gen {
                return;
            }
            n.desc_gen = gen;
            match n.parent {
                0 => {
                    if cur != root {
                        self.detached_marks.push(cur);
                    }
                    return;
                }
                p => cur = p,
            }
        }
    }

    /// The viewport basis moved: re-parse every inline `style` that names a viewport unit and
    /// re-cascade its subtree.
    /// sk-css flattens `vw`/`vh` to px at parse time, and an inline body parses only when its text
    /// changes.
    pub(crate) fn invalidate_viewport_inline(&mut self) -> usize {
        self.invalidate_inline_where(has_viewport_unit)
    }

    pub(crate) fn invalidate_rem_inline(&mut self) -> usize {
        self.invalidate_inline_where(has_rem_unit)
    }

    fn invalidate_inline_where(&mut self, names_the_unit: fn(&str) -> bool) -> usize {
        let hits: Vec<u32> = self
            .nodes
            .iter()
            .enumerate()
            .filter_map(|(i, n)| {
                let n = n.as_ref()?;
                n.style
                    .iter()
                    .any(|(_, v)| names_the_unit(v))
                    .then_some(i as u32)
            })
            .collect();
        for &id in &hits {
            if let Some(n) = self.node_mut(id) {
                n.inline_dirty = true;
            }
            self.inline_dirty_nodes.push(id);
            self.mark_restyle(id);
            self.touch(id);
        }
        if !hits.is_empty() {
            self.dirty = true;
        }
        hits.len()
    }

    /// A web font was registered: re-parse inline `font-family`/`font` bodies, since sk-css picks
    /// the family at parse time from the fonts installed then.
    pub(crate) fn invalidate_font_inline(&mut self) -> usize {
        self.invalidate_inline_named(&["font", "font-family"])
    }

    /// Re-parses every inline `style` that declares one of `names`.
    fn invalidate_inline_named(&mut self, names: &[&str]) -> usize {
        let keys: Vec<u32> = names
            .iter()
            .filter_map(|k| self.name_ids.get(*k).copied())
            .collect();
        if keys.is_empty() {
            return 0;
        }
        let hits: Vec<u32> = self
            .nodes
            .iter()
            .enumerate()
            .filter_map(|(i, n)| {
                let n = n.as_ref()?;
                n.style
                    .iter()
                    .any(|(k, _)| keys.contains(k))
                    .then_some(i as u32)
            })
            .collect();
        for &id in &hits {
            if let Some(n) = self.node_mut(id) {
                n.inline_dirty = true;
            }
            self.inline_dirty_nodes.push(id);
            self.mark_restyle(id);
            self.touch(id);
        }
        if !hits.is_empty() {
            self.dirty = true;
        }
        hits.len()
    }

    /// Runs once per JS wave that touched the tree, never during a draw.
    pub fn restyle(&mut self, engine: Option<&sk_css::Engine>, env: &sk_css::Env) {
        match engine {
            Some(e) => self.restyle_with(e, env),
            // No sheet before the first `__sk_set_stylesheet`: run against an empty one (inline
            // styles and custom properties still cascade) rather than a divergent path.
            None => NO_SHEET.with(|e| self.restyle_with(e, env)),
        }
    }

    /// The subtree roots this pass walks, none overlapping, so each inherits from a parent whose
    /// computed map is current. A marked node is a root; an element never cascaded, or a
    /// text/anchor node, promotes the root to itself.
    fn restyle_roots(&mut self) -> Vec<u32> {
        let gen = self.restyle_gen;
        self.restyle_gen = self.restyle_gen.wrapping_add(1).max(1);
        let detached = std::mem::take(&mut self.detached_marks);
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

        let mut roots: Vec<u32> = Vec::new();
        // Most `detached` entries were inserted since; those are found from the root along the
        // trail.
        let mut tops: Vec<u32> = Vec::new();
        for top in std::iter::once(self.root).chain(detached) {
            let Some(n) = self.node(top) else { continue };
            if n.desc_gen != gen || (top != self.root && n.parent != 0) || tops.contains(&top) {
                continue;
            }
            tops.push(top);
        }
        for top in tops {
            if self.collect_roots(top, gen, &mut roots, 0) {
                roots.push(top);
            }
        }
        roots
    }

    fn collect_roots(&self, id: u32, gen: u32, roots: &mut Vec<u32>, depth: u32) -> bool {
        let Some(n) = self.node(id) else { return false };
        if n.mark_gen == gen || (n.kind == Kind::Element && n.computed.is_none()) || depth > 1024 {
            return true;
        }
        let start = roots.len();
        let element = n.kind == Kind::Element;
        for &c in &n.children {
            let Some(child) = self.node(c) else { continue };
            let whole = child.desc_gen == gen && self.collect_roots(c, gen, roots, depth + 1);
            if !whole {
                continue;
            }
            if !element {
                // A walk inherits from its root's parent; text and anchor nodes have no computed
                // map.
                roots.truncate(start);
                return true;
            }
            roots.push(c);
        }
        false
    }

    /// The root element's computed `font-size` in px, resolved without committing a cascade: `rem`
    /// is flattened at sheet-compile time, and a committed cascade would make every `transition`
    /// element animate from the old geometry.
    pub(crate) fn probe_root_font_size(
        &self,
        engine: Option<&sk_css::Engine>,
        env: &sk_css::Env,
    ) -> Option<f32> {
        let engine = engine?;
        let n = self.node(self.root)?;
        let parsed;
        let inline: Option<&sk_css::InlineStyle> = if n.inline_dirty {
            let body = n
                .style
                .iter()
                .map(|(k, v)| format!("{}: {}", self.name(*k), v))
                .collect::<Vec<_>>()
                .join("; ");
            parsed = (!body.is_empty())
                .then(|| sk_css::parse_inline_style(&body, &mut Vec::new()))
                .filter(|s| !s.is_empty());
            parsed.as_ref()
        } else {
            n.inline.as_deref()
        };
        let states = engine.compute_node_at(
            NodeRef {
                tree: self,
                id: self.root,
            },
            inline,
            &std::rc::Rc::new(sk_css::Vars::default()),
            None,
            ROOT_PARENT_FONT_SIZE,
            env,
            &mut Vec::new(),
        );
        match states.base.text.font_size? {
            gpui::AbsoluteLength::Pixels(p) => Some(f32::from(p)),
            // `Rems` cannot reach here (the mapper flattens); resolving against the current basis
            // would never move, so take the value as is.
            gpui::AbsoluteLength::Rems(r) => Some(r.0 * sk_css::rem_basis()),
        }
    }

    /// What a walk starting at `r` inherits from the nearest ancestor with a computed map: custom
    /// properties, `pointer-events: none`, `color`, corner radii, and the `@media` environment.
    #[allow(clippy::type_complexity)]
    fn walk_inputs(
        &self,
        r: u32,
        env: &sk_css::Env,
        root_vars: &std::rc::Rc<sk_css::Vars>,
    ) -> (
        std::rc::Rc<sk_css::Vars>,
        bool,
        Inherited,
        sk_css::InheritedRadii,
        sk_css::Env,
    ) {
        let root_env = crate::windows::env_for_node(self, r, env);
        let mut cur = self.node(r).map_or(0, |n| n.parent);
        let mut parent_computed = None;
        for _ in 0..4096 {
            let Some(p) = self.node(cur) else { break };
            if let Some(c) = p.computed.as_ref() {
                parent_computed = Some(c);
                break;
            }
            if p.kind == Kind::Element {
                break;
            }
            cur = p.parent;
        }
        let inherited = parent_computed
            .map(|c| c.vars.clone())
            .unwrap_or_else(|| root_vars.clone());
        let inherited_inert =
            parent_computed.is_some_and(|c| c.extras.pointer_events_none == Some(true));
        // `currentColor` resolves against the computed `color`, inherited when none is declared, so
        // the effective colour rides down beside the vars map.
        let inherited_text = Inherited {
            color: parent_computed.and_then(|c| c.color),
            font_size: parent_computed.map_or(ROOT_PARENT_FONT_SIZE, |c| c.font_size),
            font: parent_computed
                .map(|c| c.computed_font.clone())
                .unwrap_or_default(),
        };
        let inherited_radii = parent_computed
            .map(|c| sk_css::InheritedRadii::of(&c.base))
            .unwrap_or_default();
        (
            inherited,
            inherited_inert,
            inherited_text,
            inherited_radii,
            root_env,
        )
    }

    fn cascade_one(
        &self,
        engine: &sk_css::Engine,
        id: u32,
        (parent_vars, parent_inert, parent_text, parent_radii): (
            &std::rc::Rc<sk_css::Vars>,
            bool,
            Inherited,
            sk_css::InheritedRadii,
        ),
        env: &sk_css::Env,
        chain: &sk_css::container::Chain,
        diagnostics: &mut Vec<String>,
    ) -> (sk_css::ComputedStates, sk_css::InheritedRadii) {
        let n = self.node(id).expect("cascade_one on a live node");
        CASCADE_SUBJECT.with(|c| c.set(id));
        let mut states = sk_css::container::with_chain(chain, || {
            engine.compute_node_with_font(
                NodeRef { tree: self, id },
                n.inline.as_deref(),
                parent_vars,
                parent_text.color,
                parent_text.font_size,
                &parent_text.font,
                env,
                diagnostics,
            )
        });
        CASCADE_SUBJECT.with(|c| c.set(0));
        crate::containers::note_percentage_style(self, id, states.percentage_sizing);
        // `None` = nothing declared, so inherit; an explicit `pointer-events: auto` opts a
        // descendant back in (reka's `data-[state=open]:pointer-events-auto` overlay).
        let inert = states.extras.pointer_events_none.unwrap_or(parent_inert);
        states.extras.pointer_events_none = Some(inert);
        let radii = sk_css::resolve_inherited_radii(&mut states, parent_radii);
        if let Some(starting) = &mut states.starting_style {
            sk_css::resolve_inherited_radii(std::rc::Rc::make_mut(starting), parent_radii);
        }
        (states, radii)
    }

    /// The query containers above `id`, from the committed styles of its ancestors; `None` when
    /// nothing in the sheet can ask.
    fn container_chain(&self, id: u32, active: bool) -> sk_css::container::Chain {
        if !active {
            return None;
        }
        let mut above = Vec::new();
        let mut cur = self.node(id).map_or(0, |n| n.parent);
        for _ in 0..4096 {
            let Some(n) = self.node(cur) else { break };
            if n.computed.is_some() {
                above.push(cur);
            }
            cur = n.parent;
        }
        let mut chain = None;
        for &a in above.iter().rev() {
            if let Some(cs) = self.node(a).and_then(|n| n.computed.as_ref()) {
                chain = sk_css::container::Container::link(
                    a,
                    &cs.extras,
                    &cs.vars,
                    cs.font_size,
                    &chain,
                );
            }
        }
        chain
    }

    fn containers_active(&self, engine: &sk_css::Engine) -> bool {
        engine.uses_containers() || self.inline_container_units
    }

    /// The nodes whose own style alone may have moved ([`Tree::mark_self`]). A result that changes
    /// what children inherit becomes an ordinary subtree mark, so a Vue `<Transition>` on a
    /// page-sized container does not re-cascade the page per phase.
    fn settle_self_marks(
        &mut self,
        engine: &sk_css::Engine,
        env: &sk_css::Env,
    ) -> Vec<(u32, sk_css::ComputedStates)> {
        let mut marks = std::mem::take(&mut self.self_marks);
        if marks.is_empty() || self.dirty_all {
            return Vec::new();
        }
        marks.sort_unstable();
        marks.dedup();
        let root_vars = std::rc::Rc::new(sk_css::Vars::default());
        let mut diagnostics = Vec::new();
        let mut stored = Vec::new();
        let mut whole = Vec::new();
        let containers = self.containers_active(engine);
        for id in marks {
            let Some(n) = self.node(id) else { continue };
            if n.kind != Kind::Element || n.mark_gen == self.restyle_gen || n.computed.is_none() {
                continue;
            }
            let (vars, inert, text, radii, here_env) = self.walk_inputs(id, env, &root_vars);
            let (states, child_radii) = self.cascade_one(
                engine,
                id,
                (&vars, inert, text, radii),
                &here_env,
                &self.container_chain(id, containers),
                &mut diagnostics,
            );
            let Some(old) = n.computed.as_ref() else {
                continue;
            };
            if *old == states {
                continue;
            }
            let hands_down_the_same = old.vars == states.vars
                && old.color == states.color
                && old.font_size == states.font_size
                && old.computed_font == states.computed_font
                && old.extras.pointer_events_none == states.extras.pointer_events_none
                && old.extras.container_type == states.extras.container_type
                && old.extras.container_name == states.extras.container_name
                && sk_css::InheritedRadii::of(&old.base) == child_radii;
            if hands_down_the_same {
                stored.push((id, states));
            } else {
                whole.push(id);
            }
        }
        for id in whole {
            self.mark_restyle(id);
        }
        stored
    }

    fn restyle_with(&mut self, engine: &sk_css::Engine, env: &sk_css::Env) {
        let _profile = sk_runtime::redjs::profiling::span("style", "Restyle DOM");
        // [`READ_CACHE`] is keyed by this sheet's string addresses; a pass against a different
        // engine restarts it.
        thread_local! { static CACHE_ENGINE: Cell<usize> = const { Cell::new(0) }; }
        let addr = engine as *const sk_css::Engine as usize;
        if CACHE_ENGINE.with(|c| c.replace(addr)) != addr {
            forget_read_cache();
            note_parent_keys(engine);
        }
        let sheet_changed = REBIND_ANIMATIONS.with(|pending| pending.replace(false));
        self.sibling_combinators = engine.uses_sibling_combinators();
        let pass = crate::perf::Span::start();
        let phase = crate::perf::Span::start();
        // Document order is required: custom properties inherit, so a parent is computed before its
        // children.
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
                let (style, reads_layout) = sk_css::container::noting_units(|| {
                    sk_css::parse_inline_style(&body, &mut diags)
                });
                self.inline_container_units |= reads_layout || body.contains("cq");
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

        let mut diagnostics: Vec<String> = Vec::new();
        let full = self.dirty_all;
        let containers = self.containers_active(engine);
        crate::containers::install();
        if full {
            crate::containers::forget_all();
        }
        let settled = self.settle_self_marks(engine, env);
        let mut roots = self.restyle_roots();
        let custom_marks = std::mem::take(&mut self.custom_marks);
        let selective = !custom_marks.is_empty() && !full && roots.is_empty() && settled.is_empty();
        let mut marked_paths = std::collections::HashSet::new();
        if !custom_marks.is_empty() && !full {
            for &id in roots.iter().chain(&custom_marks) {
                self.mark_restyle(id);
                if selective {
                    let mut at = id;
                    while at != 0 && marked_paths.insert(at) {
                        at = self.node(at).map_or(0, |n| n.parent);
                    }
                }
            }
            roots = self.restyle_roots();
        }
        let cascaded_anything = !roots.is_empty() || !settled.is_empty();
        let mut forwarded = Vec::new();
        let mut container_matches_changed = false;
        let computed: Vec<(u32, sk_css::ComputedStates)> = {
            let this: &Tree = self;
            let root_vars = std::rc::Rc::new(sk_css::Vars::default());
            let mut out = Vec::new();
            // `(node, inherited custom properties, an ancestor has `pointer-events: none`)`. It
            // inherits so an invisible `pointer-events-none` tab panel stacked over the visible one
            // is inert all the way down.
            let mut stack: Vec<(
                u32,
                std::rc::Rc<sk_css::Vars>,
                bool,
                Inherited,
                sk_css::InheritedRadii,
                sk_css::Env,
                sk_css::container::Chain,
                CascadeChange,
            )> = Vec::new();
            let multiwindow = crate::windows::any_secondary();
            for r in roots {
                // An element hangs under exactly one painted root, and each root belongs to one
                // window; that window's `@media` environment applies.
                let (inherited, inherited_inert, inherited_text, inherited_radii, root_env) =
                    this.walk_inputs(r, env, &root_vars);
                stack.push((
                    r,
                    inherited,
                    inherited_inert,
                    inherited_text,
                    inherited_radii,
                    root_env,
                    this.container_chain(r, containers),
                    if selective {
                        CascadeChange::Interaction
                    } else {
                        CascadeChange::All
                    },
                ));
                while let Some((
                    id,
                    parent_vars,
                    parent_inert,
                    parent_text,
                    parent_radii,
                    here_env,
                    chain,
                    delta,
                )) = stack.pop()
                {
                    let Some(n) = this.node(id) else { continue };
                    if delta
                        .variables()
                        .is_some_and(|(_, changes)| changes.is_empty())
                        && !marked_paths.contains(&id)
                    {
                        continue;
                    }
                    let (
                        child_vars,
                        child_inert,
                        child_text,
                        child_radii,
                        child_chain,
                        child_delta,
                    ) = if n.kind == Kind::Element {
                        let previous = n.computed.as_ref();
                        let pass_through =
                            delta
                                .variables()
                                .zip(previous)
                                .filter(|((_, changes), old)| {
                                    !custom_marks.contains(&id)
                                        && !engine.reads_changed_vars(old, changes)
                                        && !n.style.iter().any(|(k, value)| {
                                            changes.affects_declaration(this.name(*k), value)
                                        })
                                });
                        let computed = pass_through.is_none().then(|| {
                            this.cascade_one(
                                engine,
                                id,
                                (
                                    &parent_vars,
                                    parent_inert,
                                    parent_text.clone(),
                                    parent_radii,
                                ),
                                &here_env,
                                &chain,
                                &mut diagnostics,
                            )
                        });
                        let (states, vars, radii, forwarded_changes) =
                            if let Some((states, radii)) = &computed {
                                (states, states.vars.clone(), *radii, None)
                            } else {
                                let ((old_parent, changes), old) = pass_through.unwrap();
                                let (vars, inherited) = engine.inherit_changed_vars(
                                    changes,
                                    &old.vars,
                                    old_parent,
                                    &parent_vars,
                                    &here_env,
                                );
                                if !std::rc::Rc::ptr_eq(&vars, &old.vars) {
                                    forwarded.push((id, vars.clone()));
                                }
                                (
                                    old,
                                    vars,
                                    sk_css::InheritedRadii::of(&old.base),
                                    Some((old.vars.clone(), inherited)),
                                )
                            };
                        let inert = states.extras.pointer_events_none == Some(true);
                        let text = Inherited {
                            color: states.color,
                            font_size: states.font_size,
                            font: states.computed_font.clone(),
                        };
                        let child_chain = if containers {
                            sk_css::container::Container::link(
                                id,
                                &states.extras,
                                &vars,
                                states.font_size,
                                &chain,
                            )
                        } else {
                            None
                        };
                        let child_delta = if let Some((prior, changes)) = forwarded_changes {
                            CascadeChange::Variables(prior, changes)
                        } else if let Some(old) =
                            previous.filter(|_| !matches!(delta, CascadeChange::All))
                        {
                            let changes =
                                std::rc::Rc::new(sk_css::VarChanges::between(&old.vars, &vars));
                            if containers
                                && (old.extras.container_type != states.extras.container_type
                                    || old.extras.container_name != states.extras.container_name
                                    || old.font_size != states.font_size
                                    || engine.container_reads_changed_vars(&changes))
                            {
                                container_matches_changed = true;
                                CascadeChange::All
                            } else if old.color == states.color
                                && old.font_size == states.font_size
                                && old.computed_font == states.computed_font
                                && old.extras.pointer_events_none
                                    == states.extras.pointer_events_none
                                && sk_css::InheritedRadii::of(&old.base) == radii
                            {
                                CascadeChange::Variables(old.vars.clone(), changes)
                            } else {
                                CascadeChange::Interaction
                            }
                        } else {
                            CascadeChange::All
                        };
                        if let Some((states, _)) = computed {
                            out.push((id, states));
                        }
                        (vars, inert, text, radii, child_chain, child_delta)
                    } else {
                        (
                            parent_vars,
                            parent_inert,
                            parent_text,
                            parent_radii,
                            chain,
                            delta,
                        )
                    };
                    for &c in n.children.iter().rev() {
                        // The window boundary: a popped-out node is painted by another window, so
                        // its subtree answers that window's environment, on a full-document pass
                        // too.
                        let child_env = if multiwindow {
                            crate::windows::env_at_root(c, env).unwrap_or(here_env)
                        } else {
                            here_env
                        };
                        stack.push((
                            c,
                            child_vars.clone(),
                            child_inert,
                            child_text.clone(),
                            child_radii,
                            child_env,
                            child_chain.clone(),
                            child_delta.clone(),
                        ));
                    }
                }
            }
            out
        };
        for (id, vars) in forwarded {
            if let Some(cs) = self.node_mut(id).and_then(|n| n.computed.as_mut()) {
                cs.vars = vars;
            }
        }
        // Nodes restyled alone, unless a walk this pass reached them too: that result uses what the
        // walk's ancestors hand down now.
        let computed = if settled.is_empty() {
            computed
        } else {
            let walked: std::collections::HashSet<u32> =
                computed.iter().map(|(id, _)| *id).collect();
            let mut computed = computed;
            computed.extend(settled.into_iter().filter(|(id, _)| !walked.contains(id)));
            computed
        };
        let cascade_ms = phase.ms();
        let cascaded = computed.len() as u64;
        let phase = crate::perf::Span::start();
        let track_layout = containers || crate::containers::watching();
        for d in diagnostics {
            warn_css(&format!("[css] {d}"));
        }
        let now = std::time::Instant::now();
        let has_keyframes = engine.has_keyframes();
        // `@keyframes` with `var()` resolve against the animating element, so their advisories
        // belong to it too; drained with the animator's own.
        let keyframe_diagnostics = std::cell::RefCell::new(Vec::<String>::new());
        let mut restyled: Vec<u32> = Vec::new();
        // The cascade switches environments at a painted root and this loop does not walk the tree,
        // so ask for the window again, only for a node about to start an animation and only when a
        // pop-out exists.
        let multiwindow = crate::windows::any_secondary();
        let mut display_flips: Vec<(u32, bool)> = Vec::new();
        ANIMATOR.with(|a| {
            let mut animator = a.borrow_mut();
            let mut revealed = std::collections::HashSet::new();
            for (id, states) in &computed {
                let previous = self.node(*id).and_then(|n| n.computed.as_ref());
                let was_none = animator
                    .display_at(*id, now)
                    .or_else(|| previous.and_then(|c| c.base.display))
                    == Some(gpui::Display::None);
                if was_none && states.base.display != Some(gpui::Display::None) {
                    revealed.insert(*id);
                }
            }
            for (id, states) in computed {
                let old_watchers = self
                    .node(id)
                    .and_then(|n| n.computed.as_ref())
                    .and_then(|s| s.custom_state_dependencies.clone());
                self.update_custom_watchers(
                    id,
                    old_watchers.as_deref().map_or(&[], Vec::as_slice),
                    states
                        .custom_state_dependencies
                        .as_deref()
                        .map_or(&[], Vec::as_slice),
                );
                let node_env = if multiwindow && (has_keyframes || !states.animations.is_empty()) {
                    crate::windows::env_for_node(self, id, env)
                } else {
                    *env
                };
                // A stylesheet replacement can change the referenced block without changing any
                // computed declaration.
                let rebind = (sheet_changed || node_env != *env) && !states.animations.is_empty();
                // Hidden: its runs are cancelled (`animationcancel`) and none start; showing it
                // again starts them afresh.
                let self_none = states.base.display == Some(gpui::Display::None);
                let animated = !states.animations.is_empty() || animator.has_animations(id);
                let hidden_above = self.hidden_above(id);
                let hidden = animated && (self_none || hidden_above);
                let under_reveal = if revealed.is_empty() {
                    false
                } else {
                    let mut parent = self.node(id).map_or(0, |node| node.parent);
                    let mut found = false;
                    while let Some(node) = self.node(parent) {
                        if revealed.contains(&parent) {
                            found = true;
                            break;
                        }
                        parent = node.parent;
                    }
                    found
                };
                let keywords = (animated
                    || !states.transitions.is_empty()
                    || animator.has_script_animations(id))
                    && crate::inherit::interpolate_keywords(
                        self,
                        id,
                        states.extras.interpolate_size,
                    );
                animator.set_interpolate_keywords(id, keywords);
                if let Some(n) = self.node_mut(id) {
                    let was_none = n
                        .computed
                        .as_ref()
                        .map(|c| c.base.display == Some(gpui::Display::None));
                    if was_none.is_some_and(|was| was != self_none) {
                        display_flips.push((id, self_none));
                    }
                    if hidden {
                        animator.cancel_animations(id, now);
                    }
                    if track_layout {
                        crate::containers::note_committed(
                            id,
                            n.parent,
                            n.computed.is_none(),
                            &states,
                        );
                    }
                    if n.computed.as_ref() != Some(&states) {
                        let before = if track_layout && crate::containers::born_this_frame(id) {
                            None
                        } else {
                            n.computed.as_ref()
                        };
                        if hidden_above {
                            animator.cancel_transitions(id);
                        } else if !under_reveal {
                            animator.on_computed(id, before, &states, now);
                            if self_none && animator.display_at(id, now).is_none() {
                                animator.cancel_transitions(id);
                            }
                        }
                        // Against the painting window's environment: `@keyframes` inside `@media`
                        // exist only where the query holds. They are document-scoped, so a sheet
                        // without any skips the check.
                        if !hidden
                            && (has_keyframes
                                || !states.animations.is_empty()
                                || animator.has_animations(id))
                        {
                            animator.on_computed_animations(id, &states, now, &|name| {
                                engine.keyframes_for_siblings(
                                    name,
                                    &states.vars,
                                    states.sibling_values,
                                    &node_env,
                                    &mut keyframe_diagnostics.borrow_mut(),
                                )
                            });
                        }
                        // A restyle under a stationary pointer: the flip already happened (or never
                        // will), so the animator would aim at the old fold.
                        if !hidden_above && !under_reveal && states.hover_transitions() {
                            let target = fold_hover_target(id, &states);
                            if target.is_some() || animator.holds_state(id) {
                                animator.on_state(id, &states, target.as_ref(), now);
                            }
                        }
                        n.computed = Some(states);
                        restyled.push(id);
                    } else if rebind && !hidden {
                        animator.on_computed_animations(id, &states, now, &|name| {
                            engine.keyframes_for_siblings(
                                name,
                                &states.vars,
                                states.sibling_values,
                                &node_env,
                                &mut keyframe_diagnostics.borrow_mut(),
                            )
                        });
                    }
                }
            }
            // A `display` flip reaches the whole subtree, including nodes not in this pass: hiding
            // cancels every run under it, showing starts those of every non-hidden descendant.
            let mut started = std::collections::HashSet::new();
            for (id, none) in std::mem::take(&mut display_flips) {
                if !none && self.hidden_above(id) {
                    continue; // shown, but still under a hidden ancestor
                }
                let mut stack: Vec<u32> = self
                    .node(id)
                    .map(|n| n.children.clone())
                    .unwrap_or_default();
                while let Some(c) = stack.pop() {
                    let Some(node) = self.node(c) else { continue };
                    if none {
                        animator.cancel_transitions(c);
                        if animator.has_animations(c) {
                            animator.cancel_animations(c, now);
                        }
                    } else {
                        let Some(cs) = node.computed.as_ref() else {
                            continue;
                        };
                        if cs.base.display == Some(gpui::Display::None) {
                            continue; // its subtree stays hidden
                        }
                        if revealed.contains(&id)
                            && cs.starting_style.is_some()
                            && started.insert(c)
                        {
                            animator.on_computed(c, None, cs, now);
                        }
                        if !cs.animations.is_empty() {
                            let node_env = if multiwindow {
                                crate::windows::env_for_node(self, c, env)
                            } else {
                                *env
                            };
                            animator.on_computed_animations(c, cs, now, &|name| {
                                engine.keyframes_for_siblings(
                                    name,
                                    &cs.vars,
                                    cs.sibling_values,
                                    &node_env,
                                    &mut keyframe_diagnostics.borrow_mut(),
                                )
                            });
                        }
                    }
                    stack.extend(node.children.iter().copied());
                }
            }
            for d in animator.take_diagnostics() {
                warn_css(&format!("[css transition] {d}"));
            }
        });
        for d in keyframe_diagnostics.into_inner() {
            warn_css(&format!("[css] {d}"));
        }
        crate::perf::note_restyle_shape(full, restyled.len() as u64);
        for id in restyled {
            self.touch(id);
        }
        if !cascaded_anything || (selective && !container_matches_changed) {
            crate::perf::note_restyle(pass.ms(), inline_ms, cascade_ms, phase.ms(), cascaded);
            return;
        }
        self.group_anchors.clear();
        self.group_hover_deps.clear();
        self.group_focus_deps.clear();
        self.generated_focus.clear();
        self.generated_hover.clear();
        if engine.tracks_state_dependencies() {
            for (i, slot) in self.nodes.iter().enumerate() {
                let Some(slot) = slot else { continue };
                if let Some(cs) = &slot.computed {
                    if let Some(deps) = &cs.generated_dependencies {
                        for &(key, state) in deps.iter() {
                            let map = if state == sk_css::Interaction::Hover {
                                &mut self.generated_hover
                            } else {
                                &mut self.generated_focus
                            };
                            map.entry(key as u32).or_default().push(i as u32);
                        }
                    }
                    if let Some((key, _)) = &cs.group_hover {
                        self.group_anchors.insert(*key as u32);
                        self.group_hover_deps
                            .entry(*key as u32)
                            .or_default()
                            .push(i as u32);
                    }
                    if let Some((key, _)) = &cs.group_active {
                        self.group_anchors.insert(*key as u32);
                    }
                    for key in cs.group_focus.iter().flat_map(|g| g.anchors()) {
                        self.group_focus_deps
                            .entry(key as u32)
                            .or_default()
                            .push(i as u32);
                    }
                }
            }
        }
        crate::perf::note_restyle(pass.ms(), inline_ms, cascade_ms, phase.ms(), cascaded);
    }
}

thread_local! {
    static NO_SHEET: sk_css::Engine = sk_css::Engine::compile("");
    static RUNTIME_CSS: RefCell<Vec<String>> = const { RefCell::new(Vec::new()) };
    static CSS_WARNED: RefCell<std::collections::HashSet<String>> =
        RefCell::new(std::collections::HashSet::new());
    static ENGINE: RefCell<Option<sk_css::Engine>> = const { RefCell::new(None) };
    /// A root whose own `font-size` is in `rem`, settled: the basis CSS gives it and the size it
    /// computes to under that basis, which must not be adopted as one ([`restyle_all`]).
    static REM_SETTLED: Cell<Option<(f32, f32)>> = const { Cell::new(None) };
    static SHEET_SOURCE: RefCell<String> = const { RefCell::new(String::new()) };
    /// `<style>` texts the runtime keeps for `document.head` (not tree nodes); they arrive with the
    /// sheet.
    static HEAD_SHEETS: RefCell<Vec<String>> = const { RefCell::new(Vec::new()) };
    static ANIMATOR: RefCell<sk_css::Animator> = RefCell::new(sk_css::Animator::new());
    static REBIND_ANIMATIONS: Cell<bool> = const { Cell::new(false) };
    /// The `@media` environment for everything the primary window paints. A pop-out's lives in its
    /// `WindowState`; this is also the fallback for a pop-out that has not painted, and
    /// `matchMedia` is global to the one JS realm.
    static ENV: Cell<sk_css::Env> = Cell::new(sk_css::Env::default());
}

/// Publishes the window environment. `true` when it changed: the caller must mark the tree dirty (a
/// resize or theme flip can toggle any `@media` rule).
pub fn set_env(env: sk_css::Env) -> bool {
    set_reduce_motion(env.reduce_motion);
    let primary = crate::windows::primary();
    let env = sk_css::Env {
        dynamic_range_high: crate::surface::dynamic_range_high(primary),
        ..env
    };
    crate::windows::set_env(primary, env);
    ENV.with(|e| {
        if e.get() == env {
            return false;
        }
        e.set(env);
        true
    })
}

pub fn set_window_env(k: crate::windows::WinKey, env: sk_css::Env) -> bool {
    let env = sk_css::Env {
        dynamic_range_high: crate::surface::dynamic_range_high(k),
        ..env
    };
    if !crate::windows::set_env(k, env) {
        return false;
    }
    let root = crate::windows::root_of(k);
    if root != 0 {
        with_tree(|t| t.mark_restyle_subtree(root));
    }
    true
}

pub fn env() -> sk_css::Env {
    ENV.with(|e| e.get())
}

pub fn animation_overlay(
    id: u32,
    base: &gpui::StyleRefinement,
    now: std::time::Instant,
) -> Option<gpui::StyleRefinement> {
    ANIMATOR.with(|a| a.borrow().overlay(id, base, now))
}

pub fn animated_extras(id: u32, now: std::time::Instant) -> Option<sk_css::Extras> {
    ANIMATOR.with(|animator| animator.borrow().animated_extras(id, now))
}

thread_local! {
    /// Elements the pointer is inside, per gpui's hover edges. Lets a restyle landing under a
    /// stationary pointer re-fold the target: an edge cannot answer "what state am I in now?".
    static HOVERED: RefCell<std::collections::HashSet<u32>> =
        RefCell::new(std::collections::HashSet::new());
}

/// `id` joined (`true`) or left the hover path (topmost element under the pointer and its
/// ancestors, from [`crate::pointer::resolve_hover`]).
/// A rung refined over the computed style has no clock, so a hover would snap: the renderer strips
/// `transition`ed properties from the rung and this routes the flip into the animator.
pub fn note_hover(id: u32, hovered: bool) {
    let changed = HOVERED.with(|h| {
        let mut h = h.borrow_mut();
        if hovered {
            h.insert(id)
        } else {
            h.remove(&id)
        }
    });
    if !changed {
        return;
    }
    let now = std::time::Instant::now();
    retarget_state(id, now);
    let deps = read_tree(|t| t.group_hover_deps.get(&id).cloned());
    for dep in deps.into_iter().flatten() {
        retarget_state(dep, now);
    }
}

/// `:hover` as of the last frame's resolution ([`note_hover`]).
pub fn is_hovered(id: u32) -> bool {
    HOVERED.with(|h| h.borrow().contains(&id))
}

/// The style in effect for `id` this frame (`getComputedStyle`). A pressed element reports its
/// unpressed style: gpui refines `:active` at paint from a hitbox that cannot be read back.
pub fn live_style(id: u32) -> Option<gpui::StyleRefinement> {
    use gpui::Refineable as _;
    let cs = read_tree(|t| t.node(id).and_then(|n| n.computed.clone()))?;
    let mut live = (*cs.base).clone();
    if let Some(overlay) = animation_overlay(id, &cs.base, std::time::Instant::now()) {
        live.refine(&overlay);
    }
    if let Some(group) = &cs.group_focus {
        read_tree(|t| {
            group.fold(&mut live, |key, state| {
                crate::live_state::focus_holds(t, key as u32, state)
            })
        });
    }
    if let Some(rung) = fold_hover_target(id, &cs) {
        match sk_css::strip_transitioned(&rung, &cs.transitions) {
            Some(stripped) => live.refine(&stripped),
            None => live.refine(&rung),
        }
    }
    Some(live)
}

fn fold_hover_target(id: u32, cs: &sk_css::ComputedStates) -> Option<gpui::StyleRefinement> {
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

/// True while any transition runs, so the host keeps repainting. Also drops settled tracks.
pub fn animating() -> bool {
    let now = std::time::Instant::now();
    ANIMATOR.with(|a| {
        let a = a.borrow();
        a.any_active(now) || a.any_animation(now)
    })
}

/// Nodes the animator is moving at `now`, plus their ancestors, into `out`: subtrees the
/// retained-render cache must not replay (`cache::begin_frame`).
pub(crate) fn moving_paths(now: std::time::Instant, out: &mut std::collections::HashSet<u32>) {
    ANIMATOR.with(|a| {
        let a = a.borrow();
        read_tree(|t| {
            for id in a.moving_nodes(now) {
                let mut cur = id;
                while cur != 0 && out.insert(cur) {
                    cur = t.node(cur).map_or(0, |n| n.parent);
                }
            }
        });
    });
}

/// Only the moving nodes, without ancestors: boxes under which `crate::cull` cannot trust layout
/// rectangles.
pub(crate) fn moving_nodes(now: std::time::Instant, out: &mut std::collections::HashSet<u32>) {
    ANIMATOR.with(|a| out.extend(a.borrow().moving_nodes(now)));
}

/// Advances the animator clock and queues boundary events. Must run every frame from
/// [`crate::frame_tick`], not behind [`animating`]'s short-circuit, or a frame repainting for
/// another reason skips the clock and queues no `animationstart`.
pub fn advance_animations() -> bool {
    let now = std::time::Instant::now();
    ANIMATOR.with(|a| {
        let mut a = a.borrow_mut();
        // `tick` queues `animationstart`/`animationend`; it runs before `finish_settled` drops
        // settled transition tracks (the source of `transitionend`).
        a.tick(now);
        a.finish_settled(now)
    })
}

pub fn take_anim_events() -> Vec<sk_css::AnimEvent> {
    ANIMATOR.with(|a| a.borrow_mut().take_events())
}

thread_local! {
    static SCRIPT_DIRTY: Cell<bool> = const { Cell::new(false) };
    static TIMELINE_ORIGIN: Cell<Option<std::time::Instant>> = const { Cell::new(None) };
}

/// Hands the animator the document timeline's zero on first script-animation use: redjs keeps one
/// clock for `performance.now()`, rAF and `redjs_now`, so its origin is a fixed `Instant`.
fn ensure_timeline_origin(animator: &mut sk_css::Animator) {
    if TIMELINE_ORIGIN.with(|o| o.get()).is_some() {
        return;
    }
    let now = std::time::Instant::now();
    let ms = unsafe { sk_runtime::redjs::ffi::redjs_now() };
    let origin = now
        .checked_sub(std::time::Duration::from_secs_f64(ms.max(0.0) / 1000.0))
        .unwrap_or(now);
    TIMELINE_ORIGIN.with(|o| o.set(Some(origin)));
    animator.set_timeline_origin(origin);
}

fn script_touched(node: Option<u32>) {
    if let Some(node) = node {
        with_tree(|t| t.touch(node));
    }
    SCRIPT_DIRTY.with(|d| d.set(true));
}

/// Creates or re-targets script animation `id`'s effect; keyframes compile against `node`'s custom
/// properties.
pub(crate) fn script_anim_set(
    id: u32,
    node: u32,
    frames: &[sk_css::ScriptFrame],
    timing: sk_css::ScriptTiming,
) -> sk_css::ScriptKeyframeNotes {
    let (vars, keywords) = read_tree(|t| {
        let own = t.node(node).and_then(|n| n.computed.as_ref());
        (
            own.map(|c| c.vars.clone()),
            crate::inherit::interpolate_keywords(
                t,
                node,
                own.and_then(|c| c.extras.interpolate_size),
            ),
        )
    });
    let vars = vars.unwrap_or_default();
    let (keyframes, notes) = with_engine(|engine| {
        let compile = || sk_css::script_keyframes(frames, &vars, &env());
        match engine {
            Some(engine) => engine.with_functions(compile),
            None => compile(),
        }
    });
    let previous = ANIMATOR.with(|a| {
        let mut a = a.borrow_mut();
        ensure_timeline_origin(&mut a);
        a.set_interpolate_keywords(node, keywords);
        let previous = a.script_node(id);
        a.script_set(id, node, keyframes, timing);
        previous
    });
    // A new target stops the old one painting it.
    if previous.is_some_and(|p| p != node) {
        script_touched(previous);
    }
    script_touched(Some(node));
    notes
}

pub(crate) fn script_anim_clock(id: u32, clock: sk_css::ScriptClock) {
    let node = ANIMATOR.with(|a| a.borrow_mut().script_clock(id, clock));
    script_touched(node);
}

pub(crate) fn script_anim_drop(id: u32) {
    let node = ANIMATOR.with(|a| a.borrow_mut().script_drop(id));
    script_touched(node);
}

pub(crate) fn script_anim_values(id: u32) -> Option<Vec<(String, String)>> {
    let now = std::time::Instant::now();
    ANIMATOR.with(|a| {
        let a = a.borrow();
        let node = a.script_node(id)?;
        let base = read_tree(|t| {
            t.node(node)
                .and_then(|n| n.computed.as_ref().map(|c| c.base.clone()))
        })
        .unwrap_or_default();
        a.script_values(id, &base, now)
    })
}

/// Did a script animation change what a node paints since the last call? Read once per frame by
/// [`crate::frame_tick`].
/// Each script-animation door touches the node and asks for a draw: a paused or seeked animation is
/// not "animating".
pub(crate) fn take_script_dirty() -> bool {
    SCRIPT_DIRTY.with(|d| d.replace(false))
}

/// Runs `f` under the installed sheet, or an empty one before the first.
pub(crate) fn with_sheet<R>(f: impl FnOnce(&sk_css::Engine) -> R) -> R {
    ENGINE.with(|e| match e.borrow().as_ref() {
        Some(engine) => f(engine),
        None => NO_SHEET.with(|engine| f(engine)),
    })
}

/// Starts, keeps or drops the CSS animations of a node outside the tree (a view transition
/// pseudo-element), as a restyle does for a tree node.
pub(crate) fn animate_detached(id: u32, states: &sk_css::ComputedStates, now: std::time::Instant) {
    let env = env();
    let diagnostics = std::cell::RefCell::new(Vec::new());
    with_sheet(|engine| {
        ANIMATOR.with(|a| {
            a.borrow_mut()
                .on_computed_animations(id, states, now, &|name| {
                    engine.keyframes_for_siblings(
                        name,
                        &states.vars,
                        states.sibling_values,
                        &env,
                        &mut diagnostics.borrow_mut(),
                    )
                });
        });
    });
    for d in diagnostics.into_inner() {
        warn_css(&format!("[css] {d}"));
    }
}

pub(crate) fn animations_finished(id: u32, now: std::time::Instant) -> bool {
    ANIMATOR.with(|a| a.borrow().animations_finished(id, now))
}

pub(crate) fn cancel_detached(id: u32) {
    let now = std::time::Instant::now();
    ANIMATOR.with(|a| a.borrow_mut().cancel_animations(id, now));
}

pub fn animator_gc() {
    ANIMATOR.with(|a| {
        a.borrow_mut()
            .gc(&|id| crate::view_transition::is_pseudo(id) || read_tree(|t| t.node(id).is_some()));
    });
    // An unmounted node never gets a leave edge (gpui drops the listener), so sweep the set or a
    // reused id comes up pre-hovered.
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

/// Sources of the installed sheet: the app's CSS, then each runtime `<style>` text, so at equal
/// specificity those win as in a browser. One text per sheet, not one concatenation: sk-css parses
/// each once per distinct text, so a changing `<style>` does not re-parse (and leak) the build's
/// sheet.
fn sheet_sources() -> Vec<String> {
    let mut out = vec![SHEET_SOURCE.with(|s| s.borrow().clone())];
    HEAD_SHEETS.with(|h| out.extend(h.borrow().iter().cloned()));
    RUNTIME_CSS.with(|r| out.extend(r.borrow().iter().cloned()));
    out
}

fn sync_runtime_sheets() {
    let next = with_tree(|t| {
        let mut placed: Vec<((bool, u32, Vec<usize>), u32)> = Vec::new();
        for &id in t.style_nodes.iter() {
            // Reused controls keep unopened content in detached fragments (Reka SelectViewport
            // includes a `<style>` there). Order is document order (child index path), not node id:
            // ids are reused, last freed first.
            let mut ancestor = id;
            let mut path = Vec::new();
            let mut top = None;
            for _ in 0..4096 {
                if ancestor == t.root || crate::windows::window_of_root(ancestor).is_some() {
                    top = Some(ancestor);
                    break;
                }
                let Some(node) = t.node(ancestor) else { break };
                let Some(parent) = t.node(node.parent) else {
                    break;
                };
                path.push(
                    parent
                        .children
                        .iter()
                        .position(|&c| c == ancestor)
                        .unwrap_or(usize::MAX),
                );
                ancestor = node.parent;
            }
            let Some(top) = top else { continue };
            path.reverse();
            placed.push(((top != t.root, top, path), id));
        }
        placed.sort();
        let mut sheets = Vec::new();
        for (_, id) in placed {
            fn walk(t: &Tree, id: u32, out: &mut String) {
                let Some(n) = t.node(id) else { return };
                out.push_str(&n.text);
                for &c in &n.children {
                    walk(t, c, out);
                }
            }
            let mut out = String::new();
            walk(t, id, &mut out);
            sheets.push(out);
        }
        sheets
    });
    if RUNTIME_CSS.with(|r| *r.borrow() == next) {
        return;
    }
    RUNTIME_CSS.with(|r| *r.borrow_mut() = next);
    // The dedupe set is not cleared: a `<style>` arrives in pieces and each step recompiles, so
    // clearing would re-print every advisory.
    recompile_sheet(
        crate::perf::FullReason::StyleElement,
        "a <style> element in the tree changed — recompiling the stylesheet",
    );
}

#[cfg(test)]
pub(crate) fn set_stylesheet(css: String) {
    set_stylesheet_with_head(css, Vec::new());
}

/// Installs the app's stylesheet plus the `document.head` `<style>` texts, each its own sheet after
/// `css` (see [`sheet_sources`]). Returns `false` and compiles nothing when they are already
/// installed: the runtime re-sends the sheet whenever a head `<style>` is touched.
pub fn set_stylesheet_with_head(css: String, head: Vec<String>) -> bool {
    let same = ENGINE.with(|e| e.borrow().is_some())
        && SHEET_SOURCE.with(|s| *s.borrow() == css)
        && HEAD_SHEETS.with(|h| *h.borrow() == head);
    if same {
        return false;
    }
    // A genuinely new sheet is a new set of gaps: an advisory suppressed against the old one may
    // speak again (the HMR path).
    let fresh = SHEET_SOURCE.with(|s| *s.borrow() != css);
    if fresh {
        CSS_WARNED.with(|w| w.borrow_mut().clear());
    }
    let before = crate::perf::live_bytes();
    SHEET_SOURCE.with(|s| *s.borrow_mut() = css);
    HEAD_SHEETS.with(|h| *h.borrow_mut() = head);
    let span = crate::perf::Span::start();
    let took = std::time::Instant::now();
    let sources = sheet_sources();
    let engine = compile(&sources);
    crate::perf::note_recompile(span.ms());
    set_engine(engine);
    let len = sources.iter().map(String::len).sum();
    drop(sources);
    crate::perf::note_sheet_compile(
        took.elapsed().as_secs_f32() * 1000.0,
        len,
        before,
        crate::perf::live_bytes(),
    );
    true
}

/// Compiles the sheets after handing their `@font-face` rules to [`crate::fonts`], which sk-css
/// does not keep.
fn compile(sources: &[String]) -> sk_css::Engine {
    crate::fonts::note_sheet(&sources.join("\n"));
    let sheets: Vec<&str> = sources.iter().map(String::as_str).collect();
    sk_css::Engine::compile_sheets(&sheets)
}

/// Web fonts were registered: sk-css chose every `font-family` against the fonts installed when it
/// compiled and parsed, so recompile, re-parse inline styles and restyle.
pub(crate) fn recompile_for_fonts() {
    recompile_sheet(
        crate::perf::FullReason::Fonts,
        "web fonts were registered — recompiling the stylesheet",
    );
    with_tree(|t| {
        t.invalidate_font_inline();
        t.mark_dirty();
    });
}

/// The app's `--shading` effects were replaced (`gpui::reload_app_effects`): sk-css resolves an
/// effect's name and arguments when a sheet compiles or an inline style parses, so recompile,
/// re-parse inline `--shading` and restyle. A renamed or removed effect then degrades as an unknown
/// one does.
pub fn effects_changed() {
    recompile_sheet(
        crate::perf::FullReason::Stylesheet,
        "the app's `--shading` effects were rebuilt — recompiling the stylesheet",
    );
    with_tree(|t| {
        t.invalidate_inline_named(&["--shading"]);
        t.mark_dirty();
    });
}

pub(crate) fn set_engine(engine: sk_css::Engine) {
    // A font load, late component style or rem recompile must not erase a running
    // animation/transition.
    REBIND_ANIMATIONS.with(|pending| pending.set(true));
    for d in &engine.diagnostics {
        warn_css(&format!("[css] {d}"));
    }
    note_parent_keys(&engine);
    ENGINE.with(|c| *c.borrow_mut() = Some(engine));
    // The cache is keyed by the old sheet's string addresses, which the new sheet may reuse for
    // other names.
    forget_read_cache();
}

fn forget_read_cache() {
    READ_CACHE.with(|c| c.iter().for_each(|s| s.set((0, 0, 0))));
}

/// Names [`ReadOnOthers`] cannot learn by listening: sk-css offers `.stack > *` only to a
/// `.stack`'s children, so no other child asks its parent about `stack`
/// (`Engine::classes_keyed_on_the_parent`).
fn note_parent_keys(engine: &sk_css::Engine) {
    READ_ON_OTHERS.with(|r| {
        let mut r = r.borrow_mut();
        let map = &mut r.names[NameKind::Class as usize];
        for name in engine.classes_keyed_on_the_parent() {
            let readers = match map.get_mut(name) {
                Some(readers) => readers,
                None => map.entry(name.into()).or_default(),
            };
            readers.all = true;
            readers.tags = Vec::new();
        }
    });
}

pub(crate) fn with_engine<R>(f: impl FnOnce(Option<&sk_css::Engine>) -> R) -> R {
    ENGINE.with(|e| f(e.borrow().as_ref()))
}

/// Restyles under the current engine (inline-only when none), then again if the root element
/// defines a different `rem`: `rem` is flattened at sheet-compile time, so a JS-set root font-size
/// needs a recompile. The root's own `rem` is of the initial 16px.
pub fn restyle_all() {
    let pending = read_tree(|t| t.dirty);
    sync_runtime_sheets();
    let env = env();
    // Viewport units are flattened to px at compile time like `rem`, so a resize re-resolves them
    // ([`rebase_viewport`]), only when the sheet has `vh`/`vw`.
    // vw/vh/rem share one process-wide basis, so two windows of different sizes sharing a `vh`
    // sheet would recompile the one `Engine` every frame. The basis stays the primary window's;
    // `@media` conditions stay per-window.
    let basis_moved = sk_css::set_viewport_basis(env.viewport_width, env.viewport_height);
    if basis_moved && sk_css::uses_viewport_units() {
        rebase_viewport(env.viewport_width, env.viewport_height);
    }
    // Inline styles no recompile reaches: re-parse each body that names a viewport unit.
    if basis_moved {
        with_tree(|t| t.invalidate_viewport_inline());
    }
    if sk_css::uses_viewport_units() && crate::windows::any_secondary() {
        crate::render::warn_compat(
            "multiwindow-viewport-units",
            "A-M05: this stylesheet uses `vw`/`vh`/`rem`, which Skathi flattens to px when \
             the sheet is COMPILED — so those lengths resolve against the PRIMARY window's \
             viewport in every window, including pop-outs. `@media` conditions, `env()` and \
             `prefers-color-scheme` are already per-window.",
        );
    }
    // The rem basis is settled before the document is cascaded, not by cascading it: `rem` is
    // flattened at sheet-compile time, so the basis precedes choosing the sheet.
    // A full cascade commits `node.computed`, so the pass after the recompile would look like an
    // author's style change to the animator and `onMounted` would measure the panel mid-transition.
    // The root has no parent or siblings, so one `compute_node` gives the basis. The root's own
    // `rem` has no fixed point under adoption (14, 12.25, 10.7...): two samples are linear in the
    // basis and give the size at 16px.
    let probe = || {
        ENGINE.with(|eng| {
            let eng = eng.borrow();
            read_tree(|t| t.probe_root_font_size(eng.as_ref(), &env))
        })
    };
    let mut earlier: Option<(f32, f32)> = None;
    for _ in 0..REM_BASIS_PASSES {
        let Some(root_font_size) = probe() else {
            break;
        };
        let basis = sk_css::rem_basis();
        if REM_SETTLED.with(Cell::get) == Some((basis, root_font_size)) {
            break;
        }
        let at_initial = earlier.and_then(|(b0, p0)| {
            let moved = (root_font_size - p0).abs() > 1e-3 && (basis - b0).abs() > 1e-3;
            moved.then(|| p0 + (root_font_size - p0) / (basis - b0) * (ROOT_PARENT_FONT_SIZE - b0))
        });
        earlier = Some((basis, root_font_size));
        let next = at_initial.unwrap_or(root_font_size);
        let moved = sk_css::set_rem_basis(next);
        if moved {
            with_tree(|t| t.invalidate_rem_inline());
        }
        let recompiled = moved
            && recompile_sheet(crate::perf::FullReason::RemBasis, &format!(
                "rem basis is now {next}px (the root element's font-size) — recompiling \n             the stylesheet"
            ));
        if at_initial.is_some() {
            let settled = if recompiled {
                probe()
            } else {
                Some(root_font_size)
            };
            REM_SETTLED.with(|c| c.set(settled.map(|size| (sk_css::rem_basis(), size))));
            break;
        }
        REM_SETTLED.with(|c| c.set(None));
        if !recompiled {
            break;
        }
    }
    // The recompile and re-resolve above mark the tree for the pass that follows, which is this
    // one; left armed, every such flush restyled twice.
    with_tree(|t| t.dirty = pending);
    ENGINE.with(|eng| {
        let eng = eng.borrow();
        with_tree(|t| t.restyle(eng.as_ref(), &env));
    });
}

fn has_viewport_unit(value: &str) -> bool {
    const UNITS: [&str; 24] = [
        "vw", "vh", "vi", "vb", "vmin", "vmax", "svw", "svh", "svi", "svb", "svmin", "svmax",
        "lvw", "lvh", "lvi", "lvb", "lvmin", "lvmax", "dvw", "dvh", "dvi", "dvb", "dvmin", "dvmax",
    ];
    has_unit(value, &UNITS)
}

fn has_rem_unit(value: &str) -> bool {
    has_unit(value, &["rem"])
}

fn has_unit(value: &str, units: &[&str]) -> bool {
    let b = value.as_bytes();
    let mut i = 0;
    while i < b.len() {
        if b[i].is_ascii_digit() || (b[i] == b'.' && b.get(i + 1).is_some_and(u8::is_ascii_digit)) {
            while i < b.len() && (b[i].is_ascii_digit() || b[i] == b'.') {
                i += 1;
            }
            let start = i;
            while i < b.len() && b[i].is_ascii_alphabetic() {
                i += 1;
            }
            let unit = value[start..i].to_ascii_lowercase();
            if units.contains(&unit.as_str()) {
                return true;
            }
        } else if b[i].is_ascii_alphabetic()
            || b[i] == b'_'
            || (b[i] == b'-'
                && b.get(i + 1)
                    .is_some_and(|c| c.is_ascii_alphabetic() || *c == b'-' || *c == b'_'))
        {
            // An identifier (`var(--x9vw)`): its digits are not a number.
            while i < b.len() && (b[i].is_ascii_alphanumeric() || b[i] == b'-' || b[i] == b'_') {
                i += 1;
            }
        } else {
            i += 1;
        }
    }
    false
}

/// A forced style flush: run the pending restyle now because JS is about to read a computed value.
/// The cascade runs once per wave, so a `getComputedStyle` read saw the previous style and reka's
/// `<Presence>` (reads `animationName` after `nextTick`) unmounted dialogs instantly instead of
/// playing the exit animation. Only the cascade is flushed, not layout.
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
        crate::perf::note_style_flush();
        restyle_all();
        crate::render::gc_node_state();
        animator_gc();
    }
}

fn recompile_sheet(reason: crate::perf::FullReason, why: &str) -> bool {
    // `dev`, not `css`: this is not a divergence from a browser, and a `css` advisory has to name
    // one.
    crate::log::record("dev", format!("[css] {why}"));
    let sources = sheet_sources();
    if sources.iter().all(|s| s.trim().is_empty()) {
        return false;
    }
    let before = crate::perf::live_bytes();
    let span = crate::perf::Span::start();
    let took = std::time::Instant::now();
    let engine = compile(&sources);
    crate::perf::note_recompile(span.ms());
    crate::perf::note_full(reason);
    set_engine(engine);
    let len = sources.iter().map(String::len).sum();
    drop(sources);
    crate::perf::note_sheet_compile(
        took.elapsed().as_secs_f32() * 1000.0,
        len,
        before,
        crate::perf::live_bytes(),
    );
    with_tree(|t| t.mark_dirty());
    true
}

/// The viewport basis moved under a sheet that uses `vh`/`vw`: re-resolve just those rules in the
/// installed engine ([`sk_css::Engine::rebase_viewport`]), or recompile when it cannot. A window
/// drag does this every step.
fn rebase_viewport(width: f32, height: f32) {
    let took = std::time::Instant::now();
    let rebased = ENGINE.with(|e| e.borrow_mut().as_mut().is_some_and(|e| e.rebase_viewport()));
    if !rebased {
        recompile_sheet(crate::perf::FullReason::ViewportBasis, &format!(
            "viewport basis is now {width}x{height}px — recompiling the stylesheet (it uses vh/vw units)"
        ));
        return;
    }
    crate::log::record("dev", format!(
        "[css] viewport basis is now {width}x{height}px — re-resolved the stylesheet's vh/vw rules ({:.1} ms)",
        took.elapsed().as_secs_f32() * 1000.0
    ));
    crate::perf::note_full(crate::perf::FullReason::ViewportBasis);
    with_tree(|t| t.mark_dirty());
}

/// One recompile to adopt the root's font-size, and one more if that was itself relative; a third
/// would be a cycle.
const REM_BASIS_PASSES: usize = 3;

const ROOT_PARENT_FONT_SIZE: f32 = 16.0;

#[derive(Clone, Debug, PartialEq)]
struct Inherited {
    color: Option<gpui::Hsla>,
    font_size: f32,
    font: gpui::Font,
}

#[derive(Clone)]
enum CascadeChange {
    All,
    Interaction,
    Variables(std::rc::Rc<sk_css::Vars>, std::rc::Rc<sk_css::VarChanges>),
}

impl CascadeChange {
    fn variables(&self) -> Option<(&std::rc::Rc<sk_css::Vars>, &std::rc::Rc<sk_css::VarChanges>)> {
        match self {
            Self::Variables(old, changes) => Some((old, changes)),
            _ => None,
        }
    }
}

#[derive(Clone, Copy)]
pub struct NodeRef<'t> {
    pub tree: &'t Tree,
    pub id: u32,
}

impl<'t> NodeRef<'t> {
    fn node(&self) -> &'t Node {
        self.tree
            .node(self.id)
            .expect("NodeRef points at a live node")
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
                return Some(NodeRef {
                    tree: self.tree,
                    id,
                });
            }
        }
        None
    }

    fn note_sibling_question(&self, edge: u8, walk: u8) {
        let subject = CASCADE_SUBJECT.with(Cell::get);
        if subject == 0 {
            return;
        }
        let parent = self.node().parent;
        if let Some(p) = self.tree.node(parent) {
            let flag = if self.id == subject { edge } else { walk };
            p.sel_flags.set(p.sel_flags.get() | flag);
        }
        if self.id != subject {
            if let Some(s) = self.tree.node(subject) {
                if s.parent != parent {
                    note_name_read(self.id, subject, s.tag, POSITION, NameKind::Position);
                }
            }
        }
    }

    fn note_read(&self, name: &str, kind: NameKind) {
        let subject = CASCADE_SUBJECT.with(Cell::get);
        if subject == 0 || subject == self.id {
            return;
        }
        let tag = self.tree.node(subject).map_or(0, |n| n.tag);
        note_name_read(self.id, subject, tag, name, kind);
    }

    /// The same for a question about this node's own content: `:empty`, or `:has()`
    /// ([`sel::HAS_SELF`] when asked of itself, [`sel::HAS_DESC`] when a descendant's selector
    /// asked an ancestor).
    /// Anchor nodes are Vue fragments carrying element children, so the walk flattens through them
    /// or a `:has()` would miss everything under a `v-if`.
    fn note_content_question(&self, flag: u8) {
        let subject = CASCADE_SUBJECT.with(Cell::get);
        if subject == 0 {
            return;
        }
        let flag = match flag {
            sel::HAS if self.id == subject => sel::HAS_SELF,
            sel::HAS => sel::HAS_DESC,
            other => other,
        };
        let n = self.node();
        n.sel_flags.set(n.sel_flags.get() | flag);
        if flag & sel::HAS != 0 {
            self.tree.has_anchors.set(true);
        }
    }
}

impl<'t> sk_css::MatchDom for NodeRef<'t> {
    const CASCADE_STATE: bool = true;
    fn generated_interaction(&self, state: sk_css::Interaction) -> bool {
        match state {
            sk_css::Interaction::Hover => is_hovered(self.id),
            sk_css::Interaction::Active => crate::live_state::active_holds(self.tree, self.id),
            _ => crate::live_state::focus_holds(self.tree, self.id, state),
        }
    }
    fn key(&self) -> u64 {
        self.id as u64
    }
    fn parent(&self) -> Option<Self> {
        let p = self.node().parent;
        (p != 0).then(|| NodeRef {
            tree: self.tree,
            id: p,
        })
    }
    fn sibling_values(&self) -> (u32, u32) {
        let parent = self.node().parent;
        if parent == 0 {
            return (1, 1);
        }
        let mut positions = self.tree.sibling_positions.borrow_mut();
        let children = positions.entry(parent).or_insert_with(|| {
            self.tree
                .element_children(parent)
                .enumerate()
                .map(|(index, child)| (child, index as u32 + 1))
                .collect()
        });
        (
            children.get(&self.id).copied().unwrap_or(1),
            children.len() as u32,
        )
    }

    fn prev_sibling(&self) -> Option<Self> {
        self.note_sibling_question(sel::PREV_EDGE, sel::PREV_WALK);
        self.element_sibling(false)
    }
    fn next_sibling(&self) -> Option<Self> {
        self.note_sibling_question(sel::NEXT_EDGE, sel::NEXT_WALK);
        self.element_sibling(true)
    }
    /// Flags the anchor as reading other nodes and its parent as having such a child: it reads its
    /// later siblings' subtrees, which no ancestor walk from a change reaches.
    fn note_following_sibling_anchor(&self) {
        if CASCADE_SUBJECT.with(Cell::get) == 0 {
            return;
        }
        self.note_content_question(sel::HAS);
        if let Some(p) = self.tree.node(self.node().parent) {
            p.sel_flags.set(p.sel_flags.get() | sel::HAS_NEXT);
        }
    }
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
        struct Restore(u32);
        impl Drop for Restore {
            fn drop(&mut self) {
                HAS_DEPTH.with(|d| d.set(self.0));
            }
        }
        let depth = HAS_DEPTH.with(|d| d.replace(d.get() + 1));
        let _restore = Restore(depth);
        if depth == 0 {
            self.note_content_question(sel::HAS);
        }
        walk(self.tree, self.id, f);
    }
    fn has_tag(&self, name: &str) -> bool {
        // The `html`/`body` aliasing for the root lives in sk-css (`element.rs`) so every
        // `MatchDom` gets it.
        self.tree.name(self.node().tag) == name
    }
    fn tag_eq(&self, other: &Self) -> bool {
        self.node().tag == other.node().tag
    }
    fn has_class(&self, name: &str) -> bool {
        self.note_read(name, NameKind::Class);
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
        self.note_read(name, NameKind::Scope);
        self.node().scopes.iter().any(|s| s == name)
    }
    /// An empty `class` reads as absent: the channel cannot tell `class=""` from none.
    fn attr(&self, name: &str) -> Option<&str> {
        self.note_read(name, NameKind::Attr);
        // `class` rides its own channel (`set_class`), not the property map.
        if name == "class" {
            let raw = self.node().class_attr.as_str();
            return (!raw.is_empty()).then_some(raw);
        }
        let key = *self.tree.name_ids.get(name)?;
        self.node()
            .props
            .iter()
            .find(|(k, _, _)| *k == key)
            .map(|(_, v, _)| v.as_str())
    }
    /// `:empty` per Selectors 3: only an element or a text node with data makes a parent non-empty.
    /// Comments and zero-length text (Vue leaves them for an off `v-if` and fragment ends) don't,
    /// or `empty:hidden` would never match. Whitespace-only text counts; an anchor with children is
    /// looked through.
    fn is_empty(&self) -> bool {
        self.note_content_question(sel::EMPTY);
        self.tree.is_bare(self.id)
    }
    fn is_root(&self) -> bool {
        self.node().parent == 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_mutation_bumps_the_subtree_revision_of_every_ancestor_and_of_nothing_else() {
        let (root, left, leaf, right) = with_tree(|t| {
            let root = t.create_root();
            let div = t.intern("div");
            let left = t.create_element(div);
            let leaf = t.create_element(div);
            let right = t.create_element(div);
            t.insert(left, root, 0);
            t.insert(leaf, left, 0);
            t.insert(right, root, 0);
            (root, left, leaf, right)
        });
        let before = read_tree(|t| {
            (
                t.sub_rev(root),
                t.sub_rev(left),
                t.sub_rev(leaf),
                t.sub_rev(right),
            )
        });

        with_tree(|t| t.set_class(leaf, "hot"));

        let after = read_tree(|t| {
            (
                t.sub_rev(root),
                t.sub_rev(left),
                t.sub_rev(leaf),
                t.sub_rev(right),
            )
        });
        assert!(after.0 > before.0, "the root is an ancestor of the change");
        assert!(after.1 > before.1, "so is the parent");
        assert!(after.2 > before.2, "and the node itself");
        assert_eq!(after.3, before.3, "the sibling subtree did not move");
    }

    #[test]
    fn removing_a_child_bumps_the_parent() {
        let (parent, child) = with_tree(|t| {
            let root = t.create_root();
            let div = t.intern("div");
            let parent = t.create_element(div);
            let child = t.create_element(div);
            t.insert(parent, root, 0);
            t.insert(child, parent, 0);
            (parent, child)
        });
        let before = read_tree(|t| t.sub_rev(parent));
        with_tree(|t| t.remove(child));
        assert!(read_tree(|t| t.sub_rev(parent)) > before);
    }

    #[test]
    fn hover_variables_restyle_readers_and_forward_the_map_through_idle_branches() {
        set_stylesheet(
            r#"
            @property --locked { syntax: "<percentage>"; inherits: false; initial-value: 2% }
            .group { --x: 0.25 } .group:hover { --x: 0.75; --locked: 70% }
            .pad { --other: fixed } .shadow { --x: 0.25 } .read { opacity: var(--x) }
            .alias { --derived: var(--x) } .derived { opacity: var(--derived) }
            .group:hover .group-target { --local: var(--x) }
            .explicit { --locked: inherit }
            @container (width > 9000px) { .unused { opacity: 0.1 } }
        "#
            .into(),
        );
        let (owner, pad, read, shadow, masked, derived, group_target, explicit, outside) =
            with_tree(|t| {
                let root = t.create_root();
                let tag = t.intern("div");
                let mut add = |parent, class| {
                    let id = t.create_element(tag);
                    t.set_class(id, class);
                    t.insert(id, parent, 0);
                    id
                };
                let owner = add(root, "group");
                let pad = add(owner, "pad");
                let read = add(pad, "read");
                let shadow = add(owner, "shadow");
                let masked = add(shadow, "read");
                let alias = add(owner, "alias");
                let derived = add(alias, "derived");
                (
                    owner,
                    pad,
                    read,
                    shadow,
                    masked,
                    derived,
                    add(pad, "group-target"),
                    add(owner, "explicit"),
                    add(root, "read"),
                )
            });
        restyle_all();
        let style = |id| read_tree(|t| t.node(id).unwrap().computed.clone().unwrap());
        let revisions = || read_tree(|t| (t.sub_rev(masked), t.sub_rev(outside)));
        let untouched = revisions();
        let pad_style = style(pad).base;
        assert_eq!(style(group_target).vars.get("--local"), None);
        for (hovered, expected) in [(true, 0.75), (false, 0.25)] {
            note_hover(owner, hovered);
            with_tree(|t| assert!(t.touch_hover_styled(owner)));
            flush_pending_style();
            assert_eq!(style(read).base.opacity, Some(expected));
            assert_eq!(style(derived).base.opacity, Some(expected));
            assert_eq!(style(pad).vars.get("--x").unwrap().parse(), Ok(expected));
            assert!(std::rc::Rc::ptr_eq(&pad_style, &style(pad).base));
            assert_eq!(
                style(shadow).vars.get("--x").unwrap().parse::<f32>(),
                Ok(0.25)
            );
            assert_eq!(style(masked).base.opacity, Some(0.25));
            assert_eq!(style(read).vars.get("--locked"), Some("2%"));
            assert_eq!(
                style(explicit).vars.get("--locked"),
                Some(if hovered { "70%" } else { "2%" })
            );
            assert_eq!(
                style(group_target)
                    .vars
                    .get("--local")
                    .map(str::parse::<f32>),
                hovered.then_some(Ok(expected))
            );
            assert_eq!(revisions(), untouched);
        }
    }

    #[test]
    fn hover_updates_a_named_container_style_query_across_a_variable_override() {
        set_stylesheet(
            ".owner { container-name: card; --theme: dark } .owner:hover { --theme: light }
             .shadow { --theme: dark } .leaf { opacity: 0.25 }
             @container card style(--theme: light) { .leaf { opacity: 0.75 } }"
                .into(),
        );
        let (owner, leaf) = with_tree(|t| {
            let root = t.create_root();
            let tag = t.intern("div");
            let mut add = |parent, class| {
                let id = t.create_element(tag);
                t.set_class(id, class);
                t.insert(id, parent, 0);
                id
            };
            let owner = add(root, "owner");
            let shadow = add(owner, "shadow");
            (owner, add(shadow, "leaf"))
        });
        restyle_all();
        for (hovered, expected) in [(true, 0.75), (false, 0.25)] {
            note_hover(owner, hovered);
            with_tree(|t| assert!(t.touch_hover_styled(owner)));
            flush_pending_style();
            read_tree(|t| {
                let style = t.node(leaf).unwrap().computed.as_ref().unwrap();
                assert_eq!(style.vars.get("--theme"), Some("dark"));
                assert_eq!(style.base.opacity, Some(expected));
            });
        }
    }

    #[test]
    fn the_cascade_touches_every_node_whose_fold_moved() {
        set_stylesheet(".on .leaf { opacity: 0.5 }".into());
        let (wrap, leaf, bystander) = with_tree(|t| {
            let root = t.create_root();
            let div = t.intern("div");
            let wrap = t.create_element(div);
            let leaf = t.create_element(div);
            let bystander = t.create_element(div);
            t.set_class(leaf, "leaf");
            t.insert(wrap, root, 0);
            t.insert(leaf, wrap, 0);
            t.insert(bystander, root, 0);
            (wrap, leaf, bystander)
        });
        restyle_all();
        let before = read_tree(|t| (t.sub_rev(leaf), t.sub_rev(bystander)));

        with_tree(|t| t.set_class(wrap, "on"));
        restyle_all();

        let after = read_tree(|t| (t.sub_rev(leaf), t.sub_rev(bystander)));
        assert_eq!(
            read_tree(|t| t
                .node(leaf)
                .and_then(|n| n.computed.as_ref())
                .and_then(|c| c.base.opacity)),
            Some(0.5),
            "the descendant combinator reached the leaf",
        );
        assert!(
            after.0 > before.0,
            "…so the leaf's subtree revision moved with it"
        );
        assert_eq!(after.1, before.1, "the bystander's fold did not move");
    }

    /// `<style>` is metadata content and generates no box, but its CSS must reach the cascade:
    /// libraries inject them at runtime (reka-ui, vite-plugin-css-injected-by-js,
    /// `createElement("style")`).
    #[test]
    fn a_runtime_style_element_reaches_the_cascade_and_recompiles_when_its_text_moves() {
        let opacity = |id: u32| {
            read_tree(|t| {
                t.node(id)
                    .and_then(|n| n.computed.as_ref())
                    .and_then(|c| c.base.opacity)
            })
        };

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
        assert_eq!(
            opacity(probe),
            Some(0.25),
            "the build's sheet, with an empty <style>"
        );

        // Created, then filled, as Vue and reka do.
        with_tree(|t| t.set_element_text(sheet, ".probe { opacity: 0.5 }".into()));
        restyle_all();
        assert_eq!(
            opacity(probe),
            Some(0.5),
            "a runtime <style> installs its rules"
        );

        with_tree(|t| t.set_element_text(sheet, ".probe { opacity: 0.75 }".into()));
        restyle_all();
        assert_eq!(opacity(probe), Some(0.75), "changed text re-installs");

        restyle_all();
        assert_eq!(
            opacity(probe),
            Some(0.75),
            "an unchanged sheet is not recompiled away"
        );

        with_tree(|t| t.set_element_text(sheet, String::new()));
        restyle_all();
        assert_eq!(
            opacity(probe),
            Some(0.25),
            "removing the rules restores the build's sheet"
        );
    }

    #[test]
    fn runtime_sheets_are_in_document_order() {
        set_stylesheet(String::new());
        with_tree(|t| {
            let root = t.create_root();
            let style = t.intern("style");
            let older = t.create_element(style);
            t.set_element_text(older, ".a { opacity: 0.5 }".into());
            t.insert(older, root, 0);
            let newer = t.create_element(style);
            t.set_element_text(newer, ".b { opacity: 0.5 }".into());
            t.insert(newer, root, older);
        });
        restyle_all();
        let sheets = RUNTIME_CSS.with(|r| r.borrow().clone());
        assert_eq!(
            sheets,
            vec![
                ".b { opacity: 0.5 }".to_string(),
                ".a { opacity: 0.5 }".to_string()
            ]
        );
    }

    fn runtime_sheet_probe() -> (u32, u32, u32, u32) {
        set_stylesheet(".probe { opacity: 0.25 }".into());
        with_tree(|t| {
            let root = t.create_root();
            let div = t.intern("div");
            let probe = t.create_element(div);
            t.set_class(probe, "probe");
            t.insert(probe, root, 0);
            let detached = t.create_anchor();
            let style = t.intern("style");
            let sheet = t.create_element(style);
            t.set_element_text(sheet, ".probe { opacity: 0.75 }".into());
            t.insert(sheet, detached, 0);
            (root, probe, detached, sheet)
        })
    }

    fn runtime_sheet_opacity(node: u32) -> Option<f32> {
        read_tree(|t| {
            t.node(node)
                .unwrap()
                .computed
                .as_ref()
                .unwrap()
                .base
                .opacity
        })
    }

    #[test]
    fn detached_fragment_styles_are_inert_until_connected_and_stop_on_detach() {
        let (root, probe, fragment, sheet) = runtime_sheet_probe();
        restyle_all();
        assert_eq!(
            runtime_sheet_opacity(probe),
            Some(0.25),
            "closed Select fragments install no rules"
        );

        with_tree(|t| t.insert(fragment, root, 0));
        restyle_all();
        assert_eq!(
            runtime_sheet_opacity(probe),
            Some(0.75),
            "connecting the fragment activates its style"
        );

        with_tree(|t| {
            let other_fragment = t.create_anchor();
            t.insert(fragment, other_fragment, 0);
        });
        restyle_all();
        assert_eq!(
            runtime_sheet_opacity(probe),
            Some(0.25),
            "detaching removes its rules without freeing the style"
        );

        with_tree(|t| t.set_element_text(sheet, ".probe { opacity: 0.9 }".into()));
        restyle_all();
        assert_eq!(
            runtime_sheet_opacity(probe),
            Some(0.25),
            "editing detached CSS remains inert"
        );
    }

    #[test]
    fn runtime_styles_in_a_painted_secondary_root_remain_connected() {
        let (_, probe, secondary_root, _) = runtime_sheet_probe();
        restyle_all();
        assert_eq!(runtime_sheet_opacity(probe), Some(0.25));

        crate::windows::set_painted_root(9001, secondary_root);
        restyle_all();
        assert_eq!(
            runtime_sheet_opacity(probe),
            Some(0.75),
            "SkWindow styles share the live document cascade"
        );

        crate::windows::set_painted_root(9001, 0);
        restyle_all();
        assert_eq!(
            runtime_sheet_opacity(probe),
            Some(0.25),
            "unpainting a detached root deactivates its style"
        );
        crate::windows::forget(9001);
    }

    #[test]
    fn empty_ignores_vue_anchors_and_empty_text() {
        let opacity = |id: u32| {
            read_tree(|t| {
                t.node(id)
                    .and_then(|n| n.computed.as_ref())
                    .and_then(|c| c.base.opacity)
            })
        };
        set_stylesheet(".box:empty { opacity: 0.5 }".into());
        let (bare, anchored, texted, text) = with_tree(|t| {
            let root = t.create_root();
            let div = t.intern("div");
            let mut boxed = || {
                let b = t.create_element(div);
                t.set_class(b, "box");
                t.insert(b, root, 0);
                b
            };
            let (bare, anchored, texted) = (boxed(), boxed(), boxed());
            for _ in 0..2 {
                let a = t.create_anchor();
                t.insert(a, anchored, 0);
            }
            let start = t.create_text();
            t.insert(start, anchored, 0);
            let text = t.create_text();
            t.insert(text, texted, 0);
            (bare, anchored, texted, text)
        });
        restyle_all();
        assert_eq!(opacity(bare), Some(0.5), "no children");
        assert_eq!(
            opacity(anchored),
            Some(0.5),
            "anchors and an empty text node only"
        );
        assert_eq!(opacity(texted), Some(0.5), "a text node with no data");

        with_tree(|t| t.set_text(text, " ".into()));
        restyle_all();
        assert_eq!(opacity(texted), None, "text with data, whitespace included");
        with_tree(|t| t.set_text(text, String::new()));
        restyle_all();
        assert_eq!(opacity(texted), Some(0.5), "and empty again");

        with_tree(|t| {
            let span = t.intern("span");
            let child = t.create_element(span);
            t.insert(child, anchored, 0);
        });
        restyle_all();
        assert_eq!(opacity(anchored), None, "an element child");
    }

    #[test]
    fn a_viewport_unit_is_a_dimension_token_not_a_substring() {
        for yes in [
            "100vw",
            "100svh",
            "calc(100vh - 2rem)",
            "-5vh",
            "-.5vmin",
            "85DVH",
            "min(50vw, 40rem)",
            "1.5lvmax",
        ] {
            assert!(has_viewport_unit(yes), "{yes:?}");
        }
        for no in [
            "10px",
            "var(--x9vw)",
            "abc12vh",
            "2rem",
            "",
            "50%",
            "url(a1vw.png)",
            "100v",
            "3vws",
        ] {
            assert!(!has_viewport_unit(no), "{no:?}");
        }
    }

    #[test]
    fn inline_viewport_units_and_var_composed_ones_follow_the_viewport() {
        fn px(l: Option<gpui::Length>) -> Option<f32> {
            match l? {
                gpui::Length::Definite(gpui::DefiniteLength::Absolute(
                    gpui::AbsoluteLength::Pixels(p),
                )) => Some(f32::from(p)),
                _ => None,
            }
        }
        let size = |id: u32| {
            read_tree(|t| {
                let cs = t.node(id)?.computed.as_ref()?;
                Some((px(cs.base.size.width), px(cs.base.size.height)))
            })
        };
        let resize = |w: f32, h: f32| {
            if set_env(sk_css::Env {
                viewport_width: w,
                viewport_height: h,
                ..sk_css::Env::default()
            }) {
                with_tree(|t| t.mark_dirty());
            }
            restyle_all();
        };
        resize(1652.0, 633.0);
        let (shell, child, plain) = with_tree(|t| {
            let root = t.create_root();
            let div = t.intern("div");
            let (width, height, var) = (t.intern("width"), t.intern("height"), t.intern("--w"));
            let shell = t.create_element(div);
            t.insert(shell, root, 0);
            t.set_style(shell, width, "100svw".into());
            t.set_style(shell, height, "100svh".into());
            let parent = t.create_element(div);
            t.insert(parent, root, 1);
            t.set_style(parent, var, "50vw".into());
            let child = t.create_element(div);
            t.insert(child, parent, 0);
            t.set_style(child, width, "var(--w)".into());
            let plain = t.create_element(div);
            t.insert(plain, root, 2);
            t.set_style(plain, width, "40px".into());
            (shell, child, plain)
        });
        restyle_all();
        assert_eq!(size(shell), Some((Some(1652.0), Some(633.0))));
        assert_eq!(size(child).and_then(|s| s.0), Some(826.0));

        resize(1700.0, 800.0);
        assert_eq!(
            size(shell),
            Some((Some(1700.0), Some(800.0))),
            "the inline svw/svh re-resolved"
        );
        assert_eq!(
            size(child).and_then(|s| s.0),
            Some(850.0),
            "and the var()-composed vw with it"
        );
        assert_eq!(size(plain).and_then(|s| s.0), Some(40.0));

        // Without the host's full invalidation (a scoped pass), the marks alone still reach both.
        assert!(sk_css::set_viewport_basis(1000.0, 500.0));
        let marked = with_tree(|t| t.invalidate_viewport_inline());
        assert_eq!(
            marked, 2,
            "the shell and the --w parent; never the px-only node"
        );
        ENGINE.with(|eng| with_tree(|t| t.restyle(eng.borrow().as_ref(), &env())));
        assert_eq!(size(shell), Some((Some(1000.0), Some(500.0))));
        assert_eq!(size(child).and_then(|s| s.0), Some(500.0));
    }

    #[test]
    fn sheet_viewport_units_follow_the_viewport() {
        let width = |id: u32| {
            read_tree(|t| match t.node(id)?.computed.as_ref()?.base.size.width? {
                gpui::Length::Definite(gpui::DefiniteLength::Absolute(
                    gpui::AbsoluteLength::Pixels(p),
                )) => Some(f32::from(p)),
                _ => None,
            })
        };
        let resize = |w: f32, h: f32| {
            if set_env(sk_css::Env {
                viewport_width: w,
                viewport_height: h,
                ..sk_css::Env::default()
            }) {
                with_tree(|t| t.mark_dirty());
            }
            restyle_all();
        };
        resize(1000.0, 800.0);
        assert!(set_stylesheet_with_head(
            ".box, .wide { width: 50vw } .wide { min-height: 10vh }".into(),
            vec![]
        ));
        let (bx, wide) = with_tree(|t| {
            let root = t.create_root();
            let div = t.intern("div");
            let bx = t.create_element(div);
            t.insert(bx, root, 0);
            t.set_class(bx, "box");
            let wide = t.create_element(div);
            t.insert(wide, root, 1);
            t.set_class(wide, "wide");
            t.mark_dirty();
            (bx, wide)
        });
        restyle_all();
        assert_eq!((width(bx), width(wide)), (Some(500.0), Some(500.0)));
        resize(1200.0, 800.0);
        assert_eq!(
            (width(bx), width(wide)),
            (Some(600.0), Some(600.0)),
            "rebased"
        );

        assert!(set_stylesheet_with_head(
            ".box, .wide { width: 50vw } @keyframes k { from { height: 10vh } }".into(),
            vec![]
        ));
        with_tree(|t| t.mark_dirty());
        restyle_all();
        resize(800.0, 800.0);
        assert_eq!(
            (width(bx), width(wide)),
            (Some(400.0), Some(400.0)),
            "recompiled"
        );
        sk_css::set_viewport_basis(
            sk_css::INITIAL_VIEWPORT_BASIS.0,
            sk_css::INITIAL_VIEWPORT_BASIS.1,
        );
    }

    #[test]
    fn an_unchanged_sheet_resent_compiles_nothing() {
        assert!(set_stylesheet_with_head(
            ".a { opacity: 0.5 }".into(),
            vec![".b {}".into()]
        ));
        assert!(!set_stylesheet_with_head(
            ".a { opacity: 0.5 }".into(),
            vec![".b {}".into()]
        ));
        assert!(set_stylesheet_with_head(
            ".a { opacity: 0.5 }".into(),
            vec![".b {}".into(), ".c {}".into()]
        ));
        assert!(set_stylesheet_with_head(
            ".a { opacity: 0.6 }".into(),
            vec![".b {}".into(), ".c {}".into()]
        ));
        assert_eq!(
            sheet_sources(),
            vec![
                ".a { opacity: 0.6 }".to_string(),
                ".b {}".into(),
                ".c {}".into()
            ],
            "the build's sheet, then each head text on its own"
        );
    }

    fn flush_motion_styles() {
        with_tree(|t| t.mark_dirty());
        restyle_all();
    }

    fn motion_probe(sheet: &str) -> (u32, std::time::Instant) {
        set_stylesheet(sheet.into());
        let node = with_tree(|t| {
            let root = t.create_root();
            let div = t.intern("div");
            let node = t.create_element(div);
            t.set_class(node, "motion");
            t.insert(node, root, 0);
            node
        });
        flush_motion_styles();
        let start = std::time::Instant::now();
        ANIMATOR.with(|a| a.borrow_mut().tick(start));
        let _ = take_anim_events();
        (node, start)
    }

    fn motion_opacity_at(node: u32, at: std::time::Instant) -> Option<f32> {
        let base = read_tree(|t| {
            t.node(node)
                .unwrap()
                .computed
                .as_ref()
                .unwrap()
                .base
                .clone()
        });
        animation_overlay(node, &base, at).and_then(|style| style.opacity)
    }

    #[test]
    fn late_stylesheet_and_font_recompile_preserve_a_running_animation_clock() {
        let sheet = "@keyframes enter { from { opacity: 0 } to { opacity: 1 } } \
                     .motion { animation: enter 500ms linear }";
        let (node, start) = motion_probe(sheet);
        let at = start + std::time::Duration::from_millis(200);
        assert!((motion_opacity_at(node, at).unwrap() - 0.4).abs() < 1e-5);

        assert!(set_stylesheet_with_head(
            sheet.into(),
            vec![".late-panel { color: red }".into()]
        ));
        flush_motion_styles();
        assert!(
            (motion_opacity_at(node, at).unwrap() - 0.4).abs() < 1e-5,
            "unrelated CSS must retain the elapsed clock, not drop/restart the run"
        );

        recompile_for_fonts();
        flush_motion_styles();
        assert!(
            (motion_opacity_at(node, at).unwrap() - 0.4).abs() < 1e-5,
            "registering a font must not erase the running animation"
        );
    }

    #[test]
    fn changed_keyframes_rebind_at_the_existing_clock_and_deleted_keyframes_cancel() {
        let sheet = "@keyframes enter { from { opacity: 0 } to { opacity: 1 } } \
                     .motion { animation: enter 500ms linear }";
        let (node, start) = motion_probe(sheet);
        let at = start + std::time::Duration::from_millis(200);
        set_stylesheet(sheet.replace("opacity: 1", "opacity: 0.5"));
        flush_motion_styles();
        assert!(
            (motion_opacity_at(node, at).unwrap() - 0.2).abs() < 1e-5,
            "same-name definition changes sample new frames on the existing timeline"
        );

        set_stylesheet(".motion { animation: enter 500ms linear }".into());
        flush_motion_styles();
        assert!(
            motion_opacity_at(node, at).is_none(),
            "deleted frames must not keep painting"
        );
        let events = take_anim_events();
        assert_eq!(
            events
                .iter()
                .filter(|event| event.node == node
                    && event.kind == sk_css::AnimEventKind::AnimationCancel)
                .count(),
            1
        );
    }

    #[test]
    fn removing_the_animation_and_last_keyframes_cancels_the_old_run() {
        let (node, start) = motion_probe(
            "@keyframes enter { from { opacity: 0 } to { opacity: 1 } } \
             .motion { animation: enter 500ms linear }",
        );
        set_stylesheet(".motion { opacity: 1 }".into());
        flush_motion_styles();
        assert!(motion_opacity_at(node, start).is_none());
        assert!(take_anim_events().iter().any(
            |event| event.node == node && event.kind == sk_css::AnimEventKind::AnimationCancel
        ));
    }

    #[test]
    fn unrelated_stylesheet_preserves_a_running_transition_clock() {
        let sheet = ".motion { opacity: 0; transition: opacity 500ms linear } \
                     .motion.open { opacity: 1 }";
        let (node, _) = motion_probe(sheet);
        with_tree(|t| t.set_class(node, "motion open"));
        flush_motion_styles();
        let start = std::time::Instant::now();
        let at = start + std::time::Duration::from_millis(200);
        let before = motion_opacity_at(node, at).unwrap();
        assert!(before > 0.39 && before < 0.42, "{before}");
        assert!(set_stylesheet_with_head(
            sheet.into(),
            vec![".late-panel { color: red }".into()]
        ));
        flush_motion_styles();
        assert!(
            (motion_opacity_at(node, at).unwrap() - before).abs() < 1e-5,
            "a late sheet must not snap or restart an unrelated transition"
        );
    }

    /// A `rem` recompile is the engine correcting its own basis, not an author change, so it must
    /// not start a transition.
    #[test]
    fn a_rem_recompile_at_boot_starts_no_transition() {
        set_stylesheet(".card { padding: 1.5rem; transition: all 300ms }".into());
        let card = with_tree(|t| {
            let root = t.create_root();
            // Exactly `@argon/theme`'s line: an inline root font-size written before the app
            // mounts, which the cold compile cannot have known.
            let font_size = t.intern("font-size");
            t.set_style(root, font_size, "14px".into());
            let div = t.intern("div");
            let card = t.create_element(div);
            t.set_class(card, "card");
            t.insert(card, root, 0);
            card
        });

        let probed = ENGINE.with(|eng| {
            let eng = eng.borrow();
            read_tree(|t| t.probe_root_font_size(eng.as_ref(), &env()))
        });
        assert_eq!(
            probed,
            Some(14.0),
            "the root's own inline font-size, without a cascade"
        );
        assert!(
            read_tree(|t| t.node(t.root).is_some_and(|n| n.computed.is_none())),
            "probing the basis must not commit a computed style"
        );

        restyle_all();
        assert_eq!(
            sk_css::rem_basis(),
            14.0,
            "the sheet was recompiled against the root"
        );

        let padding = read_tree(|t| {
            t.node(card)
                .and_then(|n| n.computed.as_ref())
                .and_then(|c| c.base.padding.top)
        });
        assert_eq!(
            padding,
            Some(gpui::DefiniteLength::Absolute(
                gpui::AbsoluteLength::Pixels(gpui::px(21.0))
            )),
            "the ONE committed cascade ran under the settled basis"
        );

        let base = read_tree(|t| {
            t.node(card)
                .and_then(|n| n.computed.as_ref())
                .map(|c| c.base.clone())
        })
        .expect("the card is cascaded");
        assert!(
            animation_overlay(card, &base, std::time::Instant::now()).is_none(),
            "a basis recompile is not a style change, so no transition runs across it"
        );
    }

    #[test]
    fn several_style_elements_concatenate_in_document_order() {
        let opacity = |id: u32| {
            read_tree(|t| {
                t.node(id)
                    .and_then(|n| n.computed.as_ref())
                    .and_then(|c| c.base.opacity)
            })
        };
        set_stylesheet(String::new());
        let probe = with_tree(|t| {
            let root = t.create_root();
            let div = t.intern("div");
            let probe = t.create_element(div);
            t.set_class(probe, "probe");
            t.insert(probe, root, 0);
            let style_tag = t.intern("style");
            for css in [".probe { opacity: 0.1 }", ".probe { opacity: 0.9 }"] {
                let sheet = t.create_element(style_tag);
                t.set_element_text(sheet, css.into());
                t.insert(sheet, root, 0);
            }
            probe
        });
        restyle_all();
        assert_eq!(
            opacity(probe),
            Some(0.9),
            "the later sheet wins at equal specificity"
        );
    }

    fn cascaded_comb(n: usize) -> (u32, Vec<u32>, Vec<u32>) {
        let ids = with_tree(|t| {
            let root = t.create_root();
            let div = t.intern("div");
            let mut kids = Vec::new();
            let mut grandkids = Vec::new();
            for _ in 0..n {
                let kid = t.create_element(div);
                let grandkid = t.create_element(div);
                t.insert(grandkid, kid, 0);
                t.insert(kid, root, 0);
                kids.push(kid);
                grandkids.push(grandkid);
            }
            (root, kids, grandkids)
        });
        restyle_all();
        ids
    }

    fn roots_of() -> Vec<u32> {
        let mut roots = with_tree(|t| t.restyle_roots());
        roots.sort_unstable();
        roots
    }

    #[test]
    fn hundreds_of_separate_marks_stay_a_scoped_pass() {
        let (_, _, grandkids) = cascaded_comb(300);
        with_tree(|t| {
            for &g in &grandkids {
                t.mark_restyle_subtree(g);
            }
        });
        let mut want = grandkids.clone();
        want.sort_unstable();
        assert_eq!(roots_of(), want);
        assert!(roots_of().is_empty());
    }

    #[test]
    fn a_mark_under_a_marked_ancestor_is_not_walked_twice() {
        let (_, kids, grandkids) = cascaded_comb(3);
        with_tree(|t| {
            t.mark_restyle_subtree(grandkids[1]);
            t.mark_restyle_subtree(kids[1]);
        });
        assert_eq!(roots_of(), vec![kids[1]]);
    }

    #[test]
    fn marks_in_a_parentless_container_are_found_and_follow_a_move() {
        let (_, kids, _) = cascaded_comb(2);
        let (cache, kept) = with_tree(|t| {
            let div = t.intern("div");
            let cache = t.create_element(div);
            let kept = t.create_element(div);
            t.insert(kept, cache, 0);
            (cache, kept)
        });
        restyle_all();
        with_tree(|t| t.mark_restyle_subtree(kept));
        assert_eq!(roots_of(), vec![kept]);

        with_tree(|t| {
            t.mark_restyle_subtree(kept);
            t.insert(kept, kids[0], 0);
        });
        assert_eq!(roots_of(), vec![kept]);
        let _ = cache;
    }

    #[test]
    fn an_append_walks_the_new_child_and_not_its_siblings() {
        let (root, kids, _) = cascaded_comb(50);
        let added = with_tree(|t| {
            let div = t.intern("div");
            let added = t.create_element(div);
            t.insert(added, root, 0);
            added
        });
        assert_eq!(roots_of(), vec![added]);
        let _ = kids;
    }

    /// `body:has(…)` makes the root an anchor over every node; marking it on every mutation would
    /// make each wave a whole-document walk.
    #[test]
    fn a_root_has_anchor_is_rechecked_not_walked() {
        let sheet = "body:has([data-x]) { opacity: 0.5 }";
        set_engine(sk_css::Engine::compile(sheet));
        let (root, _, grandkids) = cascaded_comb(3);
        assert!(
            read_tree(|t| t.node(root).unwrap().sel_flags.get() & sel::HAS_SELF != 0),
            "the root asked `:has()` of itself"
        );
        let engine = sk_css::Engine::compile(sheet);
        let env = env();

        let settled_ids = |engine: &sk_css::Engine| {
            with_tree(|t| t.settle_self_marks(engine, &env))
                .into_iter()
                .map(|(id, _)| id)
                .collect::<Vec<_>>()
        };

        with_tree(|t| t.set_class(grandkids[1], "x"));
        assert!(settled_ids(&engine).is_empty());
        assert!(roots_of().is_empty(), "no walk at all");

        let x = with_tree(|t| t.intern("data-x"));
        with_tree(|t| t.set_prop(grandkids[0], x, Some(String::new())));
        assert_eq!(settled_ids(&engine), vec![root]);
        assert!(roots_of().is_empty(), "no walk at all");
    }

    #[test]
    fn a_following_sibling_has_anchor_is_rechecked_when_that_sibling_changes() {
        let sheet = "div:has(+ [data-x]) { opacity: 0.5 }";
        set_engine(sk_css::Engine::compile(sheet));
        let (root, kids, _) = cascaded_comb(2);
        assert!(
            read_tree(|t| t.node(root).unwrap().sel_flags.get() & sel::HAS_NEXT != 0),
            "a child asked `:has(+ …)`"
        );
        let engine = sk_css::Engine::compile(sheet);
        let env = env();
        let x = with_tree(|t| t.intern("data-x"));
        with_tree(|t| t.set_prop(kids[1], x, Some(String::new())));
        let settled: Vec<u32> = with_tree(|t| t.settle_self_marks(&engine, &env))
            .into_iter()
            .map(|(id, _)| id)
            .collect();
        assert!(
            settled.contains(&kids[0]),
            "the earlier sibling's answer moved: {settled:?}"
        );
    }

    #[test]
    fn a_text_node_filling_up_rechecks_the_has_anchor_that_reads_empty() {
        let sheet = ".card:has(span:empty) { opacity: 0.5 }";
        set_engine(sk_css::Engine::compile(sheet));
        let (card, text) = with_tree(|t| {
            let root = t.create_root();
            let (div, span) = (t.intern("div"), t.intern("span"));
            let card = t.create_element(div);
            t.set_class(card, "card");
            let badge = t.create_element(span);
            let text = t.create_text();
            t.insert(text, badge, 0);
            t.insert(badge, card, 0);
            t.insert(card, root, 0);
            (card, text)
        });
        restyle_all();
        let engine = sk_css::Engine::compile(sheet);
        let env = env();
        with_tree(|t| t.set_text(text, "3".into()));
        let settled = with_tree(|t| t.settle_self_marks(&engine, &env));
        let card_now = settled
            .iter()
            .find(|(id, _)| *id == card)
            .map(|(_, s)| s.base.opacity);
        assert_eq!(
            card_now,
            Some(None),
            "the badge has text now: {:?}",
            settled.iter().map(|s| s.0).collect::<Vec<_>>()
        );
    }

    #[test]
    fn a_prop_set_again_after_a_removal_has_a_new_write_count() {
        let (id, value) = with_tree(|t| {
            t.create_root();
            let input = t.intern("input");
            let id = t.create_element(input);
            (id, t.intern("value"))
        });
        let write = |v: Option<&str>| {
            with_tree(|t| t.set_prop(id, value, v.map(str::to_string)));
            read_tree(|t| t.prop_write(id, "value").map(|(_, write)| write))
        };
        let first = write(Some("a")).unwrap();
        assert_ne!(
            write(Some("a")).unwrap(),
            first,
            "the same string again is still a write"
        );
        assert_eq!(write(None), None);
        assert_ne!(write(Some("a")).unwrap(), first);
    }

    #[test]
    fn a_class_read_only_by_its_own_element_restyles_that_element_alone() {
        let sheet = ".hot { opacity: 0.5 } .on .leaf { opacity: 0.25 }";
        set_engine(sk_css::Engine::compile(sheet));
        let (root, kids, grandkids) = with_tree(|t| {
            let root = t.create_root();
            let div = t.intern("div");
            let kid = t.create_element(div);
            let leaf = t.create_element(div);
            t.set_class(leaf, "leaf");
            t.insert(leaf, kid, 0);
            t.insert(kid, root, 0);
            (root, vec![kid], vec![leaf])
        });
        restyle_all();
        let engine = sk_css::Engine::compile(sheet);
        let env = env();

        with_tree(|t| t.set_class(kids[0], "hot"));
        let settled = with_tree(|t| t.settle_self_marks(&engine, &env));
        assert_eq!(settled.len(), 1);
        assert_eq!(settled[0].0, kids[0]);
        assert_eq!(settled[0].1.base.opacity, Some(0.5));
        assert!(roots_of().is_empty(), "the subtree is not walked");

        with_tree(|t| t.set_class(kids[0], "hot on"));
        let settled = with_tree(|t| t.settle_self_marks(&engine, &env));
        assert_eq!(
            settled.iter().map(|s| s.0).collect::<Vec<_>>(),
            vec![kids[0]]
        );
        assert_eq!(roots_of(), vec![grandkids[0]]);
        let _ = root;
    }

    #[test]
    fn a_moved_sibling_is_restyled_alone_with_the_elements_that_read_its_place() {
        let sheet = ".row:nth-child(2) { opacity: 0.5 } .row:first-child .cell { opacity: 0.25 }";
        set_engine(sk_css::Engine::compile(sheet));
        let (list, rows, cells) = with_tree(|t| {
            let root = t.create_root();
            let div = t.intern("div");
            let span = t.intern("span");
            let list = t.create_element(div);
            let (mut rows, mut cells) = (Vec::new(), Vec::new());
            for _ in 0..3 {
                let row = t.create_element(div);
                t.set_class(row, "row");
                let other = t.create_element(div);
                let cell = t.create_element(span);
                t.set_class(cell, "cell");
                t.insert(other, row, 0);
                t.insert(cell, row, 0);
                t.insert(row, list, 0);
                rows.push(row);
                cells.push(cell);
            }
            t.insert(list, root, 0);
            (list, rows, cells)
        });
        restyle_all();
        let opacity = |id: u32| {
            read_tree(|t| {
                t.node(id)
                    .and_then(|n| n.computed.as_ref())
                    .and_then(|c| c.base.opacity)
            })
        };
        assert_eq!(
            (opacity(rows[1]), opacity(cells[0])),
            (Some(0.5), Some(0.25))
        );

        let engine = sk_css::Engine::compile(sheet);
        let env = env();
        let added = with_tree(|t| {
            let div = t.intern("div");
            let added = t.create_element(div);
            t.set_class(added, "row");
            t.insert(added, list, rows[0]);
            added
        });
        let settled = with_tree(|t| t.settle_self_marks(&engine, &env));
        assert_eq!(
            settled.iter().map(|s| s.0).collect::<Vec<_>>(),
            vec![rows[0], rows[1]]
        );
        let mut want = vec![added, cells[0], cells[1], cells[2]];
        want.sort_unstable();
        assert_eq!(
            roots_of(),
            want,
            "the new row and the cells, none of the rows' other children"
        );

        with_tree(|t| t.remove(added));
        restyle_all();
        with_tree(|t| {
            let div = t.intern("div");
            let added = t.create_element(div);
            t.set_class(added, "row");
            t.insert(added, list, rows[0]);
        });
        restyle_all();
        assert_eq!(
            [rows[0], rows[1], cells[0], cells[1]].map(opacity),
            [Some(0.5), None, None, None],
            "the old first row is the second now, and no cell is under the first row"
        );
    }

    #[test]
    fn a_name_read_by_a_few_tags_walks_only_those_elements() {
        let sheet = "[class*=\"submit\"] button { opacity: 0.5 }";
        set_engine(sk_css::Engine::compile(sheet));
        let (panel, buttons) = with_tree(|t| {
            let root = t.create_root();
            let div = t.intern("div");
            let button = t.intern("button");
            let panel = t.create_element(div);
            let mut buttons = Vec::new();
            for _ in 0..3 {
                let row = t.create_element(div);
                let b = t.create_element(button);
                let label = t.create_element(div);
                t.insert(label, b, 0);
                t.insert(b, row, 0);
                t.insert(row, panel, 0);
                buttons.push(b);
            }
            t.insert(panel, root, 0);
            (panel, buttons)
        });
        restyle_all();
        let opacity = |id: u32| {
            read_tree(|t| {
                t.node(id)
                    .and_then(|n| n.computed.as_ref())
                    .and_then(|c| c.base.opacity)
            })
        };
        assert_eq!(opacity(buttons[0]), None);

        let engine = sk_css::Engine::compile(sheet);
        let env = env();
        with_tree(|t| t.set_class(panel, "submit-row"));
        assert!(
            with_tree(|t| t.settle_self_marks(&engine, &env)).is_empty(),
            "the panel's own style did not move"
        );
        let mut want = buttons.clone();
        want.sort_unstable();
        assert_eq!(
            roots_of(),
            want,
            "the buttons, and none of the divs around them"
        );

        with_tree(|t| t.set_class(panel, "submit-row wide"));
        restyle_all();
        assert!(buttons.iter().all(|&b| opacity(b) == Some(0.5)));
        with_tree(|t| t.set_class(panel, "wide"));
        restyle_all();
        assert!(buttons.iter().all(|&b| opacity(b).is_none()));
    }

    #[test]
    fn reordering_restyles_the_changed_index_readers_without_restyling_count_readers() {
        let sheet = ".indexed { z-index: calc(sibling-index() * 2); } \
                     .counted { z-index: sibling-count(); }";
        let engine = sk_css::Engine::compile(sheet);
        let mut tree = Tree::new();
        let root = tree.create_root();
        let tag = tree.intern("div");
        let a = tree.create_element(tag);
        let b = tree.create_element(tag);
        let counted = tree.create_element(tag);
        tree.set_class(a, "indexed");
        tree.set_class(b, "indexed");
        tree.set_class(counted, "counted");
        for child in [a, b, counted] {
            tree.insert(child, root, 0);
        }
        let env = sk_css::Env::default();
        tree.restyle(Some(&engine), &env);
        let z = |tree: &Tree, id| tree.node(id).unwrap().computed.as_ref().unwrap().extras.z();
        assert_eq!(
            [z(&tree, a), z(&tree, b), z(&tree, counted)],
            [Some(2), Some(4), Some(3)]
        );
        tree.insert(b, root, a);
        assert_ne!(tree.node(counted).unwrap().mark_gen, tree.restyle_gen);
        tree.restyle(Some(&engine), &env);
        assert_eq!(
            [z(&tree, a), z(&tree, b), z(&tree, counted)],
            [Some(4), Some(2), Some(3)]
        );
        let text = tree.create_text();
        tree.set_text(text, "text".into());
        tree.insert(text, root, a);
        assert_ne!(tree.node(a).unwrap().mark_gen, tree.restyle_gen);
        tree.remove(b);
        tree.restyle(Some(&engine), &env);
        assert_eq!([z(&tree, a), z(&tree, counted)], [Some(2), Some(2)]);
    }

    #[test]
    fn an_insert_marks_the_siblings_the_cascade_said_it_read() {
        let (root, kids, _) = cascaded_comb(5);
        let take_self_marks = || {
            with_tree(|t| {
                let mut marks = std::mem::take(&mut t.self_marks);
                marks.sort_unstable();
                marks.dedup();
                marks
            })
        };
        with_tree(|t| t.node(root).unwrap().sel_flags.set(sel::NEXT_EDGE));
        let added = with_tree(|t| {
            let div = t.intern("div");
            let added = t.create_element(div);
            t.insert(added, root, 0);
            added
        });
        assert_eq!(take_self_marks(), vec![kids[4]], "the old last child");
        assert_eq!(roots_of(), vec![added], "and the new one");

        with_tree(|t| t.node(root).unwrap().sel_flags.set(sel::PREV_WALK));
        let middle = with_tree(|t| {
            let div = t.intern("div");
            let middle = t.create_element(div);
            t.insert(middle, root, kids[3]);
            middle
        });
        let mut want = vec![kids[3], kids[4], added];
        want.sort_unstable();
        assert_eq!(take_self_marks(), want);
        assert_eq!(roots_of(), vec![middle]);
    }

    #[test]
    fn a_root_under_an_anchor_is_promoted_to_the_anchor() {
        let (anchor, inner) = with_tree(|t| {
            let root = t.create_root();
            let div = t.intern("div");
            let anchor = t.create_anchor();
            let a = t.create_element(div);
            let b = t.create_element(div);
            t.insert(a, anchor, 0);
            t.insert(b, anchor, 0);
            t.insert(anchor, root, 0);
            (anchor, b)
        });
        restyle_all();
        with_tree(|t| t.mark_restyle_subtree(inner));
        assert_eq!(roots_of(), vec![anchor]);
    }

    #[test]
    fn a_sibling_combinator_reaches_later_siblings_where_they_read() {
        let sheet = ".a + .b { opacity: 0.5 }";
        set_engine(sk_css::Engine::compile(sheet));
        let (root, kids, grandkids) = cascaded_comb(5);
        let opacity = |id: u32| {
            read_tree(|t| {
                t.node(id)
                    .and_then(|n| n.computed.as_ref())
                    .and_then(|c| c.base.opacity)
            })
        };
        with_tree(|t| kids.iter().for_each(|&k| t.set_class(k, "b")));
        restyle_all();
        let order: Vec<u32> = read_tree(|t| t.node(root).unwrap().children.clone());
        let (first, second, third) = (order[0], order[1], order[2]);

        with_tree(|t| t.set_class(first, "a b"));
        let mut want: Vec<u32> = order[1..].to_vec();
        want.push(grandkids[kids.iter().position(|&k| k == first).unwrap()]);
        want.sort_unstable();
        assert_eq!(roots_of(), want, "the divs after it, and the one under it");

        let added = with_tree(|t| {
            let div = t.intern("div");
            let added = t.create_element(div);
            t.insert(added, root, third);
            added
        });
        assert_eq!(roots_of(), vec![added], "only the new node is walked");
        let mut marked = with_tree(|t| std::mem::take(&mut t.self_marks));
        marked.sort_unstable();
        marked.dedup();
        assert_eq!(
            marked,
            vec![first, third],
            "the class change, and the one whose `+` steps across"
        );

        with_tree(|t| t.set_class(first, "b"));
        restyle_all();
        assert_eq!(opacity(second), None);
        with_tree(|t| t.set_class(first, "a b"));
        restyle_all();
        assert_eq!(opacity(second), Some(0.5), ".a + .b");
        with_tree(|t| t.set_class(first, "b"));
        restyle_all();
        assert_eq!(opacity(second), None);
    }

    #[test]
    fn an_uncascaded_element_on_the_trail_is_a_walk_root() {
        let (_, kids, _) = cascaded_comb(2);
        let fresh = with_tree(|t| {
            let div = t.intern("div");
            let fresh = t.create_element(div);
            let inner = t.create_element(div);
            t.insert(inner, fresh, 0);
            // Attached by hand without `insert`'s parent mark or its own mark: only the trail and
            // its missing computed map say it must be walked.
            t.node_mut(fresh).unwrap().parent = kids[0];
            t.node_mut(fresh).unwrap().mark_gen = 0;
            t.node_mut(kids[0]).unwrap().children.push(fresh);
            t.flag_path(kids[0]);
            fresh
        });
        assert_eq!(roots_of(), vec![fresh]);
    }

    fn font_size_of(id: u32) -> f32 {
        read_tree(|t| {
            t.node(id)
                .and_then(|n| n.computed.as_ref())
                .map(|c| c.font_size)
        })
        .expect("cascaded")
    }

    fn padding_top_of(id: u32) -> Option<gpui::DefiniteLength> {
        read_tree(|t| {
            t.node(id)
                .and_then(|n| n.computed.as_ref())
                .and_then(|c| c.base.padding.top)
        })
    }

    fn px_len(v: f32) -> Option<gpui::DefiniteLength> {
        Some(gpui::px(v).into())
    }

    /// The root's own `rem` is of the initial 16px, so `0.875rem` is a 14px basis and stays one:
    /// adopting what the root computes to shrank it on every flush (14, 12.25, 10.7…).
    #[test]
    fn a_root_font_size_in_rem_settles_on_the_initial_size() {
        set_stylesheet(":root { font-size: 0.875rem } .w { width: 2rem }".into());
        let w = with_tree(|t| {
            let root = t.create_root();
            let div = t.intern("div");
            let w = t.create_element(div);
            t.set_class(w, "w");
            let height = t.intern("height");
            t.set_style(w, height, "2rem".to_string());
            t.insert(w, root, 0);
            w
        });
        for pass in 0..4 {
            with_tree(|t| t.take_dirty());
            restyle_all();
            assert_eq!(sk_css::rem_basis(), 14.0, "pass {pass}");
            assert!(
                !read_tree(|t| t.dirty),
                "pass {pass} left a restyle pending"
            );
            let computed = read_tree(|t| {
                t.node(w)
                    .and_then(|n| n.computed.as_ref())
                    .map(|c| c.base.clone())
            })
            .expect("cascaded");
            let rem2 = Some(gpui::Length::Definite(gpui::px(28.0).into()));
            assert_eq!(computed.size.width, rem2, "the sheet's 2rem, pass {pass}");
            assert_eq!(computed.size.height, rem2, "the inline 2rem, pass {pass}");
        }
    }

    #[test]
    fn em_follows_the_computed_font_size_down_the_walk() {
        set_stylesheet(
            ":root { font-size: 62.5% } \
             .a { font-size: 2em } \
             .b { font-size: 75%; padding: .5em } \
             .c { margin-top: 1em; width: 2rem; line-height: 1.5em } \
             .n { line-height: 1.5 }"
                .into(),
        );
        let (root, a, b, c, n) = with_tree(|t| {
            let root = t.create_root();
            let div = t.intern("div");
            let mut make = |class: &str, parent: u32| {
                let e = t.create_element(div);
                t.set_class(e, class);
                t.insert(e, parent, 0);
                e
            };
            let a = make("a", root);
            let b = make("b", a);
            let c = make("c", b);
            let n = make("n", c);
            (root, a, b, c, n)
        });
        restyle_all();
        assert_eq!(
            sk_css::rem_basis(),
            10.0,
            "the root's 62.5% of 16px is the rem basis"
        );
        assert_eq!(
            [root, a, b, c, n].map(font_size_of),
            [10.0, 20.0, 15.0, 15.0, 15.0],
            "each size of its parent's, and inherited as px"
        );
        assert_eq!(padding_top_of(b), px_len(7.5), "`.5em` of its own 15px");
        let computed = read_tree(|t| {
            t.node(c)
                .and_then(|n| n.computed.as_ref())
                .map(|c| c.base.clone())
        })
        .expect("cascaded");
        assert_eq!(
            computed.margin.top,
            Some(gpui::Length::Definite(gpui::px(15.0).into()))
        );
        assert_eq!(
            computed.size.width,
            Some(gpui::Length::Definite(gpui::px(20.0).into())),
            "2rem"
        );
        assert_eq!(
            computed.text.line_height,
            px_len(22.5),
            "an em line-height is px"
        );
        let number = read_tree(|t| {
            t.node(n)
                .and_then(|n| n.computed.as_ref())
                .and_then(|c| c.base.text.line_height)
        });
        assert_eq!(
            number,
            Some(gpui::relative(1.5)),
            "a number line-height stays a number"
        );
        let reported = crate::dom::computed_style(c).unwrap_or_default();
        assert!(
            reported.lines().any(|l| l == "font-size=15px"),
            "getComputedStyle: {reported}"
        );
    }

    #[test]
    fn a_new_parent_font_size_reaches_the_em_below_it() {
        set_stylesheet(".big { font-size: 20px } .leaf { padding: 1em }".into());
        let (big, plain, leaves) = with_tree(|t| {
            let root = t.create_root();
            let div = t.intern("div");
            let big = t.create_element(div);
            t.set_class(big, "big");
            t.insert(big, root, 0);
            let plain = t.create_element(div);
            t.insert(plain, root, 1);
            let mut leaves = Vec::new();
            for parent in [big, plain] {
                let leaf = t.create_element(div);
                t.set_class(leaf, "leaf");
                t.insert(leaf, parent, 0);
                leaves.push(leaf);
            }
            (big, plain, leaves)
        });
        restyle_all();
        assert_eq!(
            leaves
                .iter()
                .map(|&l| padding_top_of(l))
                .collect::<Vec<_>>(),
            vec![px_len(20.0), px_len(16.0)]
        );

        with_tree(|t| {
            t.set_class(plain, "big");
            t.set_class(big, "");
        });
        restyle_all();
        assert_eq!(
            leaves
                .iter()
                .map(|&l| padding_top_of(l))
                .collect::<Vec<_>>(),
            vec![px_len(16.0), px_len(20.0)]
        );
        assert_eq!(font_size_of(leaves[1]), 20.0);
    }
    #[test]
    fn character_width_follows_inherited_face_and_restyles_descendants() {
        sk_css::set_font_source(|| vec!["SK Metric Narrow".into(), "SK Metric Wide".into()]);
        sk_css::set_font_metric_source(|font, size| {
            Some(sk_css::FontRelativeMetrics {
                zero_advance: size
                    * if font.family.as_ref() == "SK Metric Wide" {
                        0.8
                    } else {
                        0.6
                    },
                x_height: size * 0.4,
            })
        });
        struct Reset;
        impl Drop for Reset {
            fn drop(&mut self) {
                sk_css::set_font_metric_source(|_, _| None);
            }
        }
        let _reset = Reset;
        set_stylesheet(".narrow {font-family:'SK Metric Narrow';font-size:10px} .wide {font-family:'SK Metric Wide';font-size:10px} .leaf{width:1ch}".into());
        let (parent, leaves) = with_tree(|tree| {
            let root = tree.create_root();
            let tag = tree.intern("div");
            let mut parents = Vec::new();
            let mut leaves = Vec::new();
            for class in ["narrow", "wide"] {
                let parent = tree.create_element(tag);
                tree.set_class(parent, class);
                tree.insert(parent, root, 0);
                parents.push(parent);
                let leaf = tree.create_element(tag);
                tree.set_class(leaf, "leaf");
                tree.insert(leaf, parent, 0);
                leaves.push(leaf);
            }
            (parents[0], leaves)
        });
        let widths = || {
            read_tree(|tree| {
                leaves
                    .iter()
                    .map(|&id| {
                        tree.node(id)
                            .and_then(|node| node.computed.as_ref())
                            .and_then(|computed| computed.base.size.width)
                    })
                    .collect::<Vec<_>>()
            })
        };
        restyle_all();
        assert_eq!(
            widths(),
            vec![Some(gpui::px(6.0).into()), Some(gpui::px(8.0).into())]
        );
        with_tree(|tree| tree.set_class(parent, "wide"));
        restyle_all();
        assert_eq!(
            widths(),
            vec![Some(gpui::px(8.0).into()), Some(gpui::px(8.0).into())]
        );
    }
}
