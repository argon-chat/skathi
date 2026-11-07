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
// there; bits 52-63 remain free.
pub const EV_TRANSITIONEND: u32 = 35;
pub const EV_ANIMATIONSTART: u32 = 36;
pub const EV_ANIMATIONEND: u32 = 37;
pub const EV_ANIMATIONCANCEL: u32 = 38;

// The ClipboardEvents (`crate::clipboard`). No numbers in the payload: the
// event's `clipboardData` (a DataTransfer the host builds) travels as the
// dispatcher's eighth argument.
pub const EV_PASTE: u32 = 39;
pub const EV_COPY: u32 = 40;
pub const EV_CUT: u32 = 41;

// m26 fixes: the events the Argon client binds that the host never delivered.
// (`dblclick` has no code: it rides `click`'s bit, and the JS runtime fires it
// after the click whose `detail` is 2 — `render::bind_events`.)
/// A MouseEvent: after the right button's `mouseup` (Chromium on Windows), or
/// from the Menu key / Shift+F10 at the focused element. Payload as for a
/// mouse event (`crate::render`'s up wave and `crate::render::bind_menu_keys`).
pub const EV_CONTEXTMENU: u32 = 42;
/// `<img>` decoded its `src` (`crate::img::tick`): `a`/`b` = naturalWidth /
/// naturalHeight, `key` = the `src` it was for. A failure is [`EV_ERROR`] with
/// the same `key`.
pub const EV_LOAD: u32 = 43;
/// The host gave up on a press (the window lost activation mid-press): the
/// press's `mousedown` target hears it, and nothing more of that press does.
pub const EV_POINTERCANCEL: u32 = 44;
/// Document text selection (`crate::selection`): a press that starts one
/// fires `selectstart` at the element under it (cancelable, bubbles), and
/// every change owes `document` one `selectionchange`, delivered from the
/// frame tick at the root (the JS side retargets it to `document`).
pub const EV_SELECTSTART: u32 = 45;
pub const EV_SELECTIONCHANGE: u32 = 46;
/// A MouseEvent: the middle or the right button pressed and released over the
/// same element (UI Events §3.5), at the press and release targets' nearest
/// common ancestor, as `click` is for the primary button. Delivered in the
/// release's wave after `mouseup` and, for the right button, before
/// `contextmenu` (Chromium on Windows; `crate::render`'s up wave).
pub const EV_AUXCLICK: u32 = 47;
// A text field's edits (vn-elements `TextInputState`). `a` is the edit's
// `InputType` code (`InputEvent.inputType`), `b` 1 while an IME composes
// (`isComposing`), and the dispatcher's eighth argument the event's `data`
// string (`null` without one). `input` (code 9) carries the same, with the new
// value in `key`.
/// Before each edit, cancelable (except the IME's): a cancelled one makes no
/// edit and no `input`.
pub const EV_BEFOREINPUT: u32 = 48;
/// The IME composition's stages (CompositionEvents, `data` as above): start,
/// each pre-edit, and the end, which follows the commit's `input`.
pub const EV_COMPOSITIONSTART: u32 = 49;
pub const EV_COMPOSITIONUPDATE: u32 = 50;
pub const EV_COMPOSITIONEND: u32 = 51;

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
    /// The `class` attribute's text as written, for attribute selectors
    /// (`[class*="group/msg"]`): `classes` is the token list `.x` matches.
    pub class_attr: String,
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
    /// "Something in this subtree changed" — bumped on the node itself and on
    /// every ancestor by [`Tree::touch`], which every mutator calls. It is the
    /// signal the renderer's retained-render cache compares against: a cache
    /// boundary is only allowed to replay a recorded frame while the number it
    /// was rendered at still stands (docs/ARCHITECTURE.md §11, D-P01).
    ///
    /// Upwards only, and that is the whole design: a mutation can move any
    /// ancestor's box, so every ancestor's *recorded* frame is stale; it cannot
    /// move a node outside its own subtree except through the cascade, and the
    /// cascade reports those itself — the restyle pass touches every node whose
    /// `ComputedStates` actually changed, which is precisely the set a
    /// descendant combinator can reach.
    pub(crate) sub_rev: u64,
    /// The restyle generation in which this node's own cascade inputs changed:
    /// the next pass walks its whole subtree. See [`Tree::mark_restyle`].
    pub(crate) mark_gen: u32,
    /// The generation in which this node, or something under it, was marked.
    /// It is the trail the next pass follows down from the root to find the
    /// subtrees it has to walk ([`Tree::restyle_roots`]).
    pub(crate) desc_gen: u32,
    /// What the cascade found out, while matching, about which OTHER nodes
    /// decide how this one and its children match ([`sel`]). Written through
    /// the matcher's `&Tree` (hence the `Cell`), read by the mutators to know
    /// what a change to the child list has to restyle besides the subtree
    /// that moved. Sticky: a flag only ever makes a restyle bigger, so one
    /// that no longer applies costs time, never correctness.
    pub(crate) sel_flags: Cell<u8>,
    /// Everything the inline `style` body contributes: the mapped refinement,
    /// its transitions and `object-fit`, plus the `--x` declarations and
    /// `var()`-composed values that can only be resolved once the node's place
    /// in the tree is known.
    pub(crate) inline: Option<std::rc::Rc<vn_css::InlineStyle>>,
    /// The folded cascade result, filled by the restyle pass.
    pub computed: Option<vn_css::ComputedStates>,
}

/// [`Node::sel_flags`]: the dependencies the cascade records while it matches,
/// the way browsers' selector flags do (Servo's `ElementSelectorFlags`).
///
/// They are learned from the questions the matcher asks the tree
/// (`vn_css::MatchDom`), not from the sheet, because a sheet-wide answer is
/// useless on a real one: the Argon client's has `> :nth-child(7)` and
/// `>*:last-child` rules that every element is tested against, so "the sheet
/// has structural pseudos" is always true, while the elements whose match
/// actually got as far as asking are few.
///
/// The sibling flags live on the PARENT, for its child list. A question the
/// matcher asks of the element being cascaded itself is an EDGE dependence:
/// its answer is decided by the adjacent element (`:first-child`, `+`,
/// `:last-child`, `:has(+ …)`). One asked of any other node is a WALK: the
/// matcher is stepping along the siblings (`:nth-child()`, `~`), or asking on
/// an ancestor's behalf (`li:last-child > a`), and a walk is restyled as if
/// the whole run depended on it. Conservative, never wrong.
pub(crate) mod sel {
    /// A child's match reads the element right before it.
    pub const PREV_EDGE: u8 = 1;
    /// …or any number of the elements before it.
    pub const PREV_WALK: u8 = 2;
    /// A child's match reads the element right after it.
    pub const NEXT_EDGE: u8 = 4;
    /// …or any number of the elements after it.
    pub const NEXT_WALK: u8 = 8;
    pub const SIBLINGS: u8 = PREV_EDGE | PREV_WALK | NEXT_EDGE | NEXT_WALK;
    /// This node's match read whether it is empty (`:empty`).
    pub const EMPTY: u8 = 16;
    /// This node's OWN match read its subtree (`:has()` in the subject:
    /// `.card:has(> svg)`).
    pub const HAS_SELF: u8 = 32;
    /// A descendant's match read this node's subtree (`:has()` further left:
    /// `.dialog:has(.editor) .title`).
    pub const HAS_DESC: u8 = 64;
    pub const HAS: u8 = HAS_SELF | HAS_DESC;
}

thread_local! {
    /// The element the restyle pass is cascading right now, 0 outside one.
    /// The [`sel`] flags are only learned from the cascade's own questions; a
    /// `querySelector` or a devtools query asks the same ones and must not
    /// leave flags behind.
    static CASCADE_SUBJECT: Cell<u32> = const { Cell::new(0) };
    /// How deep inside [`NodeRef`]'s `each_child` the matcher is: the
    /// outermost call is the `:has()` anchor asking about its subtree, the
    /// nested ones are that same question walking down.
    static HAS_DEPTH: Cell<u32> = const { Cell::new(0) };
    /// See [`ReadOnOthers`].
    static READ_ON_OTHERS: RefCell<ReadOnOthers> = RefCell::new(ReadOnOthers::default());
    /// The `(name, reader tag)` pairs [`READ_ON_OTHERS`] already holds, by
    /// the address and length of the selector's own string: the matcher asks
    /// the same few names (`dark`, `group`, `data-state`) millions of times,
    /// and a hash per ask would cost more than the restyles this saves.
    /// Direct-mapped and exact (address, length AND tag, and only for the life
    /// of one sheet: [`set_engine`] empties it), so a miss costs a map lookup
    /// and a hit is never a different string.
    static READ_CACHE: [Cell<(usize, usize, u32)>; 128] = [const { Cell::new((0, 0, 0)) }; 128];
}

/// Every class, attribute and scope name the cascade has ever tested on an
/// element OTHER than the one it was cascading (an ancestor: `.dark .x`; a
/// sibling: `.a + .b`; a `:has()` descendant), with the tags of the elements
/// whose cascade asked; and the same for an ancestor's place among its
/// siblings ([`NameKind::Position`]).
///
/// Only a change to one of these names can move another element's match,
/// and only the match of an element with one of those tags. So a class,
/// attribute or scope change on a node, or a move of its place in the child
/// list, is restyled as narrowly as that allows ([`Tree::mark_own_inputs`],
/// [`Tree::child_list_changed`]): the node alone when no other element
/// reads the name; the node plus the elements of the reader tags in its
/// reach when a few tags read it; its whole subtree only when many do. The
/// node's subtree follows anyway if what it inherits moved
/// ([`Tree::settle_self_marks`]).
///
/// Learned while matching, like [`sel`], and for the same reason: Vue's
/// `<Transition>` classes (`argon-route-enter-active`), `is-active`,
/// `data-state` land on page-sized containers, and in the Argon client most
/// of them only ever appear in the subject position. The one exception
/// that matters is `class` itself: `.xsolla-checkout-container
/// [class*="submit-button"] button` reads the class ATTRIBUTE of every
/// ancestor of every `<button>`, which without the tags would have made
/// every class change in the document a subtree walk. Growing only, so a
/// reader is never forgotten while some selector still reads the name.
#[derive(Default)]
struct ReadOnOthers {
    names: [HashMap<Box<str>, Readers>; 4],
}

/// The tags whose cascade read one name on another element.
#[derive(Default)]
struct Readers {
    tags: Vec<u32>,
    /// Past [`MAX_READER_TAGS`]: as good as every element.
    all: bool,
}

/// More reader tags than this and a narrow walk stops paying.
const MAX_READER_TAGS: usize = 8;

/// Which of [`ReadOnOthers`]' maps a name goes to.
#[derive(Clone, Copy)]
enum NameKind {
    Class = 0,
    Attr = 1,
    Scope = 2,
    /// An element's place among its siblings (`:first-child`, `:nth-child()`
    /// and the rest), asked of an element that is neither the one being
    /// cascaded nor one of its siblings: `.row:first-child .cell`. One name,
    /// [`POSITION`]. A sibling's question needs no entry, because asking it
    /// already flagged the parent ([`sel`]), and the insert marks that sibling
    /// directly.
    Position = 3,
}

/// The one name of [`NameKind::Position`].
const POSITION: &str = "position";

/// Who a change to some names can reach besides the node itself.
#[derive(Debug, PartialEq)]
enum Reach {
    /// Nobody: restyle the node alone.
    Own,
    /// Elements with these tags, in the node's subtree (and, with `+` / `~`
    /// in the sheet, its later siblings' subtrees).
    Tags(Vec<u32>),
    /// Anything under it: walk the subtree.
    Subtree,
}

/// The matcher asked `name` of the node `asked` while cascading `subject`
/// (whose tag is `subject_tag`): remember it if those are different nodes.
fn note_name_read(asked: u32, subject: u32, subject_tag: u32, name: &str, kind: NameKind) {
    if subject == 0 || subject == asked {
        return;
    }
    let key = (name.as_ptr() as usize, name.len() * 4 + kind as usize, subject_tag);
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

/// Who a change to these names can reach ([`Reach`]).
fn reach_of<'a>(names: impl IntoIterator<Item = (&'a str, NameKind)>) -> Reach {
    READ_ON_OTHERS.with(|r| {
        let r = r.borrow();
        let mut tags: Vec<u32> = Vec::new();
        for (name, kind) in names {
            let Some(readers) = r.names[kind as usize].get(name) else { continue };
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

/// [`Tree::mem_estimate`]'s answer.
#[derive(Default, Debug, Clone)]
pub struct TreeMem {
    pub slots: usize,
    pub nodes: usize,
    pub elements: usize,
    pub texts: usize,
    pub anchors: usize,
    pub computed: usize,
    /// The slab itself (`Vec<Option<Node>>`, capacity).
    pub slot_bytes: usize,
    /// Heap a node owns directly: children, text, inline-style text, classes,
    /// attributes, scope hashes.
    pub owned_bytes: usize,
    /// `ComputedStates` structs, inline in each node.
    pub computed_bytes: usize,
    /// Distinct `Rc<StyleRefinement>`s the computed styles point at.
    pub refinements: usize,
    pub refinement_bytes: usize,
    /// Distinct custom-property maps, and their entries.
    pub vars_maps: usize,
    pub var_entries: usize,
    pub vars_bytes: usize,
    /// Distinct parsed inline styles.
    pub inlines: usize,
    pub inline_bytes: usize,
    pub names: usize,
    pub total_bytes: usize,
}

#[derive(Default)]
pub struct Tree {
    nodes: Vec<Option<Node>>,
    /// Ids [`Tree::alloc`] may hand out again.
    free: Vec<u32>,
    /// Ids freed this wave, not reusable until the side tables have been swept
    /// ([`Tree::release_freed`]).
    freed: Vec<u32>,
    pub root: u32,
    dirty: bool,
    /// The current restyle generation. A node whose [`Node::mark_gen`] equals
    /// it had its own cascade inputs changed since the last pass, and one whose
    /// [`Node::desc_gen`] does has such a node at or under it. The pass bumps
    /// the number, which clears every mark in the tree at once. Starts at 1, so
    /// a new node's zeros mean "never". See [`Tree::mark_restyle`].
    restyle_gen: u32,
    /// Parentless nodes other than the root that got a mark at or under them
    /// this generation: a node Vue has created and not inserted yet, or a
    /// `<KeepAlive>` cache container, which is never inserted. The pass looks
    /// for marks under these as well as under the root.
    detached_marks: Vec<u32>,
    /// Some node has carried [`sel::HAS`] since boot: until one does, a
    /// mutation skips the look for `:has()` anchors above it.
    has_anchors: Cell<bool>,
    /// Nodes whose own style, and only that, may have moved since the last
    /// pass ([`Tree::mark_self`]), and the [`sel::HAS_SELF`] anchors a
    /// mutation reached; see [`Tree::settle_self_marks`].
    self_marks: Vec<u32>,
    /// The last pass's sheet has a `+` / `~` combinator, so a node's later
    /// siblings can read it ([`vn_css::Engine::uses_sibling_combinators`]).
    /// Taken from the pass because the mutators have no engine; a new sheet
    /// restyles the whole document anyway, so a stale value never matters.
    sibling_combinators: bool,
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
    /// Anchor → the elements whose `group-hover:` rung watches it. A flip on
    /// the anchor is the only signal such a descendant ever gets: its rung is
    /// folded at build time from the hovered set, so a cached subtree holding
    /// it has to render again ([`Tree::touch_hover_styled`]), and one that
    /// declares a `transition` has its animation aimed ([`note_hover`]). Built
    /// in the same pass as [`Tree::group_anchors`] and, like it, only for a
    /// sheet that uses the variant at all — so the m11 invariant holds: a
    /// hover walks this list, never the tree.
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
        self.names.get(id as usize).map(String::as_str).unwrap_or("")
    }

    /// Does some descendant's `group-hover:` rule watch this element? See
    /// [`Tree::group_anchors`].
    pub fn is_group_anchor(&self, id: u32) -> bool {
        self.group_anchors.contains(&id)
    }

    /// `id` joined or left the hover path: touch what that repaints, its own
    /// `:hover` rung and the `group-hover:` elements watching it, so a cached
    /// subtree holding one renders again. `false` when nothing hangs off its
    /// hover (then no frame is owed).
    pub(crate) fn touch_hover_styled(&mut self, id: u32) -> bool {
        let own = self.node(id).and_then(|n| n.computed.as_ref()).is_some_and(|cs| cs.hover.is_some() || cs.control_state(1));
        if own {
            self.touch(id);
        }
        let deps = self.group_hover_deps.get(&id).cloned().unwrap_or_default();
        for &dep in &deps {
            self.touch(dep);
        }
        own || !deps.is_empty()
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
            class_attr: String::new(),
            props: Vec::new(),
            scopes: Vec::new(),
            listeners: 0,
            inline_dirty: false,
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
            // `:empty` reads whether a text child has data (`NodeRef::is_empty`),
            // so a text node gaining its first character, or losing its last,
            // restyles the element it sits in, looking through anchors.
            let flips = n.kind == Kind::Text && n.text.is_empty() != text.is_empty();
            let mut parent = n.parent;
            n.text = text;
            self.dirty = true;
            self.mark_restyle(id);
            if flips {
                while let Some(p) = self.node(parent).filter(|p| p.kind == Kind::Anchor) {
                    parent = p.parent;
                }
                // Only an element whose cascade actually asked (`sel::EMPTY`).
                // A text node gets its first characters on every mount, and
                // re-cascading the element around each one is what this used
                // to cost.
                if self.node(parent).is_some_and(|p| p.sel_flags.get() & sel::EMPTY != 0) {
                    self.mark_with_followers(parent);
                }
            }
            self.touch(id);
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
        // Its `:empty` answer, if anything asked, can have flipped.
        if self.node(id).is_some_and(|n| n.sel_flags.get() & sel::EMPTY != 0) {
            self.mark_with_followers(id);
        } else {
            self.mark_restyle(id);
        }
        self.touch_mutated(id, true);
    }

    /// Insert-or-move `child` under `parent`, before `anchor` (0 = append).
    ///
    /// The moved subtree is restyled whole: its ancestors changed, and a
    /// descendant combinator reads every one of them. Nothing else is, unless
    /// the cascade said it reads the child list ([`Tree::child_list_changed`]):
    /// a sibling whose `:first-child` / `:nth-child()` / `+` / `~` answer can
    /// move, a parent matched against `:empty` that stops being empty, a
    /// `:has()` above that can start or stop matching. That used to be "mark
    /// the parent", which re-cascaded every existing sibling of every inserted
    /// node: appending one message to a list re-cascaded the list.
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
        self.mark_restyle(child);
        // A subtree carrying this generation's marks has a trail up to where
        // it USED to hang (or none above itself, if it hung nowhere); the pass
        // searches from the root, so the trail has to run from here up too.
        if self.node(child).is_some_and(|c| c.desc_gen == self.restyle_gen) {
            self.flag_path(parent);
        }
        self.child_list_changed(parent, idx, true, was_bare);
        self.dirty = true;
        // BOTH chains: the subtree left one parent and joined another, and a
        // cache boundary over either has a stale recording.
        if old_parent != 0 {
            self.touch_mutated(old_parent, true);
        }
        self.touch_mutated(child, false);
    }

    /// Takes `id` out of its parent's child list; answers where it was.
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

    /// Vue calls remove exactly once per unmounted subtree root: recursively
    /// destroy native state and recycle every id.
    pub fn remove(&mut self, id: u32) {
        let parent = self.node(id).map(|n| n.parent).unwrap_or(0);
        let was_bare = self.bare_if_asked(parent);
        let at = self.detach(id);
        self.free_subtree(id);
        if parent != 0 {
            self.child_list_changed(parent, at, false, was_bare);
        }
        self.dirty = true;
        if parent != 0 {
            self.touch_mutated(parent, true);
        }
    }

    /// `parent`'s child list just changed at `at`: a child went in there
    /// (`inserted`), or the one that was there came out. Marks whatever the
    /// cascade said depends on that ([`Node::sel_flags`], recorded while
    /// matching):
    ///
    /// * `PREV_EDGE`: the first element after `at`, whose previous element
    ///   sibling is a different one now (`:first-child`, `+`);
    /// * `PREV_WALK`: every child after `at`, whose count of earlier siblings
    ///   moved (`:nth-child()`, `~`);
    /// * `NEXT_EDGE` / `NEXT_WALK`: the same, looking the other way
    ///   (`:last-child`, `:nth-last-child()`, `:has(+ …)`);
    /// * `EMPTY`: the parent itself, if it stopped or started being empty
    ///   (`was_bare` is what it was before).
    ///
    /// A sibling's own match is what moved, so it is restyled alone, plus the
    /// elements under it whose selectors read an ANCESTOR's position
    /// ([`NameKind::Position`]); its subtree is walked only when too many tags
    /// do. The client's sheet asks `:last-child` and `:nth-child()` of every
    /// element (Tailwind's `space-y-*` and friends sit in the universal
    /// bucket), so every parent carries these flags, and marking the siblings
    /// whole made a route change re-cascade the page next to the new one.
    /// No later-sibling walk is needed for `+` / `~` either: a selector that
    /// steps across siblings flags the parent with a WALK on the way, and the
    /// WALK marks every sibling it could have stepped from.
    ///
    /// A `:has()` above is [`Tree::touch_mutated`]'s business.
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
            // An inserted child is restyled whole already; "after" starts past it.
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
                marks.extend(children.iter().take(at).rev().find(|c| is_element(c)).copied());
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

    /// [`Tree::is_bare`], for a node whose cascade asked `:empty` of it, and
    /// `false` for every other: what [`Tree::child_list_changed`] compares
    /// against, without a walk of the child list on every insert.
    fn bare_if_asked(&self, id: u32) -> bool {
        self.node(id).is_some_and(|n| n.sel_flags.get() & sel::EMPTY != 0) && self.is_bare(id)
    }

    /// `:empty` as [`NodeRef::is_empty`] answers it, for the mutators: is
    /// there no element child and no text with data, looking through anchors?
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
        self.freed.push(id);
    }

    /// Is an ANCESTOR of `id` `display: none`, by its stored computed style?
    /// The restyle pass asks it of a node with animations, after storing the
    /// ancestors' new styles (it walks in preorder).
    pub(crate) fn hidden_above(&self, id: u32) -> bool {
        let mut cur = self.node(id).map_or(0, |n| n.parent);
        while cur != 0 {
            let Some(n) = self.node(cur) else { return false };
            if n.computed.as_ref().is_some_and(|c| c.base.display == Some(gpui::Display::None)) {
                return true;
            }
            cur = n.parent;
        }
        false
    }

    /// Hands the ids freed since the last call back to [`Tree::alloc`]. Called
    /// by the wave's flush after every side table keyed by node id has been
    /// swept for dead ids (`crate::flush_changes`).
    ///
    /// Until then a freed id is not reused. The side tables (the animator's
    /// runs, hover, rectangles, scroll handles, focus, image and canvas state)
    /// are swept by asking whether the id is still alive. An id freed and
    /// reallocated within one wave was alive again by the time they asked, so
    /// the new node inherited the old one's state. In the Argon client a
    /// dashboard widget's spinner (`animate-spin`, infinite) was removed when a
    /// space opened, a Vue anchor got its id in the same wave, and the animator
    /// kept the window redrawing every frame for good, idle. The
    /// same reuse handed a node mounted and measured in one wave the old
    /// node's rectangle instead of a layout.
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
            // An inline style is no selector's business: only this element's
            // own style, and what its subtree inherits from it, can move.
            self.mark_self(id);
            self.touch(id);
        }
    }

    pub fn set_prop(&mut self, id: u32, key: u32, value: Option<String>) {
        let reach = reach_of([(self.name(key), NameKind::Attr)]);
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
            self.mark_own_inputs(id, reach);
            self.touch_mutated(id, false);
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

    pub fn set_class(&mut self, id: u32, classes: &str) {
        if let Some(n) = self.node_mut(id) {
            if n.class_attr == classes {
                return; // `el.className = el.className`: nothing moved.
            }
            // An ordered set, as `classList` is (DOM §7.1, DOMTokenList); the
            // raw string stays in `class_attr` for `[class*=…]`. Lucide's icons
            // arrive as `lucide w-4 h-4 … w-4 h-4`, and the rule index offers a
            // class's rules once per entry (`MatchDom::each_class`), so a
            // repeat would match and fold them twice.
            let mut next: Vec<String> = Vec::new();
            for c in classes.split_whitespace() {
                if !next.iter().any(|x| x == c) {
                    next.push(c.to_string());
                }
            }
            // Only the names that came or went can move a match, plus the
            // attribute itself for `[class*=…]`.
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

    /// A class, attribute or scope of `id` changed; `reach` says whose match
    /// can have moved besides its own ([`ReadOnOthers`]). The node itself is
    /// restyled alone ([`Tree::mark_self`]); the elements of the reader tags
    /// in its reach are marked for a walk each; and when the readers are too
    /// many to be worth finding, the node's whole subtree is walked, as every
    /// such change used to be.
    ///
    /// "In its reach" is the node's subtree, and, when the sheet has `+` /
    /// `~`, its later siblings and their subtrees: everywhere a selector that
    /// reads this node from another element can be anchored. (A `:has()`
    /// above it is [`Tree::touch_mutated`]'s business.)
    fn mark_own_inputs(&mut self, id: u32, reach: Reach) {
        self.mark_reach(id, &reach, self.sibling_combinators);
    }

    /// [`Tree::mark_own_inputs`] for any change whose readers `reach` names:
    /// `id` alone, plus the reader elements in its subtree, and in its later
    /// siblings' subtrees with `later_siblings` (the subtree and the later
    /// siblings whole, for [`Reach::Subtree`]).
    fn mark_reach(&mut self, id: u32, reach: &Reach, later_siblings: bool) {
        let tags = match reach {
            Reach::Subtree if later_siblings => return self.mark_with_followers(id),
            Reach::Subtree => return self.mark_restyle(id),
            Reach::Own => return self.mark_self(id),
            Reach::Tags(tags) => tags,
        };
        self.mark_self(id);
        let mut stack: Vec<u32> = self.node(id).map(|n| n.children.clone()).unwrap_or_default();
        if later_siblings {
            if let Some(siblings) = self.node(id).and_then(|n| self.node(n.parent)).map(|p| &p.children) {
                if let Some(at) = siblings.iter().position(|&c| c == id) {
                    stack.extend_from_slice(&siblings[at + 1..]);
                }
            }
        }
        let mut hits: Vec<u32> = Vec::new();
        while let Some(c) = stack.pop() {
            let Some(n) = self.node(c) else { continue };
            if n.kind == Kind::Element && tags.contains(&n.tag) {
                // Walked whole: everything under it is covered.
                hits.push(c);
                continue;
            }
            stack.extend_from_slice(&n.children);
        }
        for c in hits {
            self.mark_restyle(c);
        }
    }

    /// "Only this element's own style can have moved": restyled alone by the
    /// next pass, and its subtree walked only if what the subtree inherits
    /// from it moved ([`Tree::settle_self_marks`]).
    fn mark_self(&mut self, id: u32) {
        if self.dirty_all || id == 0 || self.node(id).is_none_or(|n| n.mark_gen == self.restyle_gen) {
            return;
        }
        self.self_marks.push(id);
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
            self.touch(id);
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

    /// What the retained tree holds, estimated from the live structures
    /// (devtools `mem`). Shallow sizes plus the heap capacity of every
    /// container a node owns; an `Rc` shared between nodes is counted once (by
    /// pointer). The estimate leaves out allocator overhead and the inside of
    /// a `StyleRefinement` (its few boxed fields), so it is a floor.
    pub(crate) fn mem_estimate(&self) -> TreeMem {
        use std::collections::HashSet;
        use std::mem::size_of;
        let mut m = TreeMem { slots: self.nodes.len(), slot_bytes: self.nodes.capacity() * size_of::<Option<Node>>(), ..TreeMem::default() };
        let mut refinements: HashSet<*const gpui::StyleRefinement> = HashSet::new();
        let mut vars: HashSet<*const vn_css::Vars> = HashSet::new();
        let mut inlines: HashSet<*const vn_css::InlineStyle> = HashSet::new();
        let note_ref = |r: &std::rc::Rc<gpui::StyleRefinement>, set: &mut HashSet<*const gpui::StyleRefinement>| {
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
            own += n.style.capacity() * size_of::<(u32, String)>() + n.style.iter().map(|(_, v)| v.capacity()).sum::<usize>();
            own += n.classes.capacity() * size_of::<String>() + n.classes.iter().map(|c| c.capacity()).sum::<usize>();
            own += n.props.capacity() * size_of::<(u32, String, u32)>() + n.props.iter().map(|(_, v, _)| v.capacity()).sum::<usize>();
            own += n.scopes.capacity() * size_of::<String>() + n.scopes.iter().map(|s| s.capacity()).sum::<usize>();
            m.owned_bytes += own;
            if let Some(i) = &n.inline {
                inlines.insert(std::rc::Rc::as_ptr(i));
            }
            if let Some(cs) = &n.computed {
                m.computed += 1;
                note_ref(&cs.base, &mut refinements);
                for r in [&cs.hover, &cs.active, &cs.focus, &cs.focus_visible, &cs.focus_within, &cs.placeholder, &cs.selection, &cs.file_selector_button]
                    .into_iter()
                    .flatten()
                {
                    note_ref(r, &mut refinements);
                }
                for (_, r) in [&cs.group_hover, &cs.group_active].into_iter().flatten() {
                    note_ref(r, &mut refinements);
                }
                if vars.insert(std::rc::Rc::as_ptr(&cs.vars)) {
                    m.var_entries += cs.vars.len();
                }
            }
        }
        m.computed_bytes = m.computed * size_of::<vn_css::ComputedStates>();
        m.refinements = refinements.len();
        m.refinement_bytes = refinements.len() * (size_of::<gpui::StyleRefinement>() + 16);
        m.vars_maps = vars.len();
        // An entry is two `Rc<str>` fat pointers; the strings themselves are
        // shared down the tree and not counted.
        m.vars_bytes = vars.len() * (size_of::<vn_css::Vars>() + 16) + m.var_entries * 2 * size_of::<std::rc::Rc<str>>();
        m.inlines = inlines.len();
        m.inline_bytes = inlines.len() * (size_of::<vn_css::InlineStyle>() + 16);
        m.names = self.names.len();
        m.total_bytes = m.slot_bytes + m.owned_bytes + m.computed_bytes + m.refinement_bytes + m.vars_bytes + m.inline_bytes;
        m
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

    /// "Something inside this subtree changed": bumps [`Node::sub_rev`] on `id`
    /// and on every ancestor up to the root.
    ///
    /// One `u64` add per level, and the depth of a DOM is a small number — a
    /// Vue wave of a thousand mutations under one parent costs a thousand times
    /// the depth, which is nothing beside the cascade it is about to run. It is
    /// deliberately NOT change-gated: a mutator that stored the same string
    /// again still moved the DOM as far as `MutationObserver` is concerned, and
    /// a cache that replayed through it would be betting on a comparison the
    /// caller already declined to make.
    pub(crate) fn touch(&mut self, id: u32) {
        let mut cur = id;
        // A cycle cannot happen (`insert` detaches first), but a corrupt id
        // must not hang the pump.
        for _ in 0..4096 {
            let Some(n) = self.node_mut(cur) else { return };
            n.sub_rev = n.sub_rev.wrapping_add(1);
            match n.parent {
                0 => return,
                p => cur = p,
            }
        }
    }

    /// [`Tree::touch`], for a mutation that can change how a selector matches
    /// (child list, classes, attributes, scopes): on the same walk up, every
    /// `:has()` anchor above is told, since its answer reads the subtree that
    /// just changed. One whose answer a DESCENDANT's selector reads
    /// ([`sel::HAS_DESC`]) is marked, subtree and all; one only its own match
    /// reads ([`sel::HAS_SELF`]) is rechecked alone first
    /// ([`Tree::settle_self_marks`]). `id` itself counts when the change is
    /// to its child list (`include_self`), not when it is to its own
    /// attributes: `:has()` reads descendants.
    ///
    /// Until some node has ever been an anchor this is exactly `touch`.
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
            match parent {
                0 => break,
                p => cur = p,
            }
        }
        for a in whole {
            self.mark_with_followers(a);
        }
    }

    /// The subtree revision — see [`Node::sub_rev`]. `0` for a node that does
    /// not exist, which is also the revision of a never-touched one: a caller
    /// comparing it must therefore hold a node it knows is alive.
    pub fn sub_rev(&self, id: u32) -> u64 {
        self.node(id).map(|n| n.sub_rev).unwrap_or(0)
    }

    /// "Walk this element's whole subtree next pass": it was inserted or moved
    /// (its ancestors changed, and a descendant combinator reads every one of
    /// them), what it hands down to its children moved
    /// ([`Tree::settle_self_marks`]), or something about it changed that too
    /// many elements under it read ([`Tree::mark_own_inputs`]). A change that
    /// only moves the element's own style is [`Tree::mark_self`]'s.
    ///
    /// Ancestors and siblings are marked only where the cascade recorded that
    /// they read this node ([`sel`]): a change to a child list marks the
    /// siblings and the `:empty` parent that asked ([`Tree::child_list_changed`]),
    /// and any mutation marks the `:has()` anchors above it
    /// ([`Tree::touch_mutated`]). What reaches the following siblings through
    /// `+` / `~` is marked by whoever knows it can ([`Tree::mark_with_followers`]).
    ///
    /// **There is no limit on how many.** A mark is two numbers on the node:
    /// [`Node::mark_gen`] on the node itself, and [`Node::desc_gen`] on it and
    /// up its ancestors to the first one that already carries it. A wave of N
    /// marks therefore costs O(N) plus one path to the root, and the pass finds
    /// its walk roots by following the trail down instead of comparing marks
    /// against each other. That bookkeeping used to be a list, which had to
    /// give up at 64 entries and restyle the whole document instead; opening a
    /// space in the Argon client crossed it 9–10 times per open
    /// (research/reports/m26/perf-mem-1.md §4).
    fn mark_restyle(&mut self, id: u32) {
        if self.dirty_all || id == 0 {
            return;
        }
        let gen = self.restyle_gen;
        match self.node_mut(id) {
            Some(n) if n.mark_gen != gen => n.mark_gen = gen,
            // Gone, or already marked this generation.
            _ => return,
        }
        self.flag_path(id);
    }

    /// [`Tree::mark_restyle`] for `id` and, when the sheet has `+` / `~`, for
    /// every sibling after it: for a change to something about `id` that the
    /// cascade does not say who read (its `:empty` or `:has()` answer, or a
    /// name every tag reads). That is exactly how far a sibling combinator can
    /// see, and sheets without one pay nothing.
    fn mark_with_followers(&mut self, id: u32) {
        self.mark_restyle(id);
        if !self.sibling_combinators {
            return;
        }
        let later: Vec<u32> = self
            .node(id)
            .and_then(|n| self.node(n.parent))
            .map(|p| p.children.iter().skip_while(|&&c| c != id).skip(1).copied().collect())
            .unwrap_or_default();
        for s in later {
            self.mark_restyle(s);
        }
    }

    /// Lays this generation's trail from `id` up to the root, stopping at the
    /// first node that already carries it: everything above that one does too,
    /// because every trail is laid all the way up ([`Tree::insert`] lays it again
    /// when it moves a marked subtree). A parentless top other than the root is
    /// remembered in [`Tree::detached_marks`], so the pass searches under it.
    fn flag_path(&mut self, id: u32) {
        let gen = self.restyle_gen;
        let root = self.root;
        let mut cur = id;
        // A cycle cannot happen (`insert` detaches first), but a corrupt id
        // must not hang the pump.
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

    /// The viewport basis moved: every inline `style` body that names a
    /// viewport unit is re-parsed, and its subtree re-cascaded.
    ///
    /// vn-css flattens `vw`/`vh`/`svw`/`svh`/… to px when it PARSES, and an
    /// inline body is parsed only when its text changes (`set_style`). The
    /// sheet has [`restyle_all`]'s recompile for this. Inline styles had
    /// nothing, so a Vue-bound `width: 100svw` stayed at the size the window
    /// had when it was first written: the Argon client's AppShell measured
    /// 1652x633 in a 1700x800 window (m26).
    ///
    /// The subtree is marked as well as the node, which covers the
    /// `var()`-composed values: a custom property declared inline
    /// (`--h: 100svh`) is resolved where a descendant's `var(--h)` is
    /// cascaded, so those descendants are re-cascaded with it.
    ///
    /// A scan of the slab, but only on a basis change, which is a resize.
    /// Returns how many nodes were marked.
    pub(crate) fn invalidate_viewport_inline(&mut self) -> usize {
        let hits: Vec<u32> = self
            .nodes
            .iter()
            .enumerate()
            .filter_map(|(i, n)| {
                let n = n.as_ref()?;
                n.style.iter().any(|(_, v)| has_viewport_unit(v)).then_some(i as u32)
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

    /// A web font was registered: every inline `style` body that sets
    /// `font-family` (or the `font` shorthand) is re-parsed, because vn-css
    /// picks the family it draws with when it PARSES, from the fonts installed
    /// then — the inline half of [`recompile_for_fonts`]. Returns how many
    /// nodes were marked.
    pub(crate) fn invalidate_font_inline(&mut self) -> usize {
        let keys: Vec<u32> = ["font", "font-family"].iter().filter_map(|k| self.name_ids.get(*k).copied()).collect();
        if keys.is_empty() {
            return 0;
        }
        let hits: Vec<u32> = self
            .nodes
            .iter()
            .enumerate()
            .filter_map(|(i, n)| {
                let n = n.as_ref()?;
                n.style.iter().any(|(k, _)| keys.contains(k)).then_some(i as u32)
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

    /// The subtree roots this pass has to walk. No two of them overlap, and no
    /// root's parent is recomputed by the pass, so each one inherits from a
    /// parent whose computed map is current, in any order.
    ///
    /// `dirty_all` (a new sheet, or a changed `@media` environment) answers
    /// "the whole document": the mounted root plus every parentless node, since
    /// a `<KeepAlive>` cache container is never inserted and its contents still
    /// need styles for the frame they are re-attached in.
    ///
    /// Otherwise the search follows this generation's trail ([`Node::desc_gen`])
    /// down from the root and from every parentless node that got a mark
    /// ([`Tree::detached_marks`]), and visits nothing else:
    ///
    /// * a marked node is a root, and nothing under it is looked at, because
    ///   its walk covers it;
    /// * so is an element that has never been cascaded: a freshly mounted
    ///   subtree is never entered with a parent map that does not exist yet;
    /// * a text or anchor node has no map to hand down either, so a root found
    ///   under one is promoted to it.
    ///
    /// What `+` / `~` can see is the marking's business, not the search's: the
    /// siblings a mutation reaches across are marked where it happens
    /// ([`Tree::child_list_changed`], [`Tree::mark_with_followers`]).
    ///
    /// The cost is the trail plus the children of the nodes on it, however
    /// many marks there are.
    fn restyle_roots(&mut self) -> Vec<u32> {
        let gen = self.restyle_gen;
        // Every mark of this generation is spent the moment the roots are
        // known. Skipping 0 keeps a new node's zeros meaning "never".
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
        // Most entries of `detached` are nodes Vue created this wave and has
        // inserted since; those are found from the root, along the trail
        // `insert` laid. What is left is the root itself and the subtrees that
        // still hang nowhere, a handful at most.
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

    /// One step of [`Tree::restyle_roots`]'s search, at a node on this
    /// generation's trail. Pushes the walk roots found under `id`, or answers
    /// `true` when `id` itself has to be walked whole, which its caller then
    /// records (or passes further up, if the caller is no element).
    fn collect_roots(&self, id: u32, gen: u32, roots: &mut Vec<u32>, depth: u32) -> bool {
        let Some(n) = self.node(id) else { return false };
        // The depth bound only matters for a corrupt tree; walking the whole
        // subtree is always correct, just slower.
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
                // A walk inherits from its root's PARENT, and a text or anchor
                // node has no computed map to give. The walk starts here, or
                // higher, and covers everything found under here so far.
                roots.truncate(start);
                return true;
            }
            roots.push(c);
        }
        false
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
        let states = engine.compute_node_at(
            NodeRef { tree: self, id: self.root },
            inline,
            &std::rc::Rc::new(vn_css::Vars::default()),
            None,
            ROOT_PARENT_FONT_SIZE,
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

    /// What a walk starting at `r` inherits: the custom properties, the
    /// `pointer-events: none`, the `color` and the corner radii of the nearest
    /// ancestor that has a computed map (the parent, for every walk root
    /// [`Tree::restyle_roots`] hands out; a text or anchor node in between
    /// passes everything through unchanged), and the `@media` environment.
    ///
    /// **Which window's environment?** (docs/MULTIWINDOW.md §6.) An element
    /// knows its window implicitly: it hangs under exactly one painted root,
    /// and each painted root belongs to exactly one window. So the walk root
    /// resolves it once for the whole subtree. The conditions (`min-width`,
    /// `prefers-color-scheme`, `display-mode`, `resolution`,
    /// `env(titlebar-area-*)`) are already per-`Env` PARAMETERS inside
    /// `compute_node`, so this costs one map lookup per walk root and nothing
    /// per node, and it is free with no pop-out open: `window_env`
    /// short-circuits to the published env.
    #[allow(clippy::type_complexity)]
    fn walk_inputs(
        &self,
        r: u32,
        env: &vn_css::Env,
        root_vars: &std::rc::Rc<vn_css::Vars>,
    ) -> (std::rc::Rc<vn_css::Vars>, bool, Inherited, vn_css::InheritedRadii, vn_css::Env) {
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
        let inherited = parent_computed.map(|c| c.vars.clone()).unwrap_or_else(|| root_vars.clone());
        let inherited_inert = parent_computed.is_some_and(|c| c.extras.pointer_events_none == Some(true));
        // `currentColor` resolves against the element's computed `color`,
        // which is inherited when it declares none — so the effective colour
        // rides down the walk beside the vars map.
        // So does the computed font size, which every `em` below resolves
        // against.
        let inherited_text = Inherited {
            color: parent_computed.and_then(|c| c.color),
            font_size: parent_computed.map_or(ROOT_PARENT_FONT_SIZE, |c| c.font_size),
            font: parent_computed.map(|c| c.computed_font.clone()).unwrap_or_default(),
        };
        let inherited_radii = parent_computed.map(|c| vn_css::InheritedRadii::of(&c.base)).unwrap_or_default();
        (inherited, inherited_inert, inherited_text, inherited_radii, root_env)
    }

    /// One element's cascade, as the walk runs it: `compute_node` with the
    /// inherited inputs, then the two inherited properties folded in. Answers
    /// the states and the radii its children inherit.
    fn cascade_one(
        &self,
        engine: &vn_css::Engine,
        id: u32,
        (parent_vars, parent_inert, parent_text, parent_radii): (
            &std::rc::Rc<vn_css::Vars>,
            bool,
            Inherited,
            vn_css::InheritedRadii,
        ),
        env: &vn_css::Env,
        diagnostics: &mut Vec<String>,
    ) -> (vn_css::ComputedStates, vn_css::InheritedRadii) {
        let n = self.node(id).expect("cascade_one on a live node");
        // The matcher's sibling and content questions are recorded against
        // this element ([`sel`]).
        CASCADE_SUBJECT.with(|c| c.set(id));
        let mut states = engine.compute_node_with_font(
            NodeRef { tree: self, id },
            n.inline.as_deref(),
            parent_vars,
            parent_text.color,
            parent_text.font_size,
            &parent_text.font,
            env,
            diagnostics,
        );
        CASCADE_SUBJECT.with(|c| c.set(0));
        // `None` = the element declared nothing, so it takes the parent's
        // value; an explicit `pointer-events: auto` (`Some(false)`) is how a
        // descendant opts back IN, which is what reka's
        // `data-[state=open]:pointer-events-auto` overlay relies on.
        let inert = states.extras.pointer_events_none.unwrap_or(parent_inert);
        states.extras.pointer_events_none = Some(inert);
        let radii = vn_css::resolve_inherited_radii(&mut states, parent_radii);
        (states, radii)
    }

    /// The nodes whose own style, and nothing else's, may have moved
    /// ([`Tree::mark_self`]): a class, attribute or scope no selector reads on
    /// another element, an inline style, and the `:has()` anchors only their
    /// own match reads the subtree of ([`sel::HAS_SELF`]).
    ///
    /// Each is cascaded alone, against what it would inherit. A result equal
    /// to the one it has is dropped; one that differs only in what the node
    /// itself paints is returned, to be stored with the pass; and one that
    /// changes what its children inherit (custom properties, `color`, font
    /// size, `pointer-events`, corner radii) turns into an ordinary mark, subtree
    /// and all. Run before [`Tree::restyle_roots`], so a node that also lies
    /// under a walk root of the same pass is simply walked, and its result
    /// here thrown away by the caller.
    ///
    /// This is what keeps a Vue `<Transition>` on a page-sized container from
    /// re-cascading the page three times (`*-enter-from`, `*-enter-active`,
    /// `*-enter-to`), and `body:has([data-vaul-drawer])`, whose anchor is the
    /// document root, from making every mutation a whole-document walk.
    fn settle_self_marks(&mut self, engine: &vn_css::Engine, env: &vn_css::Env) -> Vec<(u32, vn_css::ComputedStates)> {
        let mut marks = std::mem::take(&mut self.self_marks);
        if marks.is_empty() || self.dirty_all {
            return Vec::new();
        }
        marks.sort_unstable();
        marks.dedup();
        let root_vars = std::rc::Rc::new(vn_css::Vars::default());
        let mut diagnostics = Vec::new();
        let mut stored = Vec::new();
        let mut whole = Vec::new();
        for id in marks {
            let Some(n) = self.node(id) else { continue };
            // Walked whole this pass anyway, or never cascaded (then it is a
            // walk root of its own).
            if n.kind != Kind::Element || n.mark_gen == self.restyle_gen || n.computed.is_none() {
                continue;
            }
            let (vars, inert, text, radii, here_env) = self.walk_inputs(id, env, &root_vars);
            let (states, child_radii) =
                self.cascade_one(engine, id, (&vars, inert, text, radii), &here_env, &mut diagnostics);
            let Some(old) = n.computed.as_ref() else { continue };
            if *old == states {
                continue;
            }
            let hands_down_the_same = old.vars == states.vars
                && old.color == states.color
                && old.font_size == states.font_size
                && old.computed_font == states.computed_font
                && old.extras.pointer_events_none == states.extras.pointer_events_none
                && vn_css::InheritedRadii::of(&old.base) == child_radii;
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

    fn restyle_with(&mut self, engine: &vn_css::Engine, env: &vn_css::Env) {
        // [`READ_CACHE`] is keyed by this sheet's string addresses; a pass
        // against a different engine starts it over ([`set_engine`] covers
        // the installed one, whose address never changes).
        thread_local! { static CACHE_ENGINE: Cell<usize> = const { Cell::new(0) }; }
        let addr = engine as *const vn_css::Engine as usize;
        if CACHE_ENGINE.with(|c| c.replace(addr)) != addr {
            forget_read_cache();
            note_parent_keys(engine);
        }
        let sheet_changed = REBIND_ANIMATIONS.with(|pending| pending.replace(false));
        self.sibling_combinators = engine.uses_sibling_combinators();
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
        let full = self.dirty_all;
        let settled = self.settle_self_marks(engine, env);
        let roots = self.restyle_roots();
        let cascaded_anything = !roots.is_empty() || !settled.is_empty();
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
            //
            // And the parent's corner radii, for a `border-radius: inherit`
            // (`vn_css::resolve_inherited_radii`).
            let mut stack: Vec<(
                u32,
                std::rc::Rc<vn_css::Vars>,
                bool,
                Inherited,
                vn_css::InheritedRadii,
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
                let (inherited, inherited_inert, inherited_text, inherited_radii, root_env) =
                    this.walk_inputs(r, env, &root_vars);
                stack.push((r, inherited, inherited_inert, inherited_text, inherited_radii, root_env));
                while let Some((id, parent_vars, parent_inert, parent_text, parent_radii, here_env)) =
                    stack.pop()
                {
                    let Some(n) = this.node(id) else { continue };
                    // Text and anchor nodes have no cascade of their own; they
                    // simply pass the map down to whatever hangs under them.
                    let (child_vars, child_inert, child_text, child_radii) = if n.kind == Kind::Element {
                        let (states, radii) = this.cascade_one(
                            engine,
                            id,
                            (&parent_vars, parent_inert, parent_text.clone(), parent_radii),
                            &here_env,
                            &mut diagnostics,
                        );
                        let inert = states.extras.pointer_events_none == Some(true);
                        let vars = states.vars.clone();
                        let text = Inherited { color: states.color, font_size: states.font_size, font: states.computed_font.clone() };
                        out.push((id, states));
                        (vars, inert, text, radii)
                    } else {
                        (parent_vars, parent_inert, parent_text, parent_radii)
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
                        stack.push((
                            c,
                            child_vars.clone(),
                            child_inert,
                            child_text.clone(),
                            child_radii,
                            child_env,
                        ));
                    }
                }
            }
            out
        };
        // The nodes restyled alone ([`Tree::settle_self_marks`]), unless a walk
        // of this pass reached them too: its result is the one computed with
        // what the walk's ancestors hand down NOW.
        let computed = if settled.is_empty() {
            computed
        } else {
            let walked: std::collections::HashSet<u32> = computed.iter().map(|(id, _)| *id).collect();
            let mut computed = computed;
            computed.extend(settled.into_iter().filter(|(id, _)| !walked.contains(id)));
            computed
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
        // Every node whose fold actually MOVED. The cascade is the one way a
        // change at one node can reach a node that is not its descendant in the
        // mutation sense (a descendant combinator, an inherited `var()`, a
        // structural pseudo), so the renderer's cache learns about those from
        // here rather than from the mutator — see [`Tree::touch`].
        let mut restyled: Vec<u32> = Vec::new();
        // The cascade switches environments at a painted root and this loop does
        // not walk the tree, so the window has to be asked for again — but only
        // for a node that is actually about to start an animation, and only
        // when a pop-out exists at all. Both gates are free in the single-window
        // case: `any_secondary` is an `is_empty`, and a sheet with no
        // `@keyframes` never reaches the second one.
        let multiwindow = crate::windows::any_secondary();
        // Nodes whose `display` went to or from `none` in this pass.
        let mut display_flips: Vec<(u32, bool)> = Vec::new();
        ANIMATOR.with(|a| {
            let mut animator = a.borrow_mut();
            for (id, states) in computed {
                // Before `node_mut` takes the tree: `env_for_node` reads it.
                let node_env = if multiwindow && (has_keyframes || !states.animations.is_empty()) {
                    crate::windows::env_for_node(self, id, env)
                } else {
                    *env
                };
                // A stylesheet replacement can change the referenced block
                // without changing any computed declaration. A pop-out can
                // need the same rebinding against its own media environment.
                // **A node painted by a pop-out can need its animations
                // re-bound while its own computed style has not moved at all.**
                // The element's declarations are identical in both windows; it
                // is the `@keyframes` block its `animation-name` refers to that
                // is declared per window. The change-gate below therefore never
                // fires for it — and the run it started with, on the frame
                // before its window had painted and `ROOTS` was still empty,
                // was resolved against the PRIMARY's environment.
                let rebind = (sheet_changed || node_env != *env) && !states.animations.is_empty();
                // CSS Animations §3: nothing animates on an element that is
                // `display: none`, or under one. Its runs are cancelled (with
                // `animationcancel`) and none start; showing it again starts
                // them afresh. Before `node_mut`: the walk up reads the tree,
                // whose ancestors this preorder pass has already stored.
                let self_none = states.base.display == Some(gpui::Display::None);
                let animated = !states.animations.is_empty() || animator.has_animations(id);
                let hidden = animated && (self_none || self.hidden_above(id));
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
                        if !hidden && (has_keyframes || !states.animations.is_empty() || animator.has_animations(id)) {
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
                        restyled.push(id);
                    } else if rebind && !hidden {
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
            // A `display` flip reaches the whole subtree, including the nodes
            // whose own style did not move and so were not in this pass:
            // hiding cancels every run under the node, and showing starts the
            // runs of every descendant that is not itself hidden. (The node
            // itself was handled above.)
            for (id, none) in std::mem::take(&mut display_flips) {
                if !none && self.hidden_above(id) {
                    continue; // shown, but still under a hidden ancestor
                }
                let mut stack: Vec<u32> = self.node(id).map(|n| n.children.clone()).unwrap_or_default();
                while let Some(c) = stack.pop() {
                    let Some(node) = self.node(c) else { continue };
                    if none {
                        if animator.has_animations(c) {
                            animator.cancel_animations(c, now);
                        }
                    } else {
                        let Some(cs) = node.computed.as_ref() else { continue };
                        if cs.base.display == Some(gpui::Display::None) {
                            continue; // its subtree stays hidden
                        }
                        if !cs.animations.is_empty() {
                            let node_env =
                                if multiwindow { crate::windows::env_for_node(self, c, env) } else { *env };
                            animator.on_computed_animations(c, cs, now, &|name| {
                                engine.keyframes_for(
                                    name,
                                    &cs.vars,
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
                        // Every one: the rung is folded at build time, so a
                        // flip on the anchor has to reach each of them.
                        self.group_hover_deps.entry(*key as u32).or_default().push(i as u32);
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
    /// one text per element, in document order — see [`sync_runtime_sheets`].
    static RUNTIME_CSS: RefCell<Vec<String>> = const { RefCell::new(Vec::new()) };
    static CSS_WARNED: RefCell<std::collections::HashSet<String>> =
        RefCell::new(std::collections::HashSet::new());
    static ENGINE: RefCell<Option<vn_css::Engine>> = const { RefCell::new(None) };
    /// The text the current engine was compiled from, kept so the sheet can be
    /// compiled AGAIN against a different `rem` basis — see [`restyle_all`].
    static SHEET_SOURCE: RefCell<String> = const { RefCell::new(String::new()) };
    /// The `<style>` texts the runtime keeps for `document.head`, which are not
    /// nodes of the tree: they come with the sheet ([`set_stylesheet_with_head`])
    /// and cascade right after it.
    static HEAD_SHEETS: RefCell<Vec<String>> = const { RefCell::new(Vec::new()) };
    static ANIMATOR: RefCell<vn_css::Animator> = RefCell::new(vn_css::Animator::new());
    /// A replaced sheet can change a keyframes block without changing the
    /// element's computed declarations. Re-resolve those blocks next cascade,
    /// retaining each same-name animation's clock.
    static REBIND_ANIMATIONS: Cell<bool> = const { Cell::new(false) };
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

/// Animated non-layout CSS values, including var()-dependent background paint.
pub fn animated_extras(id: u32, now: std::time::Instant) -> Option<vn_css::Extras> {
    ANIMATOR.with(|animator| animator.borrow().animated_extras(id, now))
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

/// `id` joined (`true`) or left the hover path: the topmost element under the
/// pointer and its ancestors, resolved once a frame by
/// [`crate::pointer::resolve_hover`]. [`HOVERED`] is that path, and
/// [`crate::render::apply_computed_style`] folds the `:hover` and
/// `group-hover:` rungs from it.
///
/// This is also the whole state-transition story on the input side. In a
/// browser a `transition` on the BASE rule animates the changes a `:hover`
/// rule makes; a rung refined over the computed style has no clock at all, so
/// a hover SNAPPED (m7 recorded this as "state-rule transitions rejected by
/// design", which was about a `transition` declared ON the state rule; the
/// base-rule case is the one authors actually write and it was snapping too).
/// So the renderer strips the transitioned properties out of the rung, and this
/// routes the flip into the animator instead. What the author did NOT name in
/// `transition` stays in the rung and still snaps — which is also what CSS
/// says.
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

/// Does `:hover` match `id` right now? True for the topmost element under the
/// pointer and every ancestor of it, as of the last frame's resolution
/// ([`note_hover`]).
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
/// One honest gap, structural: `:active` is refined by gpui at paint time off
/// a hitbox we cannot read back, so a pressed element reports its unpressed
/// style here. Introduced for `vn-test` (docs/TESTING.md); nothing on the
/// paint path calls it.
pub fn live_style(id: u32) -> Option<gpui::StyleRefinement> {
    use gpui::Refineable as _;
    let cs = read_tree(|t| t.node(id).and_then(|n| n.computed.clone()))?;
    let mut live = (*cs.base).clone();
    if let Some(overlay) = animation_overlay(id, &cs.base, std::time::Instant::now()) {
        live.refine(&overlay);
    }
    // Same order as the renderer's fold. `pointer-events: none` does not keep
    // a rung off: an element that cannot be hit is still hovered while a
    // descendant that can be is.
    if let Some(rung) = fold_hover_target(id, &cs) {
        match vn_css::strip_transitioned(&rung, &cs.transitions) {
            Some(stripped) => live.refine(&stripped),
            None => live.refine(&rung),
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

/// Every node whose painted style the animator is moving at `now`, and every
/// ancestor of each, into `out` — the subtrees the retained-render cache must
/// not replay this frame (`cache::begin_frame`). One walk up per moving node,
/// stopping at the first ancestor already collected.
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

// ---------------------------------------------------------------------------
// script animations: `Element.animate()`
// ---------------------------------------------------------------------------
//
// The painting half lives in vn-css (animator/script.rs) and the object model
// in packages/vue-native/src/waapi.ts; these are the four doors between them,
// called from the `__vn_anim_*` host functions. Each one moves what a node
// paints without moving its style, so each touches the node (the render cache
// must not replay it) and asks for a draw ([`take_script_dirty`]) — a paused
// animation that is seeked is not "animating", and nothing else would.

thread_local! {
    static SCRIPT_DIRTY: Cell<bool> = const { Cell::new(false) };
    /// The instant `performance.now()` reads 0 at, taken once.
    static TIMELINE_ORIGIN: Cell<Option<std::time::Instant>> = const { Cell::new(None) };
}

/// Hands the animator the document timeline's zero, the first time a script
/// animation needs it. redjs keeps ONE clock for `performance.now()`, rAF
/// timestamps and `redjs_now`, so its origin is a fixed `Instant`.
fn ensure_timeline_origin(animator: &mut vn_css::Animator) {
    if TIMELINE_ORIGIN.with(|o| o.get()).is_some() {
        return;
    }
    let now = std::time::Instant::now();
    let ms = unsafe { vn_runtime::redjs::ffi::redjs_now() };
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

/// Creates or re-targets script animation `id`'s effect. The keyframes are
/// compiled against `node`'s custom properties (a value may be `var(--x)`).
pub(crate) fn script_anim_set(
    id: u32,
    node: u32,
    frames: &[vn_css::ScriptFrame],
    timing: vn_css::ScriptTiming,
) -> vn_css::ScriptKeyframeNotes {
    let vars = read_tree(|t| t.node(node).and_then(|n| n.computed.as_ref().map(|c| c.vars.clone())))
        .unwrap_or_default();
    let (keyframes, notes) = vn_css::script_keyframes(frames, &vars, &env());
    let previous = ANIMATOR.with(|a| {
        let mut a = a.borrow_mut();
        ensure_timeline_origin(&mut a);
        let previous = a.script_node(id);
        a.script_set(id, node, keyframes, timing);
        previous
    });
    // A new target (`effect.target = other`): the old one stops painting it.
    if previous.is_some_and(|p| p != node) {
        script_touched(previous);
    }
    script_touched(Some(node));
    notes
}

/// Moves animation `id`'s clock (play, pause, seek, rate, finish, cancel).
pub(crate) fn script_anim_clock(id: u32, clock: vn_css::ScriptClock) {
    let node = ANIMATOR.with(|a| a.borrow_mut().script_clock(id, clock));
    script_touched(node);
}

/// Forgets animation `id`: cancelled, replaced, or collected.
pub(crate) fn script_anim_drop(id: u32) {
    let node = ANIMATOR.with(|a| a.borrow_mut().script_drop(id));
    script_touched(node);
}

/// Animation `id`'s own values right now, for `commitStyles()`; `None` when
/// it has no target here.
pub(crate) fn script_anim_values(id: u32) -> Option<Vec<(String, String)>> {
    let now = std::time::Instant::now();
    ANIMATOR.with(|a| {
        let a = a.borrow();
        let node = a.script_node(id)?;
        let base = read_tree(|t| t.node(node).and_then(|n| n.computed.as_ref().map(|c| c.base.clone())))
            .unwrap_or_default();
        a.script_values(id, &base, now)
    })
}

/// Did a script animation change what some node paints since the last call?
/// Read once per frame by [`crate::frame_tick`].
pub(crate) fn take_script_dirty() -> bool {
    SCRIPT_DIRTY.with(|d| d.replace(false))
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
///
/// One text per sheet, not one concatenation: vn-css parses each on its own and
/// once per distinct text, so a `<style>` that changes does not make the
/// build's sheet parse — and leak — again (`vn_css::Engine::compile_sheets`).
fn sheet_sources() -> Vec<String> {
    let mut out = vec![SHEET_SOURCE.with(|s| s.borrow().clone())];
    HEAD_SHEETS.with(|h| out.extend(h.borrow().iter().cloned()));
    RUNTIME_CSS.with(|r| out.extend(r.borrow().iter().cloned()));
    out
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
        let mut sheets = Vec::new();
        for id in ids {
            // Reused controls keep unopened content in detached fragments
            // (Reka SelectViewport includes a <style> there). Such a sheet is
            // inert until attached to a document or painted VnWindow root.
            let mut ancestor = id;
            let mut connected = false;
            while ancestor != 0 {
                if ancestor == t.root || crate::windows::window_of_root(ancestor).is_some() {
                    connected = true;
                    break;
                }
                let Some(node) = t.node(ancestor) else { break };
                ancestor = node.parent;
            }
            if !connected {
                continue;
            }
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
    // The dedupe set is deliberately NOT cleared. A `<style>` arrives in pieces
    // — created, then filled, then sometimes refilled — and each step is a
    // recompile; clearing would make every advisory the BUILD's sheet already
    // gave re-print once per step, which is the exact noise `set_stylesheet`'s
    // freshness check exists to prevent. Whatever the new text says that has
    // not been said is a new string and speaks on its own.
    recompile_sheet(
        crate::perf::FullReason::StyleElement,
        "a <style> element in the tree changed — recompiling the stylesheet",
    );
}

/// The tests' shorthand: `css` with no head `<style>` texts.
#[cfg(test)]
pub(crate) fn set_stylesheet(css: String) {
    set_stylesheet_with_head(css, Vec::new());
}

/// Installs the app's stylesheet, with the `<style>` texts `document.head`
/// holds, each its own sheet after `css` (see [`sheet_sources`]).
///
/// Returns `false`, and compiles nothing, when the sheet and the head texts are
/// exactly what is installed already. The runtime re-sends the sheet whenever a
/// head `<style>` is touched, empty ones included: the Argon client re-sent the
/// identical text during boot, one of its six boot compiles and a full restyle
/// that changed nothing (research/reports/m26/perf-mem-2.md §1).
pub fn set_stylesheet_with_head(css: String, head: Vec<String>) -> bool {
    let same = ENGINE.with(|e| e.borrow().is_some())
        && SHEET_SOURCE.with(|s| *s.borrow() == css)
        && HEAD_SHEETS.with(|h| *h.borrow() == head);
    if same {
        return false;
    }
    // A genuinely NEW sheet is a new set of gaps: an advisory suppressed
    // against the old one must be free to speak again (this is the HMR path).
    // Recompiling the SAME text under a new rem/viewport basis is not that —
    // see [`recompile_sheet`], which is why the clear lives here and not in
    // [`set_engine`]. Nor is a head `<style>` changing: like one in the tree
    // ([`sync_runtime_sheets`]), what it says new speaks on its own.
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
    crate::perf::note_sheet_compile(took.elapsed().as_secs_f32() * 1000.0, len, before, crate::perf::live_bytes());
    true
}

/// Compiles the sheets, after handing their `@font-face` rules to
/// [`crate::fonts`] (which vn-css itself does not keep).
fn compile(sources: &[String]) -> vn_css::Engine {
    crate::fonts::note_sheet(&sources.join("\n"));
    let sheets: Vec<&str> = sources.iter().map(String::as_str).collect();
    vn_css::Engine::compile_sheets(&sheets)
}

/// Web fonts were registered: vn-css chose every `font-family` when it
/// compiled the sheet and parsed the inline styles, against the fonts
/// installed THEN, so both are done again and the document is restyled.
/// Called from [`crate::fonts`]'s frame tick, after its new installed-font
/// list is in place.
pub(crate) fn recompile_for_fonts() {
    recompile_sheet(crate::perf::FullReason::Fonts, "web fonts were registered — recompiling the stylesheet");
    with_tree(|t| {
        t.invalidate_font_inline();
        t.mark_dirty();
    });
}

/// Installs a compiled engine WITHOUT recording a source to recompile from.
/// [`set_stylesheet`] is the path the host uses; this one is for tests and for
/// the recompile inside [`restyle_all`], which already has the source.
pub(crate) fn set_engine(engine: vn_css::Engine) {
    // A font load, late component style or rem recompile must not erase a
    // running animation/transition. Its computed declarations can stay equal,
    // so explicitly rebind keyframes even when the ordinary change gate does
    // not fire. Same-name runs keep their clocks; missing definitions cancel.
    REBIND_ANIMATIONS.with(|pending| pending.set(true));
    // Through `warn_css`, so a recompile under a new rem/viewport basis does
    // not re-print what the same source already said. Compiling the sheet three
    // times at boot — once cold, once for the rem basis, once for the client
    // size — is normal and used to TRIPLE the advisory stream the developer
    // sees. `set_stylesheet` clears the dedupe set when the text itself
    // changes, which is the case that has to speak again.
    for d in &engine.diagnostics {
        warn_css(&format!("[css] {d}"));
    }
    note_parent_keys(&engine);
    ENGINE.with(|c| *c.borrow_mut() = Some(engine));
    // The cache is keyed by the old sheet's string addresses, which the new
    // sheet may reuse for different names.
    forget_read_cache();
}

fn forget_read_cache() {
    READ_CACHE.with(|c| c.iter().for_each(|s| s.set((0, 0, 0))));
}

/// The names [`ReadOnOthers`] cannot learn by listening: vn-css's index offers
/// `.stack > *` only to the children of a `.stack`, so no other child ever
/// asks its parent about `stack` (`Engine::classes_keyed_on_the_parent`). Each
/// is entered as read by every tag, which is what the matcher's questions
/// recorded when every element was asked them.
fn note_parent_keys(engine: &vn_css::Engine) {
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
    // `rem` — so a resize has to re-resolve them ([`rebase_viewport`]), and
    // only when the sheet actually contains a `vh`/`vw`. Almost none do; the
    // ones that do cannot do without it (`max-h-[85vh]` is the only thing
    // keeping reka's dialog inside the window, and dropping it turned the
    // dialog into a full-window black slab).
    // **A-M05, the phase-1 guard (docs/MULTIWINDOW.md §6.2 option C).** `vw` /
    // `vh` / `rem` are flattened to px at SHEET-COMPILE time, into one
    // process-wide basis, so two differently-sized windows sharing a
    // `vh`-using sheet would recompile the single `Engine` on every frame —
    // and repeatedly compiling it would restyle both windows on every frame.
    // The basis therefore stays the PRIMARY
    // window's, and we say so once.
    //
    // The gate is free for the apps that use no viewport unit, which is most of
    // them: `uses_viewport_units()` is false and per-window `@media` is
    // completely correct today. When it is true and a pop-out is open, the
    // conditions are still per-window (above); only the LENGTHS come from the
    // primary window's viewport.
    let basis_moved = vn_css::set_viewport_basis(env.viewport_width, env.viewport_height);
    if basis_moved && vn_css::uses_viewport_units() {
        rebase_viewport(env.viewport_width, env.viewport_height);
    }
    // …and the INLINE styles, which no recompile reaches: each body that names
    // a viewport unit is re-parsed against the new basis (m26; see
    // `Tree::invalidate_viewport_inline`).
    if basis_moved {
        with_tree(|t| t.invalidate_viewport_inline());
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
        if !recompile_sheet(crate::perf::FullReason::RemBasis, &format!(
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

/// Does this CSS value text use a viewport-relative unit (`100vw`, `85svh`,
/// `calc(100vh - 2rem)`, `-.5vmin`)? A dimension token is a number directly
/// followed by its unit; the unit set is vn-css's (`viewport_unit_px`).
fn has_viewport_unit(value: &str) -> bool {
    const UNITS: [&str; 24] = [
        "vw", "vh", "vi", "vb", "vmin", "vmax", "svw", "svh", "svi", "svb", "svmin", "svmax", "lvw", "lvh",
        "lvi", "lvb", "lvmin", "lvmax", "dvw", "dvh", "dvi", "dvb", "dvmin", "dvmax",
    ];
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
            if UNITS.contains(&unit.as_str()) {
                return true;
            }
        } else if b[i].is_ascii_alphabetic()
            || b[i] == b'_'
            || (b[i] == b'-' && b.get(i + 1).is_some_and(|c| c.is_ascii_alphabetic() || *c == b'-' || *c == b'_'))
        {
            // An identifier (`var(--x9vw)`, `abc12vh`): its digits are not a
            // number. A `-` before a digit is a sign, not an identifier.
            while i < b.len() && (b[i].is_ascii_alphanumeric() || b[i] == b'-' || b[i] == b'_') {
                i += 1;
            }
        } else {
            i += 1;
        }
    }
    false
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
        crate::perf::note_style_flush();
        restyle_all();
        crate::render::gc_node_state();
        animator_gc();
    }
}

/// Recompiles the retained stylesheet source under whatever basis just changed,
/// and marks the tree for a full restyle. `false` = there was no source to
/// recompile (before the first `__vn_set_stylesheet`), so the caller should
/// stop.
fn recompile_sheet(reason: crate::perf::FullReason, why: &str) -> bool {
    // `dev`, not `css`. Nothing here describes a divergence from a browser:
    // it says the engine flattened `rem`/`vh` at compile time and has just
    // re-flattened them against a number that moved. A developer counting CSS
    // warnings in the Console panel was being handed three of these per boot
    // (cold, the root font-size, the first real client size) as if they were
    // gaps, which is exactly the noise m19's rule — an advisory has to name a
    // real divergence — was written to keep out. The line still prints and is
    // still in the ring; it is on the channel that says "state trace".
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
    crate::perf::note_sheet_compile(took.elapsed().as_secs_f32() * 1000.0, len, before, crate::perf::live_bytes());
    with_tree(|t| t.mark_dirty());
    true
}

/// The viewport basis moved under a sheet that uses `vh`/`vw`: re-resolves just
/// the rules that use one, in the installed engine
/// ([`vn_css::Engine::rebase_viewport`]), or recompiles the sheet when the
/// engine cannot. A window drag does this on every step, and a recompile of the
/// Argon client's sheet is ~60 ms (research/reports/m26/perf-mem-3.md §2).
///
/// It keeps the engine and avoids a full sheet compilation; running motion
/// keeps its clock on either path. The resize already marked the whole
/// document for a restyle (`crate::set_env`); this marks it again for the
/// callers that move the basis some other way.
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

/// One recompile to adopt the root's font-size, and one more in case that
/// font-size was itself relative. A third would be a cycle.
const REM_BASIS_PASSES: usize = 3;

/// The font size the document root inherits: CSS's initial `medium`, which
/// is 16px in every browser's default settings. The root's own `font-size:
/// 62.5%` or `1.5em` is of this.
const ROOT_PARENT_FONT_SIZE: f32 = 16.0;

/// The inherited text values the restyle walk threads into each element:
/// what `currentColor` and `em` resolve against when the element declares
/// no `color` or `font-size` of its own.
#[derive(Clone, Debug, PartialEq)]
struct Inherited {
    color: Option<gpui::Hsla>,
    /// The parent's computed font size, px.
    font_size: f32,
    font: gpui::Font,
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

    /// Records, for the restyle pass's own questions only, that the matcher
    /// asked this node about a sibling ([`sel`]): EDGE when it is the element
    /// being cascaded, WALK for any other. On the parent, whose child list it
    /// is about.
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

    /// Records, for the restyle pass's own questions only, that the matcher
    /// asked `name` of this node on behalf of another element
    /// ([`ReadOnOthers`]).
    fn note_read(&self, name: &str, kind: NameKind) {
        let subject = CASCADE_SUBJECT.with(Cell::get);
        if subject == 0 || subject == self.id {
            return;
        }
        let tag = self.tree.node(subject).map_or(0, |n| n.tag);
        note_name_read(self.id, subject, tag, name, kind);
    }

    /// The same for a question about this node's own content: `:empty`, or
    /// `:has()`, which is [`sel::HAS_SELF`] when the element being cascaded
    /// asked it of itself and [`sel::HAS_DESC`] when a descendant's selector
    /// asked it of an ancestor.
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

impl<'t> vn_css::MatchDom for NodeRef<'t> {
    fn key(&self) -> u64 {
        self.id as u64
    }
    fn parent(&self) -> Option<Self> {
        let p = self.node().parent;
        (p != 0).then(|| NodeRef { tree: self.tree, id: p })
    }
    fn prev_sibling(&self) -> Option<Self> {
        self.note_sibling_question(sel::PREV_EDGE, sel::PREV_WALK);
        self.element_sibling(false)
    }
    fn next_sibling(&self) -> Option<Self> {
        self.note_sibling_question(sel::NEXT_EDGE, sel::NEXT_WALK);
        self.element_sibling(true)
    }
    /// Every ELEMENT child, for `:has()`.
    ///
    /// Anchor nodes are Vue fragments: not elements, but they DO carry element
    /// children, so the walk flattens through them (and through text nodes,
    /// which have none) rather than stopping. Without that a `:has()` would
    /// miss everything under a `v-if` or a `<template>`.
    ///
    /// The outermost call is the anchor asking about its own subtree, so that
    /// node is flagged [`sel::HAS`]; the nested calls are the same question
    /// walking down, and flag nothing.
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
        // The `html` / `body` aliasing for the document root lives in vn-css
        // (`element.rs`), so every `MatchDom` gets it — the query matcher and
        // the engine's own tests included.
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
    /// The DOM attribute channel, straight off `Node::props`. Names are
    /// interned once per sheet compile's worth of lookups; an attribute a
    /// selector asks for that JS has never written has no id at all, which is
    /// the cheap "absent" answer.
    fn attr(&self, name: &str) -> Option<&str> {
        self.note_read(name, NameKind::Attr);
        // `class` rides its own channel (`set_class`), not the property map.
        // An empty one reads as absent: the channel cannot tell `class=""`
        // from no attribute (docs/RUNTIME.md §5.13).
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
    /// `:empty` as Selectors 3 and the browsers read it: only an element, or a
    /// text node with data, makes a parent non-empty. A comment does not, and
    /// neither does a zero-length text node. Those are exactly what Vue leaves
    /// behind: a comment for a `v-if` that is off, and empty text for a
    /// fragment's or a Teleport's two ends in a production build. Counting
    /// them meant `empty:hidden` never matched (m26 client sweep #23).
    /// Whitespace-only text still counts, as it does in Chrome and Firefox.
    /// An anchor that carries children is looked through, as `each_child`
    /// does. The answer is [`Tree::is_bare`]'s, which the mutators also use
    /// to tell when it flips.
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

    /// The retained-render cache replays a subtree for exactly as long as
    /// [`Node::sub_rev`] says nothing in it moved, so the number has to move for
    /// every mutation and for no mutation next door. Upwards, and only upwards:
    /// a change at a node restyles its own subtree and can move any ancestor's
    /// box, but it cannot move a cousin's.
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
        let before = read_tree(|t| (t.sub_rev(root), t.sub_rev(left), t.sub_rev(leaf), t.sub_rev(right)));

        with_tree(|t| t.set_class(leaf, "hot"));

        let after = read_tree(|t| (t.sub_rev(root), t.sub_rev(left), t.sub_rev(leaf), t.sub_rev(right)));
        assert!(after.0 > before.0, "the root is an ancestor of the change");
        assert!(after.1 > before.1, "so is the parent");
        assert!(after.2 > before.2, "and the node itself");
        assert_eq!(after.3, before.3, "the sibling subtree did not move");
    }

    /// Removing a child is a change to the PARENT's subtree — `:last-child`
    /// and the parent's own box both depend on the child list — and the node
    /// that went away cannot report anything itself.
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

    /// The one way a change at one node reaches a node that is not its
    /// descendant is the CASCADE — a descendant combinator, an inherited
    /// `var()`, a structural pseudo. The mutator cannot see that far, so the
    /// restyle pass reports it: every node whose fold actually moved is
    /// touched, and a cache boundary sitting on one of them invalidates.
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

        // One class, on the ANCESTOR. Nothing about `leaf` itself was written.
        with_tree(|t| t.set_class(wrap, "on"));
        restyle_all();

        let after = read_tree(|t| (t.sub_rev(leaf), t.sub_rev(bystander)));
        assert_eq!(
            read_tree(|t| t.node(leaf).and_then(|n| n.computed.as_ref()).and_then(|c| c.base.opacity)),
            Some(0.5),
            "the descendant combinator reached the leaf",
        );
        assert!(after.0 > before.0, "…so the leaf's subtree revision moved with it");
        assert_eq!(after.1, before.1, "the bystander's fold did not move");
    }

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
        read_tree(|t| t.node(node).unwrap().computed.as_ref().unwrap().base.opacity)
    }

    #[test]
    fn detached_fragment_styles_are_inert_until_connected_and_stop_on_detach() {
        let (root, probe, fragment, sheet) = runtime_sheet_probe();
        restyle_all();
        assert_eq!(runtime_sheet_opacity(probe), Some(0.25), "closed Select fragments install no rules");

        with_tree(|t| t.insert(fragment, root, 0));
        restyle_all();
        assert_eq!(runtime_sheet_opacity(probe), Some(0.75), "connecting the fragment activates its style");

        with_tree(|t| {
            let other_fragment = t.create_anchor();
            t.insert(fragment, other_fragment, 0);
        });
        restyle_all();
        assert_eq!(runtime_sheet_opacity(probe), Some(0.25), "detaching removes its rules without freeing the style");

        with_tree(|t| t.set_element_text(sheet, ".probe { opacity: 0.9 }".into()));
        restyle_all();
        assert_eq!(runtime_sheet_opacity(probe), Some(0.25), "editing detached CSS remains inert");
    }

    #[test]
    fn runtime_styles_in_a_painted_secondary_root_remain_connected() {
        let (_, probe, secondary_root, _) = runtime_sheet_probe();
        restyle_all();
        assert_eq!(runtime_sheet_opacity(probe), Some(0.25));

        crate::windows::set_painted_root(9001, secondary_root);
        restyle_all();
        assert_eq!(runtime_sheet_opacity(probe), Some(0.75), "VnWindow styles share the live document cascade");

        crate::windows::set_painted_root(9001, 0);
        restyle_all();
        assert_eq!(runtime_sheet_opacity(probe), Some(0.25), "unpainting a detached root deactivates its style");
        crate::windows::forget(9001);
    }

    /// m26 client sweep #23: `fixed inset-0 … empty:hidden` overlays stayed
    /// laid out and painted, because the anchors Vue leaves in an empty
    /// container counted as children. A comment and a zero-length text node
    /// do not make an element non-empty; an element, or text with data, does;
    /// and a text node gaining or losing its data restyles its parent.
    #[test]
    fn empty_ignores_vue_anchors_and_empty_text() {
        let opacity = |id: u32| {
            read_tree(|t| t.node(id).and_then(|n| n.computed.as_ref()).and_then(|c| c.base.opacity))
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
            // A `v-if` that is off, and a fragment's two ends.
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
        assert_eq!(opacity(anchored), Some(0.5), "anchors and an empty text node only");
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
        for yes in ["100vw", "100svh", "calc(100vh - 2rem)", "-5vh", "-.5vmin", "85DVH", "min(50vw, 40rem)", "1.5lvmax"] {
            assert!(has_viewport_unit(yes), "{yes:?}");
        }
        for no in ["10px", "var(--x9vw)", "abc12vh", "2rem", "", "50%", "url(a1vw.png)", "100v", "3vws"] {
            assert!(!has_viewport_unit(no), "{no:?}");
        }
    }

    /// m26 6c: an INLINE `100svw` is flattened to px when it is parsed, and it
    /// is parsed only when its text changes, so a resize left it at the old
    /// window size (the Argon client's AppShell: 1652x633 in a 1700x800
    /// window). A basis change now re-parses every inline body that names a
    /// viewport unit and re-cascades its subtree, which also re-resolves a
    /// custom property declared inline and composed by a descendant's `var()`.
    #[test]
    fn inline_viewport_units_and_var_composed_ones_follow_the_viewport() {
        fn px(l: Option<gpui::Length>) -> Option<f32> {
            match l? {
                gpui::Length::Definite(gpui::DefiniteLength::Absolute(gpui::AbsoluteLength::Pixels(p))) => Some(f32::from(p)),
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
            if set_env(vn_css::Env { viewport_width: w, viewport_height: h, ..vn_css::Env::default() }) {
                // What the host does with a `true` (see `set_env`).
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
        assert_eq!(size(shell), Some((Some(1700.0), Some(800.0))), "the inline svw/svh re-resolved");
        assert_eq!(size(child).and_then(|s| s.0), Some(850.0), "and the var()-composed vw with it");
        assert_eq!(size(plain).and_then(|s| s.0), Some(40.0));

        // Without the host's full invalidation (a scoped pass), the marks
        // alone still reach both.
        assert!(vn_css::set_viewport_basis(1000.0, 500.0));
        let marked = with_tree(|t| t.invalidate_viewport_inline());
        assert_eq!(marked, 2, "the shell and the --w parent; never the px-only node");
        ENGINE.with(|eng| with_tree(|t| t.restyle(eng.borrow().as_ref(), &env())));
        assert_eq!(size(shell), Some((Some(1000.0), Some(500.0))));
        assert_eq!(size(child).and_then(|s| s.0), Some(500.0));
    }

    /// A SHEET's `vw`/`vh` follow a resize: re-resolved in the installed
    /// engine (`rebase_viewport`), or, with a viewport unit inside
    /// `@keyframes`, by compiling the sheet again. Same answer either way.
    #[test]
    fn sheet_viewport_units_follow_the_viewport() {
        let width = |id: u32| {
            read_tree(|t| match t.node(id)?.computed.as_ref()?.base.size.width? {
                gpui::Length::Definite(gpui::DefiniteLength::Absolute(gpui::AbsoluteLength::Pixels(p))) => {
                    Some(f32::from(p))
                }
                _ => None,
            })
        };
        let resize = |w: f32, h: f32| {
            if set_env(vn_css::Env { viewport_width: w, viewport_height: h, ..vn_css::Env::default() }) {
                with_tree(|t| t.mark_dirty());
            }
            restyle_all();
        };
        resize(1000.0, 800.0);
        assert!(set_stylesheet_with_head(".box, .wide { width: 50vw } .wide { min-height: 10vh }".into(), vec![]));
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
        assert_eq!((width(bx), width(wide)), (Some(600.0), Some(600.0)), "rebased");

        assert!(set_stylesheet_with_head(
            ".box, .wide { width: 50vw } @keyframes k { from { height: 10vh } }".into(),
            vec![]
        ));
        with_tree(|t| t.mark_dirty());
        restyle_all();
        resize(800.0, 800.0);
        assert_eq!((width(bx), width(wide)), (Some(400.0), Some(400.0)), "recompiled");
        vn_css::set_viewport_basis(vn_css::INITIAL_VIEWPORT_BASIS.0, vn_css::INITIAL_VIEWPORT_BASIS.1);
    }

    /// The runtime re-sends the sheet whenever a head `<style>` is touched; the
    /// same sheet and head texts again compile nothing, and any change does.
    #[test]
    fn an_unchanged_sheet_resent_compiles_nothing() {
        assert!(set_stylesheet_with_head(".a { opacity: 0.5 }".into(), vec![".b {}".into()]));
        assert!(!set_stylesheet_with_head(".a { opacity: 0.5 }".into(), vec![".b {}".into()]));
        assert!(set_stylesheet_with_head(".a { opacity: 0.5 }".into(), vec![".b {}".into(), ".c {}".into()]));
        assert!(set_stylesheet_with_head(".a { opacity: 0.6 }".into(), vec![".b {}".into(), ".c {}".into()]));
        assert_eq!(
            sheet_sources(),
            vec![".a { opacity: 0.6 }".to_string(), ".b {}".into(), ".c {}".into()],
            "the build's sheet, then each head text on its own"
        );
    }

    fn flush_motion_styles() {
        // The JS host invalidates the tree after installing a changed sheet.
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
        let base = read_tree(|t| t.node(node).unwrap().computed.as_ref().unwrap().base.clone());
        animation_overlay(node, &base, at).and_then(|style| style.opacity)
    }

    #[test]
    fn late_stylesheet_and_font_recompile_preserve_a_running_animation_clock() {
        let sheet = "@keyframes enter { from { opacity: 0 } to { opacity: 1 } } \
                     .motion { animation: enter 500ms linear }";
        let (node, start) = motion_probe(sheet);
        let at = start + std::time::Duration::from_millis(200);
        assert!((motion_opacity_at(node, at).unwrap() - 0.4).abs() < 1e-5);

        // A synchronously mounted panel can append CSS while its surrounding
        // Drawer is moving. The Drawer declarations do not change at all.
        assert!(set_stylesheet_with_head(sheet.into(), vec![".late-panel { color: red }".into()]));
        flush_motion_styles();
        assert!((motion_opacity_at(node, at).unwrap() - 0.4).abs() < 1e-5,
            "unrelated CSS must retain the elapsed clock, not drop/restart the run");

        recompile_for_fonts();
        flush_motion_styles();
        assert!((motion_opacity_at(node, at).unwrap() - 0.4).abs() < 1e-5,
            "registering a font must not erase the running animation");
    }

    #[test]
    fn changed_keyframes_rebind_at_the_existing_clock_and_deleted_keyframes_cancel() {
        let sheet = "@keyframes enter { from { opacity: 0 } to { opacity: 1 } } \
                     .motion { animation: enter 500ms linear }";
        let (node, start) = motion_probe(sheet);
        let at = start + std::time::Duration::from_millis(200);
        set_stylesheet(sheet.replace("opacity: 1", "opacity: 0.5"));
        flush_motion_styles();
        assert!((motion_opacity_at(node, at).unwrap() - 0.2).abs() < 1e-5,
            "same-name definition changes sample new frames on the existing timeline");

        set_stylesheet(".motion { animation: enter 500ms linear }".into());
        flush_motion_styles();
        assert!(motion_opacity_at(node, at).is_none(), "deleted frames must not keep painting");
        let events = take_anim_events();
        assert_eq!(events.iter().filter(|event| event.node == node
            && event.kind == vn_css::AnimEventKind::AnimationCancel).count(), 1);
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
        assert!(take_anim_events().iter().any(|event| event.node == node
            && event.kind == vn_css::AnimEventKind::AnimationCancel));
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
        assert!(set_stylesheet_with_head(sheet.into(), vec![".late-panel { color: red }".into()]));
        flush_motion_styles();
        assert!((motion_opacity_at(node, at).unwrap() - before).abs() < 1e-5,
            "a late sheet must not snap or restart an unrelated transition");
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

    /// A root with `n` children, each with one child of its own, all cascaded.
    /// Returns `(root, children, grandchildren)`.
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

    /// The pass's walk roots, read without running the cascade.
    fn roots_of() -> Vec<u32> {
        let mut roots = with_tree(|t| t.restyle_roots());
        roots.sort_unstable();
        roots
    }

    /// The mass mount that used to fall off the 64-mark cliff into a
    /// whole-document pass: 300 separate marks are 300 walk roots, and nothing
    /// else is walked.
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
        // The marks are spent: the next pass has nothing to do.
        assert!(roots_of().is_empty());
    }

    /// A mark under a marked ancestor is covered by the ancestor's walk.
    #[test]
    fn a_mark_under_a_marked_ancestor_is_not_walked_twice() {
        let (_, kids, grandkids) = cascaded_comb(3);
        with_tree(|t| {
            t.mark_restyle_subtree(grandkids[1]);
            t.mark_restyle_subtree(kids[1]);
        });
        assert_eq!(roots_of(), vec![kids[1]]);
    }

    /// A `<KeepAlive>` cache container is never inserted, and a mark inside it
    /// is still found. Moving a marked node from there into the document finds
    /// it under its new parent.
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

        // Marked while detached, then moved: found under its new parent, and
        // neither parent is walked, since no cascade read either child list.
        with_tree(|t| {
            t.mark_restyle_subtree(kept);
            t.insert(kept, kids[0], 0);
        });
        assert_eq!(roots_of(), vec![kept]);
        let _ = cache;
    }

    /// The common case of a mount: appending to a list whose children no rule
    /// reads the positions of walks the new child and nothing else, however
    /// long the list is. It used to walk the whole list.
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

    /// `body:has(…)` makes the document root an anchor whose subtree is every
    /// node, so marking it on every mutation would make every wave a
    /// whole-document walk. It is rechecked alone instead, and walked only
    /// when its own answer moves.
    #[test]
    fn a_root_has_anchor_is_rechecked_not_walked() {
        let sheet = "body:has([data-x]) { opacity: 0.5 }";
        set_engine(vn_css::Engine::compile(sheet));
        let (root, _, grandkids) = cascaded_comb(3);
        assert!(
            read_tree(|t| t.node(root).unwrap().sel_flags.get() & sel::HAS_SELF != 0),
            "the root asked `:has()` of itself"
        );
        let engine = vn_css::Engine::compile(sheet);
        let env = env();

        let settled_ids = |engine: &vn_css::Engine| {
            with_tree(|t| t.settle_self_marks(engine, &env)).into_iter().map(|(id, _)| id).collect::<Vec<_>>()
        };

        // A class nothing reads anywhere: the node and the anchor are asked
        // again, alone, and neither answer moved.
        with_tree(|t| t.set_class(grandkids[1], "x"));
        assert!(settled_ids(&engine).is_empty());
        assert!(roots_of().is_empty(), "no walk at all");

        // `data-x` is what the `:has()` walk reads on the root's descendants,
        // and only the root reads it, so the writer has no reader under it and
        // is restyled alone. The root's own answer moved, but nothing it hands
        // down did, so it is restyled alone as well.
        let x = with_tree(|t| t.intern("data-x"));
        with_tree(|t| t.set_prop(grandkids[0], x, Some(String::new())));
        assert_eq!(settled_ids(&engine), vec![root]);
        assert!(roots_of().is_empty(), "no walk at all");
    }

    /// A class no selector reads on another element moves only the element's
    /// own style: restyled alone, no walk. One that IS read on others (`.on
    /// .leaf`, read by a `div`) walks the `div`s under it.
    #[test]
    fn a_class_read_only_by_its_own_element_restyles_that_element_alone() {
        let sheet = ".hot { opacity: 0.5 } .on .leaf { opacity: 0.25 }";
        set_engine(vn_css::Engine::compile(sheet));
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
        let engine = vn_css::Engine::compile(sheet);
        let env = env();

        with_tree(|t| t.set_class(kids[0], "hot"));
        let settled = with_tree(|t| t.settle_self_marks(&engine, &env));
        assert_eq!(settled.len(), 1);
        assert_eq!(settled[0].0, kids[0]);
        assert_eq!(settled[0].1.base.opacity, Some(0.5));
        assert!(roots_of().is_empty(), "the subtree is not walked");

        // `on` is read on the leaf's ancestors by `.on .leaf`.
        // (The node is settled again: the test never stored the first answer.)
        with_tree(|t| t.set_class(kids[0], "hot on"));
        let settled = with_tree(|t| t.settle_self_marks(&engine, &env));
        assert_eq!(settled.iter().map(|s| s.0).collect::<Vec<_>>(), vec![kids[0]]);
        assert_eq!(roots_of(), vec![grandkids[0]]);
        let _ = root;
    }

    /// A sibling whose place in the list moved is restyled alone: its own
    /// `:nth-child()` answer is what changed. Under it, only the elements whose
    /// selectors read an ANCESTOR's place (`.row:first-child .cell`) are
    /// walked.
    #[test]
    fn a_moved_sibling_is_restyled_alone_with_the_elements_that_read_its_place() {
        let sheet = ".row:nth-child(2) { opacity: 0.5 } .row:first-child .cell { opacity: 0.25 }";
        set_engine(vn_css::Engine::compile(sheet));
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
            read_tree(|t| t.node(id).and_then(|n| n.computed.as_ref()).and_then(|c| c.base.opacity))
        };
        assert_eq!((opacity(rows[1]), opacity(cells[0])), (Some(0.5), Some(0.25)));

        let engine = vn_css::Engine::compile(sheet);
        let env = env();
        let added = with_tree(|t| {
            let div = t.intern("div");
            let added = t.create_element(div);
            t.set_class(added, "row");
            t.insert(added, list, rows[0]);
            added
        });
        // The old first row became the second, the second the third.
        let settled = with_tree(|t| t.settle_self_marks(&engine, &env));
        assert_eq!(settled.iter().map(|s| s.0).collect::<Vec<_>>(), vec![rows[0], rows[1]]);
        let mut want = vec![added, cells[0], cells[1], cells[2]];
        want.sort_unstable();
        assert_eq!(roots_of(), want, "the new row and the cells, none of the rows' other children");

        // And the narrow pass gets the answer right.
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

    /// A name a few tags read on others walks the elements of those tags in
    /// the writer's subtree, not the subtree. The case is the Argon client's
    /// `.xsolla-checkout-container [class*="submit-button"] button`, which
    /// reads the class ATTRIBUTE of every ancestor of every `<button>`: without
    /// the tags, any class change anywhere would walk everything under it.
    #[test]
    fn a_name_read_by_a_few_tags_walks_only_those_elements() {
        let sheet = "[class*=\"submit\"] button { opacity: 0.5 }";
        set_engine(vn_css::Engine::compile(sheet));
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
            read_tree(|t| t.node(id).and_then(|n| n.computed.as_ref()).and_then(|c| c.base.opacity))
        };
        assert_eq!(opacity(buttons[0]), None);

        let engine = vn_css::Engine::compile(sheet);
        let env = env();
        with_tree(|t| t.set_class(panel, "submit-row"));
        assert!(with_tree(|t| t.settle_self_marks(&engine, &env)).is_empty(), "the panel's own style did not move");
        let mut want = buttons.clone();
        want.sort_unstable();
        assert_eq!(roots_of(), want, "the buttons, and none of the divs around them");

        // And the narrow pass gets the answer right, both ways.
        with_tree(|t| t.set_class(panel, "submit-row wide"));
        restyle_all();
        assert!(buttons.iter().all(|&b| opacity(b) == Some(0.5)));
        with_tree(|t| t.set_class(panel, "wide"));
        restyle_all();
        assert!(buttons.iter().all(|&b| opacity(b).is_none()));
    }

    /// …and when the cascade did read positions, the insert marks exactly the
    /// siblings it said, to be restyled alone: an edge question marks the
    /// neighbour, a walk marks the run.
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
        // Pretend the cascade asked `:last-child` of the children.
        with_tree(|t| t.node(root).unwrap().sel_flags.set(sel::NEXT_EDGE));
        let added = with_tree(|t| {
            let div = t.intern("div");
            let added = t.create_element(div);
            t.insert(added, root, 0);
            added
        });
        assert_eq!(take_self_marks(), vec![kids[4]], "the old last child");
        assert_eq!(roots_of(), vec![added], "and the new one");

        // `:nth-child()`: an insert in the middle moves every later index.
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

    /// A Vue fragment's anchor carries element children but no computed map,
    /// so a walk cannot start under it: the root is promoted to the anchor.
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

    /// With `+` / `~` in the sheet, a name that later siblings read reaches
    /// them: the readers among them are walked. An insert reaches only the
    /// siblings whose cascade stepped across the spot, which the parent's
    /// flags name; the rest of the run is not walked any more.
    #[test]
    fn a_sibling_combinator_reaches_later_siblings_where_they_read() {
        let sheet = ".a + .b { opacity: 0.5 }";
        set_engine(vn_css::Engine::compile(sheet));
        let (root, kids, grandkids) = cascaded_comb(5);
        let opacity = |id: u32| {
            read_tree(|t| t.node(id).and_then(|n| n.computed.as_ref()).and_then(|c| c.base.opacity))
        };
        with_tree(|t| kids.iter().for_each(|&k| t.set_class(k, "b")));
        restyle_all();
        // `kids` were inserted at the front one by one, so they run backwards.
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
        assert_eq!(marked, vec![first, third], "the class change, and the one whose `+` steps across");

        // And the answers (the checks above spent their marks unstyled).
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

    /// A node created this wave and never cascaded is a walk root wherever the
    /// search meets it, so a subtree is never entered with a parent map that
    /// does not exist yet.
    #[test]
    fn an_uncascaded_element_on_the_trail_is_a_walk_root() {
        let (_, kids, _) = cascaded_comb(2);
        let fresh = with_tree(|t| {
            let div = t.intern("div");
            let fresh = t.create_element(div);
            let inner = t.create_element(div);
            t.insert(inner, fresh, 0);
            // Attached by hand, without the parent mark `insert` would add,
            // and with its own mark taken off: only the trail through it and
            // its missing computed map say it has to be walked.
            t.node_mut(fresh).unwrap().parent = kids[0];
            t.node_mut(fresh).unwrap().mark_gen = 0;
            t.node_mut(kids[0]).unwrap().children.push(fresh);
            t.flag_path(kids[0]);
            fresh
        });
        assert_eq!(roots_of(), vec![fresh]);
    }

    fn font_size_of(id: u32) -> f32 {
        read_tree(|t| t.node(id).and_then(|n| n.computed.as_ref()).map(|c| c.font_size)).expect("cascaded")
    }

    fn padding_top_of(id: u32) -> Option<gpui::DefiniteLength> {
        read_tree(|t| t.node(id).and_then(|n| n.computed.as_ref()).and_then(|c| c.base.padding.top))
    }

    fn px_len(v: f32) -> Option<gpui::DefiniteLength> {
        Some(gpui::px(v).into())
    }

    /// CSS Values 4 §6.1.1 down a real walk: the root's `62.5%` is of the
    /// initial 16px, each `font-size` em or percentage is of the parent's
    /// computed size, every other em is of the element's own, an element
    /// that declares no size inherits the px, and `rem` stays on the root.
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
        assert_eq!(vn_css::rem_basis(), 10.0, "the root's 62.5% of 16px is the rem basis");
        assert_eq!(
            [root, a, b, c, n].map(font_size_of),
            [10.0, 20.0, 15.0, 15.0, 15.0],
            "each size of its parent's, and inherited as px"
        );
        assert_eq!(padding_top_of(b), px_len(7.5), "`.5em` of its own 15px");
        let computed = read_tree(|t| t.node(c).and_then(|n| n.computed.as_ref()).map(|c| c.base.clone()))
            .expect("cascaded");
        assert_eq!(computed.margin.top, Some(gpui::Length::Definite(gpui::px(15.0).into())));
        assert_eq!(computed.size.width, Some(gpui::Length::Definite(gpui::px(20.0).into())), "2rem");
        assert_eq!(computed.text.line_height, px_len(22.5), "an em line-height is px");
        let number = read_tree(|t| t.node(n).and_then(|n| n.computed.as_ref()).and_then(|c| c.base.text.line_height));
        assert_eq!(number, Some(gpui::relative(1.5)), "a number line-height stays a number");
        let reported = crate::dom::computed_style(c).unwrap_or_default();
        assert!(reported.lines().any(|l| l == "font-size=15px"), "getComputedStyle: {reported}");
    }

    /// Two cousins with the same rules under parents of different font sizes:
    /// the matched-properties cache must not hand the second the first's px.
    /// And a class that changes only its own element's font size still walks
    /// the subtree under it, since every em there moves.
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
        assert_eq!(leaves.iter().map(|&l| padding_top_of(l)).collect::<Vec<_>>(), vec![px_len(20.0), px_len(16.0)]);

        // `.big` is read by its own element alone, so this is a self-mark; the
        // font size it hands down moved, so the leaf under it is walked too.
        with_tree(|t| {
            t.set_class(plain, "big");
            t.set_class(big, "");
        });
        restyle_all();
        assert_eq!(leaves.iter().map(|&l| padding_top_of(l)).collect::<Vec<_>>(), vec![px_len(16.0), px_len(20.0)]);
        assert_eq!(font_size_of(leaves[1]), 20.0);
    }
    #[test]
    fn character_width_follows_inherited_face_and_restyles_descendants() {
        vn_css::set_font_source(|| vec!["VN Metric Narrow".into(), "VN Metric Wide".into()]);
        vn_css::set_font_metric_source(|font, size| Some(vn_css::FontRelativeMetrics {
            zero_advance: size * if font.family.as_ref() == "VN Metric Wide" {0.8} else {0.6},
            x_height: size * 0.4,
        }));
        struct Reset;
        impl Drop for Reset {fn drop(&mut self) {vn_css::set_font_metric_source(|_, _| None);}}
        let _reset = Reset;
        set_stylesheet(".narrow {font-family:'VN Metric Narrow';font-size:10px} .wide {font-family:'VN Metric Wide';font-size:10px} .leaf{width:1ch}".into());
        let (parent, leaves) = with_tree(|tree| {
            let root=tree.create_root(); let tag=tree.intern("div"); let mut parents=Vec::new(); let mut leaves=Vec::new();
            for class in ["narrow","wide"] {
                let parent=tree.create_element(tag);tree.set_class(parent,class);tree.insert(parent,root,0);parents.push(parent);
                let leaf=tree.create_element(tag);tree.set_class(leaf,"leaf");tree.insert(leaf,parent,0);leaves.push(leaf);
            }
            (parents[0], leaves)
        });
        let widths = || read_tree(|tree| leaves.iter().map(|&id| tree.node(id).and_then(|node|node.computed.as_ref()).and_then(|computed|computed.base.size.width)).collect::<Vec<_>>());
        restyle_all();
        assert_eq!(widths(),vec![Some(gpui::px(6.0).into()),Some(gpui::px(8.0).into())]);
        with_tree(|tree|tree.set_class(parent,"wide"));
        restyle_all();
        assert_eq!(widths(),vec![Some(gpui::px(8.0).into()),Some(gpui::px(8.0).into())]);
    }

}
