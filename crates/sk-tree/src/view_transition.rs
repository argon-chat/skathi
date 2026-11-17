//! Same-document view transitions: the old state captured as snapshots, the update, then the
//! pseudo-element tree (`::view-transition` and its groups) styled by the cascade and animated by
//! the animator over live captures of the new state.
//!
//! While one runs, the document still lays out and paints, but into captures (gpui's
//! `vn_with_capture`) rather than the window; the overlay composites them.

use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet};
use std::rc::Rc;
use std::time::{Duration, Instant};

use gpui::{
    canvas, div, AnyElement, InteractiveElement as _, IntoElement as _, ParentElement as _,
    Refineable as _, Styled as _, Window,
};
use sk_css::view_transition::{inherit_animation_timing, NAME_ATTR, TAGS};
use sk_css::{ComputedStates, MatchDom};

use crate::tree::{self, Kind, NodeRef, Tree};

const PSEUDO_BASE: u32 = 0xFF00_0000;
const UPDATE_TIMEOUT: Duration = Duration::from_secs(4);
const OLD: u64 = 1;
const NEW: u64 = 2;

pub(crate) fn is_pseudo(id: u32) -> bool {
    id >= PSEUDO_BASE
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Phase {
    /// The next build captures the old state.
    Capture,
    Captured,
    /// The update callback runs; the old state stays on screen.
    Update,
    /// The next build captures the new state.
    AwaitNew,
    NewCaptured,
    Animate,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Reason {
    Abort,
    InvalidState(String),
    Timeout,
    /// The update callback failed; its reason is the rejection's.
    Rejected,
}

impl Reason {
    pub(crate) fn error(&self) -> (&'static str, String) {
        match self {
            Reason::Abort => ("AbortError", "Transition was skipped".into()),
            Reason::InvalidState(why) => ("InvalidStateError", why.clone()),
            Reason::Timeout => (
                "TimeoutError",
                "Transition was aborted because of timeout in DOM update".into(),
            ),
            Reason::Rejected => ("AbortError", "The update callback failed".into()),
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Note {
    /// Call the update callback.
    Update(u32),
    Ready(u32),
    Skipped(u32, Reason),
    Finished(u32),
}

#[derive(Clone)]
struct Named {
    name: Rc<str>,
    classes: Rc<[Rc<str>]>,
    capture: u64,
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct Geometry {
    width: f32,
    height: f32,
    matrix: [f32; 6],
}

struct Group {
    name: Rc<str>,
    classes: Rc<[Rc<str>]>,
    old: Option<(u64, Geometry)>,
    new: Option<u64>,
    /// Group, image pair, old, new.
    styles: [Option<ComputedStates>; 4],
}

struct Transition {
    seq: u32,
    phase: Phase,
    types: Vec<String>,
    old: Vec<Named>,
    new: Vec<Named>,
    /// The capture each node paints into this frame.
    captures: HashMap<u32, u64>,
    groups: Vec<Group>,
    overlay: Option<ComputedStates>,
    since: Instant,
}

thread_local! {
    static ACTIVE: RefCell<Option<Transition>> = const { RefCell::new(None) };
    static NOTES: RefCell<Vec<Note>> = const { RefCell::new(Vec::new()) };
    static SEQ: Cell<u32> = const { Cell::new(0) };
    static FORGET: RefCell<Vec<u64>> = const { RefCell::new(Vec::new()) };
    /// The types `:active-view-transition` exposes on the root, as last written.
    static ROOT_PROP: RefCell<Option<String>> = const { RefCell::new(None) };
}

fn capture_id(seq: u32, generation: u64, index: usize) -> u64 {
    (seq as u64) << 32 | generation << 24 | index as u64
}

fn note(n: Note) {
    NOTES.with(|q| q.borrow_mut().push(n));
}

pub(crate) fn has_notes() -> bool {
    NOTES.with(|q| !q.borrow().is_empty())
}

pub(crate) fn take_notes() -> Vec<Note> {
    NOTES.with(|q| std::mem::take(&mut *q.borrow_mut()))
}

/// Starts a transition, skipping the active one. A hidden page skips it at once.
pub(crate) fn start(types: Vec<String>, hidden: bool) -> u32 {
    if let Some(seq) = active_seq() {
        skip(seq, Reason::Abort);
    }
    let seq = SEQ.with(|s| {
        let next = s.get().wrapping_add(1).max(1);
        s.set(next);
        next
    });
    if hidden {
        note(Note::Update(seq));
        note(Note::Skipped(
            seq,
            Reason::InvalidState("View transitions cannot start in a hidden document".into()),
        ));
        return seq;
    }
    ACTIVE.with(|a| {
        *a.borrow_mut() = Some(Transition {
            seq,
            phase: Phase::Capture,
            types,
            old: Vec::new(),
            new: Vec::new(),
            captures: HashMap::new(),
            groups: Vec::new(),
            overlay: None,
            since: Instant::now(),
        })
    });
    seq
}

pub(crate) fn active_seq() -> Option<u32> {
    ACTIVE.with(|a| a.borrow().as_ref().map(|t| t.seq))
}

pub(crate) fn skip(seq: u32, reason: Reason) {
    if active_seq() == Some(seq) {
        end(Some(reason));
    }
}

/// The update callback's promise settled.
pub(crate) fn update_done(seq: u32, fulfilled: bool) {
    let waiting = ACTIVE.with(|a| {
        let mut a = a.borrow_mut();
        match a.as_mut() {
            Some(t) if t.seq == seq && t.phase == Phase::Update => {
                if fulfilled {
                    t.phase = Phase::AwaitNew;
                }
                true
            }
            _ => false,
        }
    });
    if waiting && !fulfilled {
        end(Some(Reason::Rejected));
    }
}

fn end(reason: Option<Reason>) {
    let Some(t) = ACTIVE.with(|a| a.borrow_mut().take()) else {
        return;
    };
    tree::cancel_detached(PSEUDO_BASE);
    for (i, _) in t.groups.iter().enumerate() {
        for k in 0..4 {
            tree::cancel_detached(pseudo_id(i, k));
        }
    }
    FORGET.with(|f| {
        let mut f = f.borrow_mut();
        f.push(capture_id(t.seq, OLD, 0));
        f.push(capture_id(t.seq, NEW, 0));
        f.extend(t.old.iter().chain(&t.new).map(|n| n.capture));
    });
    match reason {
        Some(reason) => {
            if matches!(t.phase, Phase::Capture | Phase::Captured) {
                note(Note::Update(t.seq));
            }
            note(Note::Skipped(t.seq, reason));
        }
        None => note(Note::Finished(t.seq)),
    }
}

fn pseudo_id(group: usize, part: usize) -> u32 {
    PSEUDO_BASE + 1 + group as u32 * 4 + part as u32
}

/// A transition holds the document's paint: the retained cache and culling must stand aside.
pub(crate) fn capturing() -> bool {
    ACTIVE.with(|a| a.borrow().is_some())
}

pub(crate) fn capture_of(id: u32) -> Option<u64> {
    ACTIVE.with(|a| {
        a.borrow()
            .as_ref()
            .and_then(|t| t.captures.get(&id).copied())
    })
}

fn name_of(t: &Tree, id: u32, cs: &ComputedStates) -> Option<Rc<str>> {
    match &cs.extras.view_transition_name {
        Some(Some(name)) if matches!(&**name, "auto" | "match-element") => {
            let own = (&**name == "auto").then(|| t.prop(id, "id")).flatten();
            Some(match own {
                Some(own) if !own.is_empty() => own.into(),
                _ => format!("-sk-element-{id}").into(),
            })
        }
        Some(Some(name)) => Some(name.clone()),
        Some(None) => None,
        None if id == t.root => Some("root".into()),
        None => None,
    }
}

/// The rendered elements with a `view-transition-name`, in tree order, with their nodes.
fn collect(t: &Tree) -> Result<Vec<(Rc<str>, Rc<[Rc<str>]>, u32)>, String> {
    let mut out = Vec::new();
    let mut seen = HashSet::new();
    let mut stack = vec![t.root];
    while let Some(id) = stack.pop() {
        let Some(node) = t.node(id) else { continue };
        if node.kind == Kind::Element {
            let Some(cs) = &node.computed else { continue };
            if cs.base.display == Some(gpui::Display::None) {
                continue;
            }
            if let Some(name) = name_of(t, id, cs) {
                if !seen.insert(name.clone()) {
                    return Err(format!("Unexpected duplicate view-transition-name: {name}"));
                }
                let classes = cs
                    .extras
                    .view_transition_class
                    .clone()
                    .unwrap_or_else(|| Rc::from([]));
                out.push((name, classes, id));
            }
        }
        stack.extend(node.children.iter().rev().copied());
    }
    Ok(out)
}

/// Before the primary window's build: a phase that captures assigns this frame's captures.
pub(crate) fn begin_build(t: &Tree, window: &Window) {
    let phase = ACTIVE.with(|a| a.borrow().as_ref().map(|t| t.phase));
    let generation = match phase {
        Some(Phase::Capture) => OLD,
        Some(Phase::AwaitNew) => NEW,
        _ => return,
    };
    if !window.vn_supports_snapshots() {
        end(Some(Reason::InvalidState(
            "This renderer cannot capture view transitions".into(),
        )));
        return;
    }
    let named = match collect(t) {
        Ok(named) => named,
        Err(why) => {
            end(Some(Reason::InvalidState(why)));
            return;
        }
    };
    ACTIVE.with(|a| {
        let mut a = a.borrow_mut();
        let Some(tr) = a.as_mut() else { return };
        tr.captures.clear();
        let mut list = Vec::new();
        if generation == OLD {
            tr.captures.insert(t.root, capture_id(tr.seq, OLD, 0));
        }
        for (i, (name, classes, node)) in named.into_iter().enumerate() {
            let capture = if node == t.root {
                capture_id(tr.seq, generation, 0)
            } else {
                capture_id(tr.seq, generation, i + 1)
            };
            tr.captures.insert(node, capture);
            list.push(Named {
                name,
                classes,
                capture,
            });
        }
        if generation == OLD {
            tr.old = list;
            tr.phase = Phase::Captured;
        } else {
            tr.new = list;
            tr.phase = Phase::NewCaptured;
        }
    });
}

fn geometry(info: &gpui::VnCaptureInfo, scale: f32) -> Geometry {
    let bb = info.border_box;
    let (w, h) = (bb.size.width.0 / scale, bb.size.height.0 / scale);
    let [[a, c], [b, d]] = info.transform.rotation_scale;
    let [tx, ty] = info.transform.translation;
    let x = (a * bb.origin.x.0 + c * bb.origin.y.0 + tx) / scale;
    let y = (b * bb.origin.x.0 + d * bb.origin.y.0 + ty) / scale;
    // `transform-origin` is the box's centre.
    let (ox, oy) = (w / 2., h / 2.);
    Geometry {
        width: w,
        height: h,
        matrix: [
            a,
            b,
            c,
            d,
            a * ox + c * oy + x - ox,
            b * ox + d * oy + y - oy,
        ],
    }
}

fn matrix(m: &[f32; 6]) -> String {
    format!(
        "matrix({}, {}, {}, {}, {}, {})",
        m[0], m[1], m[2], m[3], m[4], m[5]
    )
}

/// Per frame, before the draw: advances the phases that wait on a drawn frame or the animator.
/// `true` when the window must draw.
pub(crate) fn tick(window: &mut Window) -> bool {
    let forget = FORGET.with(|f| std::mem::take(&mut *f.borrow_mut()));
    if !forget.is_empty() {
        window.vn_forget_captures(&forget);
    }
    sync_root_prop();
    let now = Instant::now();
    let Some(phase) = ACTIVE.with(|a| a.borrow().as_ref().map(|t| t.phase)) else {
        return false;
    };
    if crate::live::page_hidden() {
        end(Some(Reason::InvalidState(
            "The document became hidden".into(),
        )));
        return true;
    }
    match phase {
        Phase::Capture | Phase::AwaitNew => true,
        Phase::Captured => {
            ACTIVE.with(|a| {
                let mut a = a.borrow_mut();
                let Some(t) = a.as_mut() else { return };
                t.old.retain(|n| window.vn_capture(n.capture).is_some());
                let root = tree::read_tree(|tr| tr.root);
                t.captures.clear();
                t.captures.insert(root, capture_id(t.seq, NEW, 0));
                t.phase = Phase::Update;
                t.since = now;
                note(Note::Update(t.seq));
            });
            true
        }
        Phase::Update => {
            if ACTIVE.with(|a| {
                a.borrow()
                    .as_ref()
                    .is_some_and(|t| now - t.since > UPDATE_TIMEOUT)
            }) {
                end(Some(Reason::Timeout));
            }
            false
        }
        Phase::NewCaptured => {
            setup(window);
            note_new_geometry(window);
            restyle(now);
            ACTIVE.with(|a| {
                if let Some(t) = a.borrow_mut().as_mut() {
                    t.phase = Phase::Animate;
                    note(Note::Ready(t.seq));
                }
            });
            true
        }
        Phase::Animate => {
            note_new_geometry(window);
            restyle(now);
            let done = ACTIVE.with(|a| {
                a.borrow().as_ref().is_some_and(|t| {
                    tree::animations_finished(PSEUDO_BASE, now)
                        && (0..t.groups.len()).all(|g| {
                            (0..4).all(|k| tree::animations_finished(pseudo_id(g, k), now))
                        })
                })
            });
            if done {
                end(None);
            }
            true
        }
    }
}

fn sync_root_prop() {
    let want = ACTIVE.with(|a| a.borrow().as_ref().map(|t| t.types.join(" ")));
    if ROOT_PROP.with(|p| *p.borrow() == want) {
        return;
    }
    ROOT_PROP.with(|p| *p.borrow_mut() = want.clone());
    tree::with_tree(|t| {
        if t.root == 0 {
            return;
        }
        let key = t.intern(sk_css::view_transition::ACTIVE_ATTR);
        t.set_prop(t.root, key, want);
    });
}

/// Pairs old and new captures by name into groups, with each side's geometry.
fn setup(window: &Window) {
    let scale = window.scale_factor();
    ACTIVE.with(|a| {
        let mut a = a.borrow_mut();
        let Some(t) = a.as_mut() else { return };
        let mut groups: Vec<Group> = t
            .old
            .iter()
            .filter_map(|n| {
                let info = window.vn_capture(n.capture)?;
                Some(Group {
                    name: n.name.clone(),
                    classes: n.classes.clone(),
                    old: Some((n.capture, geometry(&info, scale))),
                    new: None,
                    styles: Default::default(),
                })
            })
            .collect();
        for n in &t.new {
            if window.vn_capture(n.capture).is_none() {
                continue;
            }
            match groups.iter_mut().find(|g| g.name == n.name) {
                Some(g) => {
                    g.new = Some(n.capture);
                    g.classes = n.classes.clone();
                }
                None => groups.push(Group {
                    name: n.name.clone(),
                    classes: n.classes.clone(),
                    old: None,
                    new: Some(n.capture),
                    styles: Default::default(),
                }),
            }
        }
        t.groups = groups;
    });
}

/// The stand-ins the pseudo-element selectors match: `sk-vt` under the document element, then
/// each group's chain.
struct Stand {
    tag: &'static str,
    name: Option<Rc<str>>,
    classes: Rc<[Rc<str>]>,
    parent: Option<usize>,
    prev: Option<usize>,
    next: Option<usize>,
    empty: bool,
}

struct Doc<'t> {
    tree: &'t Tree,
    stands: Vec<Stand>,
}

#[derive(Clone, Copy)]
enum Vt<'a, 't> {
    Real(NodeRef<'t>),
    Stand(&'a Doc<'t>, usize),
}

impl<'a, 't> Vt<'a, 't> {
    fn stand(&self) -> Option<&'a Stand> {
        match *self {
            Vt::Stand(doc, i) => Some(&doc.stands[i]),
            Vt::Real(_) => None,
        }
    }
}

impl<'a, 't> MatchDom for Vt<'a, 't> {
    fn key(&self) -> u64 {
        match self {
            Vt::Real(r) => r.key(),
            Vt::Stand(_, i) => (PSEUDO_BASE as u64) + *i as u64,
        }
    }
    fn parent(&self) -> Option<Self> {
        match *self {
            Vt::Real(r) => r.parent().map(Vt::Real),
            Vt::Stand(doc, i) => Some(match doc.stands[i].parent {
                Some(p) => Vt::Stand(doc, p),
                None => Vt::Real(NodeRef {
                    tree: doc.tree,
                    id: doc.tree.root,
                }),
            }),
        }
    }
    fn prev_sibling(&self) -> Option<Self> {
        match *self {
            Vt::Real(r) => r.prev_sibling().map(Vt::Real),
            Vt::Stand(doc, i) => doc.stands[i].prev.map(|p| Vt::Stand(doc, p)),
        }
    }
    fn next_sibling(&self) -> Option<Self> {
        match *self {
            Vt::Real(r) => r.next_sibling().map(Vt::Real),
            Vt::Stand(doc, i) => doc.stands[i].next.map(|p| Vt::Stand(doc, p)),
        }
    }
    fn has_tag(&self, name: &str) -> bool {
        match self {
            Vt::Real(r) => r.has_tag(name),
            Vt::Stand(..) => self.stand().is_some_and(|s| s.tag == name),
        }
    }
    fn tag_eq(&self, other: &Self) -> bool {
        self.tag_name() == other.tag_name()
    }
    fn has_class(&self, name: &str) -> bool {
        match self {
            Vt::Real(r) => r.has_class(name),
            Vt::Stand(..) => self
                .stand()
                .is_some_and(|s| s.classes.iter().any(|c| &**c == name)),
        }
    }
    fn tag_name(&self) -> &str {
        match self {
            Vt::Real(r) => r.tag_name(),
            Vt::Stand(..) => self.stand().map_or("", |s| s.tag),
        }
    }
    fn each_class(&self, f: &mut dyn FnMut(&str)) {
        match self {
            Vt::Real(r) => r.each_class(f),
            Vt::Stand(..) => {
                for c in self.stand().into_iter().flat_map(|s| s.classes.iter()) {
                    f(c);
                }
            }
        }
    }
    fn has_scope(&self, name: &str) -> bool {
        match self {
            Vt::Real(r) => r.has_scope(name),
            Vt::Stand(..) => false,
        }
    }
    fn attr(&self, name: &str) -> Option<&str> {
        match self {
            Vt::Real(r) => r.attr(name),
            Vt::Stand(..) => (name == NAME_ATTR)
                .then(|| self.stand().and_then(|s| s.name.as_deref()))
                .flatten(),
        }
    }
    fn is_empty(&self) -> bool {
        match self {
            Vt::Real(r) => r.is_empty(),
            Vt::Stand(..) => self.stand().is_some_and(|s| s.empty),
        }
    }
    fn is_root(&self) -> bool {
        match self {
            Vt::Real(r) => r.is_root(),
            Vt::Stand(..) => false,
        }
    }
    fn is_view_transition_pseudo(&self) -> bool {
        matches!(self, Vt::Stand(..))
    }
}

/// The stand-in tree for the groups: index 0 is `sk-vt`, then per group its group, image pair and
/// whichever images exist (`None` for a missing one).
fn stand_ins(groups: &[Group]) -> (Vec<Stand>, Vec<[Option<usize>; 4]>) {
    let mut stands = vec![Stand {
        tag: TAGS[0],
        name: None,
        classes: Rc::from([]),
        parent: None,
        prev: None,
        next: None,
        empty: groups.is_empty(),
    }];
    let mut at = Vec::new();
    let mut last_group: Option<usize> = None;
    for g in groups {
        let push = |stands: &mut Vec<Stand>, tag, parent, prev: Option<usize>, empty| {
            let i = stands.len();
            stands.push(Stand {
                tag,
                name: Some(g.name.clone()),
                classes: g.classes.clone(),
                parent: Some(parent),
                prev,
                next: None,
                empty,
            });
            if let Some(p) = prev {
                stands[p].next = Some(i);
            }
            i
        };
        let group = push(&mut stands, TAGS[1], 0, last_group, false);
        last_group = Some(group);
        let pair = push(
            &mut stands,
            TAGS[2],
            group,
            None,
            g.old.is_none() && g.new.is_none(),
        );
        let old = g.old.map(|_| push(&mut stands, TAGS[3], pair, None, true));
        let new = g.new.map(|_| push(&mut stands, TAGS[4], pair, old, true));
        at.push([Some(group), Some(pair), old, new]);
    }
    (stands, at)
}

fn group_vars(g: &Group, new: Option<Geometry>) -> String {
    let old = g.old.map(|(_, geometry)| geometry).or(new);
    let new = new.or(old);
    let (Some(old), Some(new)) = (old, new) else {
        return String::new();
    };
    let mut vars = format!(
        "--sk-vt-width: {}px; --sk-vt-height: {}px; --sk-vt-transform: {}; \
         --sk-vt-old-width: {}px; --sk-vt-old-height: {}px; --sk-vt-old-transform: {};",
        new.width,
        new.height,
        matrix(&new.matrix),
        old.width,
        old.height,
        matrix(&old.matrix)
    );
    if g.old.is_some() && g.new.is_some() {
        vars.push_str(" --sk-vt-isolation: isolate; --sk-vt-blend: plus-lighter;");
    }
    vars
}

thread_local! {
    static NEW_GEOMETRY: RefCell<HashMap<u64, Geometry>> = RefCell::new(HashMap::new());
}

/// Cascades the stand-ins against the current sheet and new geometry, and keeps their animations
/// in step with what they compute.
fn restyle(now: Instant) {
    ACTIVE.with(|a| {
        let mut a = a.borrow_mut();
        let Some(t) = a.as_mut() else { return };
        let (stands, at) = stand_ins(&t.groups);
        let env = tree::env();
        let mut diagnostics = Vec::new();
        let computed = tree::read_tree(|tree| {
            let Some(root) = tree.node(tree.root) else {
                return None;
            };
            let root_vars = root
                .computed
                .as_ref()
                .map(|cs| cs.vars.clone())
                .unwrap_or_default();
            let root_color = root.computed.as_ref().and_then(|cs| cs.color);
            let doc = Doc { tree, stands };
            tree::with_sheet(|engine| {
                let mut compute = |i: usize, inline: &str, vars: &Rc<sk_css::Vars>| {
                    let inline = (!inline.is_empty())
                        .then(|| sk_css::parse_inline_style(inline, &mut diagnostics));
                    engine.compute_node(
                        Vt::Stand(&doc, i),
                        inline.as_ref(),
                        vars,
                        root_color,
                        &env,
                        &mut diagnostics,
                    )
                };
                let overlay = compute(0, "", &root_vars);
                let groups: Vec<[Option<ComputedStates>; 4]> = t
                    .groups
                    .iter()
                    .zip(&at)
                    .map(|(g, slots)| {
                        let new = g
                            .new
                            .and_then(|c| NEW_GEOMETRY.with(|n| n.borrow().get(&c).copied()));
                        let group =
                            compute(slots[0].unwrap_or(0), &group_vars(g, new), &overlay.vars);
                        let pair = compute(slots[1].unwrap_or(0), "", &group.vars);
                        let image = |slot: Option<usize>,
                                     compute: &mut dyn FnMut(
                            usize,
                            &str,
                            &Rc<sk_css::Vars>,
                        )
                            -> ComputedStates| {
                            slot.map(|i| compute(i, "", &pair.vars))
                        };
                        let old = image(slots[2], &mut compute);
                        let new = image(slots[3], &mut compute);
                        [Some(group), Some(pair), old, new]
                    })
                    .collect();
                Some((overlay, groups))
            })
        });
        let Some((overlay, groups)) = computed else {
            return;
        };
        if t.overlay.as_ref() != Some(&overlay) {
            tree::animate_detached(PSEUDO_BASE, &overlay, now);
            t.overlay = Some(overlay);
        }
        for (gi, (g, mut styles)) in t.groups.iter_mut().zip(groups).enumerate() {
            let timing = styles[0]
                .as_ref()
                .and_then(|s| s.animations.first().cloned());
            for (k, slot) in styles.iter_mut().enumerate() {
                let Some(states) = slot.as_mut() else {
                    continue;
                };
                if k > 0 {
                    states.animations = Rc::new(inherit_animation_timing(
                        &states.animations,
                        timing.as_ref(),
                    ));
                }
                if g.styles[k].as_ref() != Some(states) {
                    tree::animate_detached(pseudo_id(gi, k), states, now);
                }
            }
            g.styles = styles;
        }
    });
}

/// Records each new capture's geometry as of the last drawn frame: the groups follow the live
/// elements.
fn note_new_geometry(window: &Window) {
    let scale = window.scale_factor();
    let captures: Vec<u64> = ACTIVE.with(|a| {
        a.borrow()
            .as_ref()
            .map(|t| t.groups.iter().filter_map(|g| g.new).collect())
            .unwrap_or_default()
    });
    NEW_GEOMETRY.with(|n| {
        let mut n = n.borrow_mut();
        n.clear();
        for c in captures {
            if let Some(info) = window.vn_capture(c) {
                n.insert(c, geometry(&info, scale));
            }
        }
    });
}

fn styled(states: &ComputedStates, id: u32, now: Instant) -> gpui::Div {
    let mut d = div();
    *d.style() = (*states.base).clone();
    if let Some(overlay) = tree::animation_overlay(id, &states.base, now) {
        d.style().refine(&overlay);
    }
    d
}

fn image(
    states: &ComputedStates,
    id: u32,
    capture: u64,
    aspect: Option<f32>,
    now: Instant,
) -> AnyElement {
    let mut d = styled(states, id, now);
    // A replaced box's natural ratio: it decides whichever side is `auto`.
    if !d.style().aspect_ratio.is_some_and(|r| r > 0.0) {
        d.style().aspect_ratio = aspect;
    }
    let plus_lighter = states.extras.plus_lighter == Some(true);
    d.child(
        canvas(
            |_, _, _| (),
            move |bounds, _, window, _| window.vn_paint_snapshot(capture, bounds, plus_lighter),
        )
        .size_full(),
    )
    .into_any_element()
}

/// What the window shows over the document while a transition runs: the old state frozen until
/// the pseudo-elements exist, then those. It takes every pointer event.
pub fn build_overlay() -> Option<AnyElement> {
    let now = Instant::now();
    let phase = ACTIVE.with(|a| a.borrow().as_ref().map(|t| t.phase))?;
    let root = div()
        .id("sk-view-transition")
        .absolute()
        .top_0()
        .left_0()
        .size_full()
        .occlude();
    if phase != Phase::Animate {
        let frozen: Vec<u64> = ACTIVE.with(|a| {
            a.borrow()
                .as_ref()
                .map(|t| {
                    std::iter::once(capture_id(t.seq, OLD, 0))
                        .chain(t.old.iter().map(|n| n.capture))
                        .collect()
                })
                .unwrap_or_default()
        });
        let mut seen = HashSet::new();
        let frozen: Vec<u64> = frozen.into_iter().filter(|c| seen.insert(*c)).collect();
        return Some(
            root.child(
                canvas(
                    |_, _, _| (),
                    move |_, _, window, _| {
                        for c in &frozen {
                            window.vn_paint_snapshot_in_place(*c);
                        }
                    },
                )
                .size_full(),
            )
            .into_any_element(),
        );
    }
    ACTIVE.with(|a| {
        let a = a.borrow();
        let t = a.as_ref()?;
        let mut overlay = match &t.overlay {
            Some(states) => {
                let mut d = styled(states, PSEUDO_BASE, now);
                let style = d.style();
                style.position = Some(gpui::Position::Absolute);
                style.inset.top = Some(gpui::px(0.).into());
                style.inset.left = Some(gpui::px(0.).into());
                style.size.width = Some(gpui::relative(1.).into());
                style.size.height = Some(gpui::relative(1.).into());
                d
            }
            None => div().absolute().top_0().left_0().size_full(),
        };
        for (gi, g) in t.groups.iter().enumerate() {
            let [Some(group), Some(pair), old, new] = &g.styles else {
                continue;
            };
            let aspect = |geometry: Option<Geometry>| {
                geometry
                    .filter(|g| g.height > 0.)
                    .map(|g| g.width / g.height)
            };
            let mut pair_el = styled(pair, pseudo_id(gi, 1), now);
            pair_el.style().vn_isolate = Some(pair.extras.isolate == Some(true));
            if let (Some(states), Some((capture, geometry))) = (old, g.old) {
                pair_el = pair_el.child(image(
                    states,
                    pseudo_id(gi, 2),
                    capture,
                    aspect(Some(geometry)),
                    now,
                ));
            }
            if let (Some(states), Some(capture)) = (new, g.new) {
                let geometry = NEW_GEOMETRY.with(|n| n.borrow().get(&capture).copied());
                pair_el = pair_el.child(image(
                    states,
                    pseudo_id(gi, 3),
                    capture,
                    aspect(geometry),
                    now,
                ));
            }
            overlay = overlay.child(styled(group, pseudo_id(gi, 0), now).child(pair_el));
        }
        Some(root.child(overlay).into_any_element())
    })
}

#[cfg(test)]
pub(crate) fn reset() {
    ACTIVE.with(|a| a.borrow_mut().take());
    NOTES.with(|n| n.borrow_mut().clear());
    FORGET.with(|f| f.borrow_mut().clear());
    NEW_GEOMETRY.with(|n| n.borrow_mut().clear());
}

/// The phase changes a drawn frame would make, without a window: the old state is captured, the
/// new one is, the animations finish.
#[cfg(test)]
pub(crate) fn step() {
    let phase = ACTIVE.with(|a| a.borrow().as_ref().map(|t| t.phase));
    match phase {
        Some(Phase::Capture) => ACTIVE.with(|a| {
            if let Some(t) = a.borrow_mut().as_mut() {
                t.phase = Phase::Update;
                note(Note::Update(t.seq));
            }
        }),
        Some(Phase::AwaitNew) => ACTIVE.with(|a| {
            if let Some(t) = a.borrow_mut().as_mut() {
                t.phase = Phase::Animate;
                note(Note::Ready(t.seq));
            }
        }),
        Some(Phase::Animate) => end(None),
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tree::{read_tree, restyle_all, set_stylesheet, with_tree};
    use gpui::{point, px, size, Bounds, ScaledPixels, TransformationMatrix};

    fn element(t: &mut Tree, parent: u32, class: &str) -> u32 {
        let div = t.intern("div");
        let el = t.create_element(div);
        t.set_class(el, class);
        t.insert(el, parent, 0);
        el
    }

    #[test]
    fn a_group_transform_puts_the_box_where_the_element_painted() {
        let m = TransformationMatrix {
            rotation_scale: [[0., -2.], [2., 0.]],
            translation: [300., 40.],
        };
        let info = gpui::VnCaptureInfo {
            id: 1,
            border_box: Bounds {
                origin: point(ScaledPixels(20.), ScaledPixels(10.)),
                size: size(ScaledPixels(200.), ScaledPixels(100.)),
            },
            transform: m,
        };
        let g = geometry(&info, 2.);
        assert_eq!((g.width, g.height), (100., 50.));
        let [a, b, c, d, e, f] = g.matrix;
        for (qx, qy) in [(0., 0.), (100., 0.), (0., 50.), (100., 50.)] {
            let (x, y) = (qx - 50., qy - 25.);
            let css = (a * x + c * y + e + 50., b * x + d * y + f + 25.);
            let device = m.apply(point(px(20. + 2. * qx), px(10. + 2. * qy)));
            let (dx, dy) = (f32::from(device.x) / 2., f32::from(device.y) / 2.);
            assert!((css.0 - dx).abs() < 1e-3 && (css.1 - dy).abs() < 1e-3);
        }
    }

    #[test]
    fn names_are_collected_in_tree_order_and_a_duplicate_is_refused() {
        set_stylesheet(
            ".hero { view-transition-name: hero } \
             .card { view-transition-name: card; view-transition-class: big wide } \
             .gone { display: none } .gone .hero2 { view-transition-name: hidden }"
                .into(),
        );
        let (root, hero, card, twin) = with_tree(|t| {
            let root = t.create_root();
            let card = element(t, root, "card");
            let hero = element(t, root, "hero");
            let gone = element(t, root, "gone");
            element(t, gone, "hero2");
            let twin = element(t, root, "");
            (root, hero, card, twin)
        });
        restyle_all();
        let named = read_tree(|t| collect(t)).unwrap();
        let names: Vec<(&str, u32)> = named.iter().map(|(n, _, id)| (&**n, *id)).collect();
        assert_eq!(names, [("root", root), ("card", card), ("hero", hero)]);
        assert_eq!(
            named[1].1.iter().map(|c| &**c).collect::<Vec<_>>(),
            ["big", "wide"]
        );
        with_tree(|t| t.set_class(twin, "card"));
        restyle_all();
        assert!(read_tree(|t| collect(t)).is_err_and(|e| e.contains("card")));
        set_stylesheet(
            ":root { view-transition-name: none } .hero { view-transition-name: hero }".into(),
        );
        with_tree(|t| t.mark_dirty());
        restyle_all();
        let named = read_tree(|t| collect(t)).expect("no duplicate once `.card` names nothing");
        assert_eq!(
            named.iter().map(|(n, ..)| &**n).collect::<Vec<_>>(),
            ["hero"]
        );
    }

    /// Transition 9 animating a "card" group (old and new) and a "solo" one (new only).
    fn animating(types: Vec<String>) {
        let old = (
            capture_id(9, OLD, 1),
            Geometry {
                width: 10.,
                height: 10.,
                matrix: [1., 0., 0., 1., 0., 0.],
            },
        );
        let (card_new, solo_new) = (capture_id(9, NEW, 1), capture_id(9, NEW, 2));
        let group = |name: &str, old, new| Group {
            name: name.into(),
            classes: Rc::from([]),
            old,
            new: Some(new),
            styles: Default::default(),
        };
        ACTIVE.with(|a| {
            *a.borrow_mut() = Some(Transition {
                seq: 9,
                phase: Phase::Animate,
                types,
                old: Vec::new(),
                new: Vec::new(),
                captures: HashMap::new(),
                groups: vec![
                    group("card", Some(old), card_new),
                    group("solo", None, solo_new),
                ],
                overlay: None,
                since: Instant::now(),
            })
        });
        NEW_GEOMETRY.with(|n| {
            n.borrow_mut().insert(
                card_new,
                Geometry {
                    width: 40.,
                    height: 20.,
                    matrix: [1., 0., 0., 1., 5., 6.],
                },
            );
            n.borrow_mut().insert(
                solo_new,
                Geometry {
                    width: 8.,
                    height: 8.,
                    matrix: [1., 0., 0., 1., 0., 0.],
                },
            );
        });
    }

    #[test]
    fn reduced_motion_leaves_the_pseudo_elements_nothing_to_run() {
        reset();
        set_stylesheet(String::new());
        with_tree(|t| t.create_root());
        restyle_all();
        tree::set_reduce_motion(true);
        animating(Vec::new());
        let now = Instant::now();
        restyle(now);
        let finished =
            (0..2).all(|g| (0..4).all(|k| tree::animations_finished(pseudo_id(g, k), now)));
        tree::set_reduce_motion(false);
        end(None);
        assert!(finished);
    }

    #[test]
    fn pseudo_elements_cascade_with_their_groups_geometry_and_timing() {
        reset();
        set_stylesheet(
            "::view-transition-group(*) { animation-duration: 2s } \
             ::view-transition-old(card) { animation-name: slide } \
             @keyframes slide { to { opacity: 0.2 } } \
             :active-view-transition-type(forward) .x { opacity: 0.5 }"
                .into(),
        );
        let x = with_tree(|t| {
            let root = t.create_root();
            element(t, root, "x")
        });
        restyle_all();
        animating(vec!["forward".into()]);
        sync_root_prop();
        restyle_all();
        let opacity = |id| {
            read_tree(|t| {
                t.node(id)
                    .and_then(|n| n.computed.as_ref())
                    .and_then(|c| c.base.opacity)
            })
        };
        assert_eq!(opacity(x), Some(0.5));
        let now = Instant::now();
        restyle(now);
        ACTIVE.with(|a| {
            let a = a.borrow();
            let t = a.as_ref().unwrap();
            let [Some(group), Some(pair), Some(old), Some(new)] = &t.groups[0].styles else {
                panic!("the card has every part");
            };
            assert_eq!(group.base.size.width, Some(px(40.).into()));
            assert_eq!(group.animations[0].duration, Duration::from_secs(2));
            assert_eq!(pair.extras.isolate, Some(true));
            assert_eq!(&*old.animations[0].name, "slide");
            assert_eq!(old.animations[0].duration, Duration::from_secs(2));
            assert_eq!(old.extras.plus_lighter, Some(true));
            assert_eq!(&*new.animations[0].name, "-ua-view-transition-fade-in");
            let [_, Some(solo_pair), None, Some(solo_new)] = &t.groups[1].styles else {
                panic!("the solo group has no old image");
            };
            assert_ne!(solo_pair.extras.isolate, Some(true));
            assert_ne!(solo_new.extras.plus_lighter, Some(true));
        });
        assert!(!tree::animations_finished(pseudo_id(0, 0), now));
        end(None);
        assert_eq!(take_notes(), [Note::Finished(9)]);
        assert!(tree::animations_finished(pseudo_id(0, 0), now));
        sync_root_prop();
        restyle_all();
        assert_eq!(opacity(x), None);
    }
}
