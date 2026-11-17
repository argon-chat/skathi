//! Document text selection: mouse selection over painted text, the `Selection` a page reads and
//! writes, and what Ctrl+C copies outside a text field.
//!
//! A selection is two DOM boundary points ([`Pos`]: a text node and UTF-16 offset, an element and a
//! child index, or an element and an offset into its own text), so `getSelection()` round-trips them.
//! It costs nothing per frame while nobody selects ([`PAINTING`], [`RECORDING`] are the two flags).

use std::cell::{Cell, RefCell};
use std::collections::HashMap;

use gpui::{
    AnyElement, App, Bounds, DispatchPhase, Element, ElementId, GlobalElementId, HighlightStyle,
    InspectorElementId, InteractiveElement, IntoElement, LayoutId, MouseButton, MouseDownEvent,
    MouseMoveEvent, MouseUpEvent, Pixels, Point, StyledText, TextLayout, Window,
};
use sk_css::{TextTransform, UserSelect, WhiteSpaceMode};

use crate::tree::{self, Kind, Node, Tree};
use crate::windows::{self, WinKey};

/// `selectstart` (cancelable, bubbles) and `selectionchange` (at `document`).
pub(crate) use crate::tree::{EV_SELECTIONCHANGE, EV_SELECTSTART};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Pos {
    pub(crate) node: u32,
    pub(crate) offset: u32,
    /// `node` is an element and `offset` is into the text it carries itself.
    pub(crate) own: bool,
}

impl Pos {
    pub(crate) fn at(node: u32, offset: u32) -> Pos {
        Pos {
            node,
            offset,
            own: false,
        }
    }
    fn own(node: u32, offset: u32) -> Pos {
        Pos {
            node,
            offset,
            own: true,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Us {
    None,
    Text,
    All,
}

/// `auto` takes `none` and `all` from the parent; anything else is `text`.
fn used(parent: Us, declared: Option<UserSelect>) -> Us {
    match declared {
        Some(UserSelect::None) => Us::None,
        Some(UserSelect::Text) => Us::Text,
        Some(UserSelect::All) => Us::All,
        Some(UserSelect::Auto) | None => match parent {
            Us::Text => Us::Text,
            other => other,
        },
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Grain {
    Char,
    Word,
    Block,
}

struct Gesture {
    win: WinKey,
    press: Point<Pixels>,
    last: Point<Pixels>,
    grain: Grain,
    extend: bool,
    dragging: bool,
    released: bool,
    anchor: Option<(Pos, Pos)>,
    waits: u8,
}

struct Recorded {
    layout: TextLayout,
    visible: Bounds<Pixels>,
    win: WinKey,
    ws: Option<WhiteSpaceMode>,
    case: Option<TextTransform>,
    lead: usize,
    len: usize,
}

#[derive(Default)]
struct State {
    sel: Option<(Pos, Pos)>,
    painted: HashMap<u32, (u32, u32)>,
    gesture: Option<Gesture>,
    layouts: HashMap<u32, Recorded>,
    change_owed: bool,
}

thread_local! {
    static STATE: RefCell<State> = RefCell::new(State::default());
    static PAINTING: Cell<bool> = const { Cell::new(false) };
    static RECORDING: Cell<bool> = const { Cell::new(false) };
    static WORK: Cell<bool> = const { Cell::new(false) };
    static PRESS_PREVENTED: Cell<bool> = const { Cell::new(false) };
}

fn set_work(s: &State) {
    WORK.with(|c| c.set(s.gesture.is_some() || s.change_owed));
}

fn path(t: &Tree, id: u32) -> Option<Vec<u32>> {
    let mut out = Vec::new();
    let mut cur = id;
    for _ in 0..4096 {
        let n = t.node(cur)?;
        if n.parent == 0 {
            if cur != t.root {
                return None;
            }
            out.reverse();
            return Some(out);
        }
        let parent = t.node(n.parent)?;
        out.push(parent.children.iter().position(|&c| c == cur)? as u32);
        cur = n.parent;
    }
    None
}

fn key(t: &Tree, p: Pos) -> Option<Vec<u32>> {
    let mut k = path(t, p.node)?;
    if p.own {
        k.push(0);
    }
    k.push(p.offset);
    Some(k)
}

fn length(t: &Tree, id: u32, own: bool) -> u32 {
    match t.node(id) {
        Some(n) if n.kind == Kind::Text || own => utf16_len(&n.text),
        Some(n) if n.kind == Kind::Element && !n.text.is_empty() => 1,
        Some(n) => n.children.len() as u32,
        None => 0,
    }
}

fn utf16_len(s: &str) -> u32 {
    s.chars().map(|c| c.len_utf16() as u32).sum()
}

fn ordered(t: &Tree, a: Pos, f: Pos) -> Option<(Vec<u32>, Vec<u32>)> {
    let (ka, kf) = (key(t, a)?, key(t, f)?);
    Some(if ka <= kf { (ka, kf) } else { (kf, ka) })
}

enum Step {
    Enter(u32),
    Leave(u32),
    Text {
        holder: u32,
        from: u32,
        to: u32,
        ws: Option<WhiteSpaceMode>,
        case: Option<TextTransform>,
    },
}

#[derive(Clone, Copy)]
struct Ctx {
    us: Us,
    ws: Option<WhiteSpaceMode>,
    case: Option<TextTransform>,
}

fn skipped(t: &Tree, n: &Node) -> bool {
    let tag = t.name(n.tag);
    crate::render::is_metadata(tag)
        || matches!(
            tag,
            "input" | "textarea" | "select" | "svg" | "video" | "canvas" | "img"
        )
        || n.computed
            .as_ref()
            .is_some_and(|cs| cs.base.display == Some(gpui::Display::None))
}

fn clip(p: &[u32], start: &[u32], end: &[u32], len: u32) -> (u32, u32) {
    let from = if start.len() > p.len() && start.starts_with(p) {
        start[p.len()].min(len)
    } else if start <= p {
        0
    } else {
        len
    };
    let to = if end.len() > p.len() && end.starts_with(p) {
        end[p.len()].min(len)
    } else if end > p {
        len
    } else {
        0
    };
    (from, to.max(from))
}

fn walk(t: &Tree, start: &[u32], end: &[u32], f: &mut dyn FnMut(&Tree, Step, Ctx)) {
    let mut p = Vec::new();
    let ctx = Ctx {
        us: Us::Text,
        ws: None,
        case: None,
    };
    walk_node(t, t.root, &mut p, start, end, ctx, f);
}

fn walk_node(
    t: &Tree,
    id: u32,
    p: &mut Vec<u32>,
    start: &[u32],
    end: &[u32],
    ctx: Ctx,
    f: &mut dyn FnMut(&Tree, Step, Ctx),
) {
    let Some(n) = t.node(id) else { return };
    match n.kind {
        Kind::Anchor => {}
        Kind::Text => {
            if !n.text.is_empty() {
                let (from, to) = clip(p, start, end, utf16_len(&n.text));
                if from < to {
                    f(
                        t,
                        Step::Text {
                            holder: id,
                            from,
                            to,
                            ws: ctx.ws,
                            case: ctx.case,
                        },
                        ctx,
                    );
                }
            }
        }
        Kind::Element => {
            if skipped(t, n) {
                return;
            }
            let extras = n.computed.as_ref().map(|cs| &cs.extras);
            let ctx = Ctx {
                us: used(ctx.us, extras.and_then(|x| x.user_select)),
                ws: extras.and_then(|x| x.white_space).or(ctx.ws),
                case: extras.and_then(|x| x.text_transform).or(ctx.case),
            };
            f(t, Step::Enter(id), ctx);
            if !n.text.is_empty() {
                p.push(0);
                let (from, to) = clip(p, start, end, utf16_len(&n.text));
                p.pop();
                if from < to {
                    f(
                        t,
                        Step::Text {
                            holder: id,
                            from,
                            to,
                            ws: ctx.ws,
                            case: ctx.case,
                        },
                        ctx,
                    );
                }
            } else {
                for (i, &c) in n.children.iter().enumerate() {
                    p.push(i as u32);
                    let before = &p[..] < start && !start.starts_with(p);
                    let after = &p[..] >= end;
                    if !before && !after {
                        walk_node(t, c, p, start, end, ctx, f);
                    }
                    p.pop();
                    if after {
                        break;
                    }
                }
            }
            f(t, Step::Leave(id), ctx);
        }
    }
}

/// The text a holder paints, as the build shapes it (white-space, trim, text-transform), with the
/// DOM offset each painted character came from.
pub(crate) struct TextMap {
    display: String,
    starts: Vec<u32>,
    dom: Vec<u32>,
    dom_end: u32,
}

fn collapse(raw: &str, mode: WhiteSpaceMode) -> Vec<(usize, char, u32)> {
    const COLLAPSIBLE: [char; 5] = [' ', '\t', '\n', '\r', '\u{c}'];
    let mut out = Vec::with_capacity(raw.len());
    let (mut byte, mut at) = (0usize, 0u32);
    if !mode.collapses_spaces() {
        for c in raw.chars() {
            out.push((byte, c, at));
            byte += c.len_utf8();
            at += c.len_utf16() as u32;
        }
        return out;
    }
    let keeps = mode.preserves_newlines();
    let mut run: Option<(bool, u32)> = None;
    for c in raw.chars() {
        let units = c.len_utf16() as u32;
        if COLLAPSIBLE.contains(&c) {
            let (saw, from) = run.unwrap_or((false, at));
            run = Some((saw || c == '\n', from));
            at += units;
            continue;
        }
        if let Some((saw, from)) = run.take() {
            out.push((byte, if keeps && saw { '\n' } else { ' ' }, from));
            byte += 1;
        }
        out.push((byte, c, at));
        byte += c.len_utf8();
        at += units;
    }
    if let Some((saw, from)) = run {
        out.push((byte, if keeps && saw { '\n' } else { ' ' }, from));
    }
    out
}

impl TextMap {
    pub(crate) fn new(
        raw: &str,
        ws: Option<WhiteSpaceMode>,
        lead: usize,
        len: usize,
        case: Option<TextTransform>,
    ) -> TextMap {
        let all = collapse(raw, ws.unwrap_or(WhiteSpaceMode::Normal));
        let end = lead + len;
        let shown: Vec<(char, u32)> = all
            .iter()
            .filter(|(b, ..)| *b >= lead && *b < end)
            .map(|&(_, c, at)| (c, at))
            .collect();
        let dom_end = all
            .iter()
            .find(|(b, ..)| *b >= end)
            .map(|x| x.2)
            .unwrap_or_else(|| utf16_len(raw));
        let plain: String = shown.iter().map(|x| x.0).collect();
        let display = case.and_then(|c| c.apply(&plain)).unwrap_or(plain);
        let (n, m) = (shown.len(), display.chars().count());
        let mut starts = Vec::with_capacity(m);
        let mut dom = Vec::with_capacity(m);
        for (j, (b, _)) in display.char_indices().enumerate() {
            starts.push(b as u32);
            let i = if m == n { j } else { j * n / m.max(1) };
            dom.push(shown.get(i).map(|x| x.1).unwrap_or(dom_end));
        }
        TextMap {
            display,
            starts,
            dom,
            dom_end,
        }
    }

    pub(crate) fn to_dom(&self, byte: usize) -> u32 {
        let i = self.starts.partition_point(|&s| (s as usize) < byte);
        self.dom.get(i).copied().unwrap_or(self.dom_end)
    }

    pub(crate) fn to_display(&self, dom: u32) -> usize {
        let i = self.dom.partition_point(|&d| d < dom);
        self.starts
            .get(i)
            .map_or(self.display.len(), |&s| s as usize)
    }

    fn slice(&self, from: u32, to: u32) -> &str {
        &self.display[self.to_display(from)..self.to_display(to)]
    }
}

fn map_of(
    t: &Tree,
    holder: u32,
    ws: Option<WhiteSpaceMode>,
    case: Option<TextTransform>,
) -> Option<TextMap> {
    let n = t.node(holder)?;
    let collapsed = crate::render::processed_text(&n.text, ws.unwrap_or(WhiteSpaceMode::Normal));
    let shown = match n.kind {
        Kind::Text => crate::render::trim_edges(t, n, holder, &collapsed, ws),
        _ => crate::render::trim_own_edges(t, n, &collapsed, ws),
    };
    let lead = shown.as_ptr() as usize - collapsed.as_ptr() as usize;
    Some(TextMap::new(&n.text, ws, lead, shown.len(), case))
}

fn painted_of(t: &Tree, sel: Option<(Pos, Pos)>) -> HashMap<u32, (u32, u32)> {
    let mut out = HashMap::new();
    let Some((a, f)) = sel else { return out };
    let Some((start, end)) = ordered(t, a, f) else {
        return out;
    };
    if start == end {
        return out;
    }
    walk(t, &start, &end, &mut |_, step, ctx| {
        if let Step::Text {
            holder, from, to, ..
        } = step
        {
            if ctx.us != Us::None {
                out.insert(holder, (from, to));
            }
        }
    });
    out
}

fn breaks_of(t: &Tree, n: &Node) -> u8 {
    if !is_block(t, n) {
        return 0;
    }
    if t.name(n.tag) == "p" {
        2
    } else {
        1
    }
}

fn is_block(t: &Tree, n: &Node) -> bool {
    if n.kind != Kind::Element {
        return false;
    }
    if let Some(parent) = t.node(n.parent) {
        let pd = parent.computed.as_ref().and_then(|cs| cs.base.display);
        if matches!(pd, Some(gpui::Display::Flex) | Some(gpui::Display::Grid)) {
            return true;
        }
    }
    let cs = n.computed.as_ref();
    if cs.is_some_and(|cs| cs.extras.inline_level == Some(true)) {
        return false;
    }
    let declared = cs.and_then(|cs| cs.base.display);
    !(declared.is_none() && crate::render::INLINE_LEVEL_TAGS.contains(&t.name(n.tag)))
}

/// The selection's text as `Selection.toString()` and a copy give it: painted text in document
/// order, line breaks between blocks, tabs between cells, nothing from `user-select: none`.
fn text_between(t: &Tree, start: &[u32], end: &[u32]) -> String {
    let mut out = String::new();
    let mut breaks = 0u8;
    let mut tab = false;
    walk(t, start, end, &mut |t, step, ctx| match step {
        Step::Enter(id) | Step::Leave(id) => {
            let Some(n) = t.node(id) else { return };
            let tag = t.name(n.tag);
            if matches!(step, Step::Enter(_)) && tag == "br" && ctx.us != Us::None {
                out.push('\n');
                breaks = 0;
                tab = false;
                return;
            }
            if matches!(step, Step::Leave(_)) && matches!(tag, "td" | "th") {
                tab = true;
            }
            breaks = breaks.max(breaks_of(t, n));
        }
        Step::Text {
            holder,
            from,
            to,
            ws,
            case,
        } => {
            if ctx.us == Us::None {
                return;
            }
            let Some(map) = map_of(t, holder, ws, case) else {
                return;
            };
            let piece = map.slice(from, to);
            if piece.is_empty() {
                return;
            }
            if !out.is_empty() {
                if breaks > 0 {
                    while out.ends_with(' ') {
                        out.pop();
                    }
                    for _ in 0..breaks {
                        out.push('\n');
                    }
                } else if tab {
                    out.push('\t');
                }
            }
            let piece = if out.is_empty() || out.ends_with('\n') {
                piece.trim_start_matches(' ')
            } else {
                piece
            };
            out.push_str(piece);
            breaks = 0;
            tab = false;
        }
    });
    let trimmed = out.trim_end_matches(' ').len();
    out.truncate(trimmed);
    out
}

pub(crate) fn selected_text() -> String {
    let Some((a, f)) = STATE.with(|s| s.borrow().sel) else {
        return String::new();
    };
    tree::read_tree(|t| match ordered(t, a, f) {
        Some((start, end)) if start != end => text_between(t, &start, &end),
        _ => String::new(),
    })
}

/// What Ctrl+C outside a text field copies; an editable `focused` control copies its own.
pub(crate) fn text_for_copy(focused: u32) -> Option<String> {
    if focused != 0 && tree::read_tree(|t| editable_at(t, focused)) {
        return None;
    }
    Some(selected_text()).filter(|s| !s.is_empty())
}

fn replace(t: &mut Tree, sel: Option<(Pos, Pos)>) -> bool {
    let next = painted_of(t, sel);
    let changed = STATE.with(|s| {
        let mut s = s.borrow_mut();
        let moved = s.sel != sel;
        let mut touched = Vec::new();
        for (id, range) in &next {
            if s.painted.get(id) != Some(range) {
                touched.push(*id);
            }
        }
        for id in s.painted.keys() {
            if !next.contains_key(id) {
                touched.push(*id);
            }
        }
        s.sel = sel;
        s.painted = next;
        PAINTING.with(|c| c.set(!s.painted.is_empty()));
        if moved {
            s.change_owed = true;
        }
        set_work(&s);
        (moved, touched)
    });
    for id in &changed.1 {
        t.touch(*id);
    }
    changed.0
}

fn replace_live(sel: Option<(Pos, Pos)>, window: &mut Window, cx: &mut App) {
    if tree::with_tree(|t| replace(t, sel)) {
        crate::render::request_repaint(window, cx);
    }
}

fn replace_from_js(sel: Option<(Pos, Pos)>) {
    tree::with_tree(|t| {
        if replace(t, sel) {
            t.mark_dirty();
        }
    });
}

pub(crate) fn current() -> Option<(Pos, Pos)> {
    STATE.with(|s| s.borrow().sel)
}

/// After a mutation wave: clears a boundary whose node went away, clamps offsets, recomputes the
/// painted parts.
pub(crate) fn gc(t: &mut Tree) {
    let (sel, idle) = STATE.with(|s| {
        let mut s = s.borrow_mut();
        s.layouts.retain(|&id, _| t.node(id).is_some());
        (s.sel, s.sel.is_none() && s.painted.is_empty())
    });
    if idle {
        return;
    }
    let fixed = sel.and_then(|(a, f)| {
        let fix = |p: Pos| -> Option<Pos> {
            path(t, p.node)?;
            Some(Pos {
                offset: p.offset.min(length(t, p.node, p.own)),
                ..p
            })
        };
        Some((fix(a)?, fix(f)?))
    });
    replace(t, fixed);
}

pub(crate) fn rebase(element: u32, text: u32) {
    let sel = STATE.with(|s| s.borrow().sel);
    let Some((a, f)) = sel else { return };
    let move_one = |p: Pos| {
        if p.own && p.node == element {
            Pos::at(text, p.offset)
        } else {
            p
        }
    };
    let moved = (move_one(a), move_one(f));
    if moved != (a, f) {
        replace_from_js(Some(moved));
    }
}

/// Focus moved to `id`: a text field takes the selection into itself, so the document's goes.
pub(crate) fn focus_moved(id: u32) {
    if STATE.with(|s| s.borrow().sel.is_none()) {
        return;
    }
    tree::with_tree(|t| {
        if editable_at(t, id) {
            replace(t, None);
        }
    });
}

pub(crate) fn set_from_js(points: Option<(Pos, Pos)>) -> bool {
    let checked = points.and_then(|(a, f)| {
        tree::read_tree(|t| {
            let fix = |p: Pos| -> Option<Pos> {
                path(t, p.node)?;
                let n = t.node(p.node)?;
                if n.kind == Kind::Element && !n.text.is_empty() && !p.own {
                    let len = utf16_len(&n.text);
                    return Some(Pos::own(p.node, if p.offset == 0 { 0 } else { len }));
                }
                Some(Pos {
                    offset: p.offset.min(length(t, p.node, p.own)),
                    ..p
                })
            };
            Some((fix(a)?, fix(f)?))
        })
    });
    let ok = checked.is_some() || points.is_none();
    replace_from_js(checked);
    ok
}

fn select_all(window: &mut Window, cx: &mut App) {
    let k = windows::key(window);
    let sel = tree::read_tree(|t| {
        let root = match windows::root_of(k) {
            0 => t.root,
            popped => popped,
        };
        let n = t.node(root)?;
        Some(if n.text.is_empty() {
            (Pos::at(root, 0), Pos::at(root, n.children.len() as u32))
        } else {
            (Pos::own(root, 0), Pos::own(root, utf16_len(&n.text)))
        })
    });
    replace_live(sel, window, cx);
}

pub(crate) fn text_element(
    t: &Tree,
    holder: u32,
    raw: &str,
    ws: Option<WhiteSpaceMode>,
    lead: usize,
    shown: &str,
    case: Option<TextTransform>,
) -> AnyElement {
    let display = crate::render::transformed_text(shown, case);
    let range = if PAINTING.with(|c| c.get()) {
        STATE.with(|s| s.borrow().painted.get(&holder).copied())
    } else {
        None
    };
    let record = RECORDING.with(|c| c.get());
    if range.is_none() && !record {
        return display.into_any_element();
    }
    let mut styled = StyledText::new(display);
    if let Some((from, to)) = range {
        let map = TextMap::new(raw, ws, lead, shown.len(), case);
        let (a, b) = (map.to_display(from), map.to_display(to));
        if a < b {
            styled = styled.with_highlights([(a..b, highlight(t, holder))]);
        }
    }
    if record {
        Recorder {
            holder,
            ws,
            case,
            lead,
            len: shown.len(),
            inner: styled,
        }
        .into_any_element()
    } else {
        styled.into_any_element()
    }
}

/// The default highlight: the translucent blue a text field paints its own selection with.
const DEFAULT_HIGHLIGHT: gpui::Hsla = gpui::Hsla {
    h: 0.6,
    s: 0.8,
    l: 0.6,
    a: 0.35,
};

/// `::selection` for a holder: the nearest element up whose cascade gave it one (Chromium's inheritance).
fn highlight(t: &Tree, holder: u32) -> HighlightStyle {
    let mut cur = match t.node(holder) {
        Some(n) if n.kind == Kind::Text => n.parent,
        _ => holder,
    };
    for _ in 0..256 {
        let Some(n) = t.node(cur) else { break };
        if let Some(sel) = n.computed.as_ref().and_then(|cs| cs.selection.as_ref()) {
            let bg = sel
                .background
                .as_ref()
                .and_then(gpui::Fill::color)
                .and_then(|b| b.as_solid());
            return HighlightStyle {
                background_color: Some(bg.unwrap_or(DEFAULT_HIGHLIGHT)),
                color: sel.text.color,
                ..Default::default()
            };
        }
        if n.parent == 0 {
            break;
        }
        cur = n.parent;
    }
    HighlightStyle {
        background_color: Some(DEFAULT_HIGHLIGHT),
        ..Default::default()
    }
}

struct Recorder {
    holder: u32,
    ws: Option<WhiteSpaceMode>,
    case: Option<TextTransform>,
    lead: usize,
    len: usize,
    inner: StyledText,
}

impl Element for Recorder {
    type RequestLayoutState = ();
    type PrepaintState = ();

    fn id(&self) -> Option<ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static std::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        id: Option<&GlobalElementId>,
        inspector_id: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, ()) {
        self.inner.request_layout(id, inspector_id, window, cx)
    }

    fn prepaint(
        &mut self,
        id: Option<&GlobalElementId>,
        inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _: &mut (),
        window: &mut Window,
        cx: &mut App,
    ) {
        self.inner
            .prepaint(id, inspector_id, bounds, &mut (), window, cx);
        let layout = self.inner.layout().clone();
        let visible = layout.bounds().intersect(&window.content_mask().bounds);
        let rec = Recorded {
            layout,
            visible,
            win: windows::key(window),
            ws: self.ws,
            case: self.case,
            lead: self.lead,
            len: self.len,
        };
        STATE.with(|s| s.borrow_mut().layouts.insert(self.holder, rec));
    }

    fn paint(
        &mut self,
        id: Option<&GlobalElementId>,
        inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _: &mut (),
        _: &mut (),
        window: &mut Window,
        cx: &mut App,
    ) {
        self.inner
            .paint(id, inspector_id, bounds, &mut (), &mut (), window, cx);
    }
}

fn editable_at(t: &Tree, id: u32) -> bool {
    up(t, id, |t, n| {
        matches!(t.name(n.tag), "input" | "textarea" | "select")
    })
}

fn up(t: &Tree, id: u32, pred: impl Fn(&Tree, &Node) -> bool) -> bool {
    let mut cur = id;
    for _ in 0..4096 {
        let Some(n) = t.node(cur) else { return false };
        if n.kind == Kind::Element && pred(t, n) {
            return true;
        }
        if n.parent == 0 {
            return false;
        }
        cur = n.parent;
    }
    false
}

fn listened(t: &Tree, target: u32, code: u32) -> bool {
    let bit = 1u64 << code;
    let mut cur = target;
    for _ in 0..4096 {
        let Some(n) = t.node(cur) else { return false };
        if n.listeners & bit != 0 {
            return true;
        }
        if n.parent == 0 {
            return false;
        }
        cur = n.parent;
    }
    false
}

fn used_at(t: &Tree, id: u32) -> Us {
    let mut chain = Vec::new();
    let mut cur = id;
    for _ in 0..4096 {
        let Some(n) = t.node(cur) else { break };
        chain.push(n.computed.as_ref().and_then(|cs| cs.extras.user_select));
        if n.parent == 0 {
            break;
        }
        cur = n.parent;
    }
    chain.iter().rev().fold(Us::Text, |us, d| used(us, *d))
}

/// The topmost node under `p` in the frame gpui last painted, so a selection starts where the
/// press's events go.
fn topmost(window: &Window, p: Point<Pixels>) -> Option<u32> {
    crate::pointer::topmost(window, p)
}

fn contains(r: crate::windows::Rect, p: Point<Pixels>) -> bool {
    let (x, y) = (f32::from(p.x), f32::from(p.y));
    x >= r.0 && x < r.0 + r.2 && y >= r.1 && y < r.1 + r.3
}

fn deepest_at(t: &Tree, k: WinKey, from: u32, p: Point<Pixels>) -> u32 {
    let mut cur = from;
    'down: for _ in 0..4096 {
        let Some(n) = t.node(cur) else { break };
        for &c in n.children.iter().rev() {
            let Some(kid) = t.node(c) else { continue };
            if kid.kind != Kind::Element || skipped(t, kid) {
                continue;
            }
            if windows::peek(k, |w| w.bounds.get(&c).copied())
                .flatten()
                .is_some_and(|r| contains(r, p))
            {
                cur = c;
                continue 'down;
            }
        }
        break;
    }
    cur
}

enum Hit {
    At(Pos),
    Wait(Vec<u32>),
    Nothing,
}

fn distance(b: Bounds<Pixels>, p: Point<Pixels>) -> (f32, f32) {
    let gap = |v: Pixels, lo: Pixels, hi: Pixels| {
        if v < lo {
            f32::from(lo - v)
        } else if v > hi {
            f32::from(v - hi)
        } else {
            0.0
        }
    };
    (gap(p.y, b.top(), b.bottom()), gap(p.x, b.left(), b.right()))
}

fn offset_in(t: &Tree, holder: u32, rec: &Recorded, p: Point<Pixels>) -> u32 {
    let layout = &rec.layout;
    let b = layout.bounds();
    let byte = if p.y < b.top() {
        0
    } else if p.y >= b.bottom() {
        layout.len()
    } else {
        match layout.index_for_position(p) {
            Ok(i) => {
                let text = layout.text();
                let next = text[i..].chars().next().map_or(i, |c| i + c.len_utf8());
                match (
                    layout.position_for_index(i),
                    layout.position_for_index(next),
                ) {
                    (Some(a), Some(b))
                        if a.y == b.y
                            && f32::from(p.x) > (f32::from(a.x) + f32::from(b.x)) / 2.0 =>
                    {
                        next
                    }
                    _ => i,
                }
            }
            Err(i) => i,
        }
    };
    let Some(n) = t.node(holder) else { return 0 };
    TextMap::new(&n.text, rec.ws, rec.lead, rec.len, rec.case).to_dom(byte)
}

fn is_holder(t: &Tree, id: u32, n: &Node) -> bool {
    !n.text.is_empty()
        && match n.kind {
            Kind::Text => true,
            Kind::Element => !skipped(t, n) && !matches!(t.name(n.tag), "button"),
            Kind::Anchor => false,
        }
        && id != 0
}

fn hit(window: &Window, p: Point<Pixels>) -> Hit {
    let k = windows::key(window);
    let top = topmost(window, p);
    STATE.with(|s| {
        let s = s.borrow();
        tree::read_tree(|t| {
            let root = match windows::root_of(k) {
                0 => t.root,
                popped => popped,
            };
            if t.node(root).is_none() {
                return Hit::Nothing;
            }
            let mut scope = deepest_at(t, k, top.unwrap_or(root), p);
            for _ in 0..4 {
                let mut best: Option<((f32, f32), u32)> = None;
                let mut missing = Vec::new();
                let mut stack = vec![scope];
                let mut seen = 0;
                while let Some(id) = stack.pop() {
                    seen += 1;
                    if seen > 4000 {
                        break;
                    }
                    let Some(n) = t.node(id) else { continue };
                    if n.kind == Kind::Element && skipped(t, n) {
                        continue;
                    }
                    if is_holder(t, id, n) {
                        match s.layouts.get(&id).filter(|r| r.win == k) {
                            // Clipped out of sight: a pointer cannot be over it.
                            Some(rec) if rec.visible.is_empty() => {}
                            Some(rec) => {
                                let d = distance(rec.visible, p);
                                if best.is_none_or(|(bd, _)| d < bd) {
                                    best = Some((d, id));
                                }
                            }
                            None => missing.push(id),
                        }
                        continue;
                    }
                    stack.extend(n.children.iter().rev().copied());
                }
                let exact = best.is_some_and(|(d, _)| d == (0.0, 0.0));
                if !exact && !missing.is_empty() {
                    missing.truncate(256);
                    return Hit::Wait(missing);
                }
                if let Some((_, id)) = best {
                    let rec = &s.layouts[&id];
                    let own = t.node(id).is_some_and(|n| n.kind == Kind::Element);
                    let offset = offset_in(t, id, rec, p);
                    return Hit::At(Pos {
                        node: id,
                        offset,
                        own,
                    });
                }
                match t.node(scope).map(|n| n.parent) {
                    Some(parent) if parent != 0 && scope != root => scope = parent,
                    _ => break,
                }
            }
            Hit::At(Pos::at(scope, 0))
        })
    })
}

fn word_at(t: &Tree, p: Pos) -> (Pos, Pos) {
    let Some(n) = t.node(p.node) else {
        return (p, p);
    };
    if n.kind == Kind::Element && !p.own {
        return (p, p);
    }
    let units: Vec<(u32, char)> = n
        .text
        .chars()
        .scan(0u32, |at, c| {
            let here = *at;
            *at += c.len_utf16() as u32;
            Some((here, c))
        })
        .collect();
    if units.is_empty() {
        return (p, p);
    }
    let class = |c: char| {
        if c.is_alphanumeric() || c == '_' {
            0
        } else if c.is_whitespace() {
            1
        } else {
            2
        }
    };
    let mut i = units
        .partition_point(|&(at, _)| at < p.offset)
        .min(units.len() - 1);
    if units[i].0 > p.offset && i > 0 {
        i -= 1;
    }
    let c = class(units[i].1);
    let (mut a, mut b) = (i, i);
    if c != 2 {
        while a > 0 && class(units[a - 1].1) == c {
            a -= 1;
        }
        while b + 1 < units.len() && class(units[b + 1].1) == c {
            b += 1;
        }
    }
    let end = units[b].0 + units[b].1.len_utf16() as u32;
    (
        Pos {
            offset: units[a].0,
            ..p
        },
        Pos { offset: end, ..p },
    )
}

fn block_at(t: &Tree, p: Pos) -> (Pos, Pos) {
    let mut cur = match t.node(p.node) {
        Some(n) if n.kind == Kind::Text => n.parent,
        Some(_) => p.node,
        None => return (p, p),
    };
    for _ in 0..4096 {
        let Some(n) = t.node(cur) else { break };
        if is_block(t, n) || n.parent == 0 {
            return if n.text.is_empty() {
                (Pos::at(cur, 0), Pos::at(cur, n.children.len() as u32))
            } else {
                (Pos::own(cur, 0), Pos::own(cur, utf16_len(&n.text)))
            };
        }
        cur = n.parent;
    }
    (p, p)
}

fn unit(t: &Tree, p: Pos, grain: Grain) -> (Pos, Pos) {
    match grain {
        Grain::Char => (p, p),
        Grain::Word => word_at(t, p),
        Grain::Block => block_at(t, p),
    }
}

/// `user-select: all`: a boundary inside such an element moves to its edge, away from the other boundary.
fn take_all(t: &Tree, a: Pos, f: Pos) -> (Pos, Pos) {
    let forward = matches!((key(t, a), key(t, f)), (Some(ka), Some(kf)) if ka <= kf);
    let outer = |p: Pos, toward_end: bool| -> Pos {
        let mut found = None;
        let mut cur = p.node;
        for _ in 0..4096 {
            let Some(n) = t.node(cur) else { break };
            if n.kind == Kind::Element && used_at(t, cur) == Us::All {
                found = Some(cur);
            }
            if n.parent == 0 {
                break;
            }
            cur = n.parent;
        }
        let Some(el) = found else { return p };
        let Some(n) = t.node(el) else { return p };
        let Some(parent) = t.node(n.parent) else {
            return p;
        };
        let i = parent.children.iter().position(|&c| c == el).unwrap_or(0) as u32;
        Pos::at(n.parent, if toward_end { i + 1 } else { i })
    };
    (outer(a, !forward), outer(f, forward))
}

fn shape(t: &Tree, anchor: (Pos, Pos), focus: Pos, grain: Grain) -> (Pos, Pos) {
    let (a, f) = if grain == Grain::Char {
        (anchor.0, focus)
    } else {
        let fu = unit(t, focus, grain);
        let before = matches!((key(t, focus), key(t, anchor.0)), (Some(kf), Some(ka)) if kf < ka);
        if before {
            (anchor.1, fu.0)
        } else {
            (anchor.0, fu.1)
        }
    };
    take_all(t, a, f)
}

/// The root's share of every press: decides after the JS `mousedown` whether it starts a selection.
pub(crate) fn bind_root<E: InteractiveElement>(e: E) -> E {
    e.capture_any_mouse_down(|_, _, _| PRESS_PREVENTED.with(|c| c.set(false)))
        .on_any_mouse_down(|ev, window, cx| press(ev, window, cx))
        .capture_any_mouse_up(on_release)
        .on_mouse_up_out(MouseButton::Left, on_release)
        .on_key_down(|ev, window, cx| {
            let k = &ev.keystroke;
            let m = &k.modifiers;
            let primary = if cfg!(target_os = "macos") {
                m.platform
            } else {
                m.control
            };
            if k.key != "a" || !primary || m.shift || m.alt || m.function {
                return;
            }
            if windows::here(window, |w| w.key_prevented) {
                return;
            }
            let focused = windows::focused_node(windows::key(window));
            if focused != 0 && tree::read_tree(|t| editable_at(t, focused)) {
                return;
            }
            select_all(window, cx);
        })
}

pub(crate) fn note_press_prevented() {
    PRESS_PREVENTED.with(|c| c.set(true));
}

fn press(ev: &MouseDownEvent, window: &mut Window, cx: &mut App) {
    if ev.button != MouseButton::Left || PRESS_PREVENTED.with(|c| c.replace(false)) {
        return;
    }
    // A gesture whose release never reached us (it went to another window) ends here; what it
    // selected stays.
    if STATE.with(|s| s.borrow().gesture.is_some()) {
        finish();
    }
    let k = windows::key(window);
    let p = ev.position;
    let has_sel = STATE.with(|s| s.borrow().sel.is_some());
    let top = topmost(window, p);
    let (us, editable, button, target) = tree::read_tree(|t| {
        let root = match windows::root_of(k) {
            0 => t.root,
            popped => popped,
        };
        let target = deepest_at(t, k, top.unwrap_or(root), p);
        (
            used_at(t, target),
            editable_at(t, target),
            up(t, target, |t, n| t.name(n.tag) == "button"),
            target,
        )
    });
    if editable {
        if has_sel {
            replace_live(None, window, cx);
        }
        return;
    }
    // `user-select: none` leaves the selection alone, so it survives a click on a "Copy" button.
    if us == Us::None {
        return;
    }
    let extend = ev.modifiers.shift && has_sel;
    if !extend && has_sel {
        replace_live(None, window, cx);
    }
    if button {
        return;
    }
    let listening = tree::read_tree(|t| listened(t, target, EV_SELECTSTART));
    if listening
        && !crate::render::js_dispatch_app(
            target,
            EV_SELECTSTART,
            &crate::EventPayload::default(),
            window,
            cx,
        )
    {
        return;
    }
    let grain = match ev.click_count {
        0 | 1 => Grain::Char,
        2 => Grain::Word,
        _ => Grain::Block,
    };
    let anchor = if extend {
        STATE.with(|s| s.borrow().sel.map(|(a, _)| (a, a)))
    } else {
        None
    };
    STATE.with(|s| {
        let mut s = s.borrow_mut();
        s.gesture = Some(Gesture {
            win: k,
            press: p,
            last: p,
            grain,
            extend,
            dragging: false,
            released: false,
            anchor,
            waits: 0,
        });
        set_work(&s);
    });
    RECORDING.with(|c| c.set(true));
    if grain != Grain::Char || extend {
        advance(window, cx);
    }
    crate::render::request_repaint(window, cx);
}

/// The root's paint while a gesture lasts: every move, since the window has the mouse captured.
pub(crate) fn paint_listeners(window: &mut Window) {
    let k = windows::key(window);
    if !WORK.with(|c| c.get())
        || STATE.with(|s| s.borrow().gesture.as_ref().is_none_or(|g| g.win != k))
    {
        return;
    }
    window.on_mouse_event(move |ev: &MouseMoveEvent, phase, window, cx| {
        if phase == DispatchPhase::Capture {
            moved(ev.position, ev.pressed_button, window, cx);
        }
    });
}

fn on_release(ev: &MouseUpEvent, window: &mut Window, cx: &mut App) {
    if ev.button == MouseButton::Left && WORK.with(|c| c.get()) {
        released(ev.position, window, cx);
    }
}

/// How far the pointer moves before a press is a drag (CSS px).
const DRAG_SLOP: f32 = 3.0;

fn moved(p: Point<Pixels>, pressed: Option<MouseButton>, window: &mut Window, cx: &mut App) {
    let k = windows::key(window);
    let go = STATE.with(|s| {
        let mut s = s.borrow_mut();
        let Some(g) = s.gesture.as_mut().filter(|g| g.win == k && !g.released) else {
            return None;
        };
        if pressed != Some(MouseButton::Left) {
            return Some(None);
        }
        if g.last == p {
            return Some(Some(false));
        }
        g.last = p;
        if !g.dragging {
            let (dx, dy) = (f32::from(p.x - g.press.x), f32::from(p.y - g.press.y));
            g.dragging = dx.hypot(dy) >= DRAG_SLOP;
        }
        Some(Some(g.dragging))
    });
    match go {
        None | Some(Some(false)) => {}
        Some(None) => {
            let last = STATE.with(|s| s.borrow().gesture.as_ref().map(|g| g.last));
            if let Some(last) = last {
                released(last, window, cx);
            }
        }
        Some(Some(true)) => advance(window, cx),
    }
}

fn released(p: Point<Pixels>, window: &mut Window, cx: &mut App) {
    let k = windows::key(window);
    let live = STATE.with(|s| {
        let mut s = s.borrow_mut();
        match s.gesture.as_mut().filter(|g| g.win == k) {
            Some(g) => {
                if g.dragging {
                    g.last = p;
                }
                g.released = true;
                true
            }
            None => false,
        }
    });
    if live {
        advance(window, cx);
    }
}

fn advance(window: &mut Window, cx: &mut App) {
    let k = windows::key(window);
    let Some((press, last, grain, dragging, released, anchor, extend, waits)) = STATE.with(|s| {
        let s = s.borrow();
        let g = s.gesture.as_ref().filter(|g| g.win == k)?;
        Some((
            g.press, g.last, g.grain, g.dragging, g.released, g.anchor, g.extend, g.waits,
        ))
    }) else {
        return;
    };
    let wanted = dragging || grain != Grain::Char || extend;
    if !wanted {
        if released {
            finish();
        }
        return;
    }
    // Text that never records (a node no frame paints) must not hold the gesture forever: past a
    // few frames, what is recorded is the answer.
    let patient = waits < 4;
    let resolve = |window: &Window, p: Point<Pixels>| -> Result<Option<Pos>, Vec<u32>> {
        match hit(window, p) {
            Hit::At(pos) => Ok(Some(pos)),
            Hit::Wait(ids) if patient => Err(ids),
            Hit::Wait(_) | Hit::Nothing => Ok(None),
        }
    };
    let anchor = match anchor {
        Some(a) => a,
        None => match resolve(window, press) {
            Ok(Some(pos)) => {
                let u = tree::read_tree(|t| unit(t, pos, grain));
                STATE.with(|s| {
                    if let Some(g) = s.borrow_mut().gesture.as_mut() {
                        g.anchor = Some(u);
                    }
                });
                u
            }
            Ok(None) => {
                finish();
                return;
            }
            Err(ids) => return wait(ids, window, cx),
        },
    };
    let focus = if dragging || extend {
        match resolve(window, last) {
            Ok(Some(pos)) => pos,
            Ok(None) => {
                finish();
                return;
            }
            Err(ids) => return wait(ids, window, cx),
        }
    } else {
        anchor.1
    };
    let sel = tree::read_tree(|t| {
        if !dragging && !extend {
            take_all(t, anchor.0, anchor.1)
        } else {
            shape(t, anchor, focus, grain)
        }
    });
    replace_live(Some(sel), window, cx);
    if released {
        finish();
    }
}

fn wait(ids: Vec<u32>, window: &mut Window, cx: &mut App) {
    STATE.with(|s| {
        if let Some(g) = s.borrow_mut().gesture.as_mut() {
            g.waits = g.waits.saturating_add(1);
        }
    });
    tree::with_tree(|t| {
        for id in ids {
            t.touch(id);
        }
    });
    crate::render::request_repaint(window, cx);
}

fn finish() {
    STATE.with(|s| {
        let mut s = s.borrow_mut();
        s.gesture = None;
        s.layouts = HashMap::new();
        set_work(&s);
    });
    RECORDING.with(|c| c.set(false));
}

pub(crate) fn tick(window: &mut Window, cx: &mut App) -> bool {
    if !WORK.with(|c| c.get()) {
        return false;
    }
    let k = windows::key(window);
    let mut moved = false;
    let gesture = STATE.with(|s| {
        s.borrow()
            .gesture
            .as_ref()
            .map(|g| (g.win, g.dragging, g.released, g.last))
    });
    if let Some((win, dragging, released, last)) = gesture.filter(|g| g.0 == k) {
        let _ = win;
        if dragging && !released {
            moved = autoscroll(k, last);
        }
        advance(window, cx);
    }
    let owed = STATE.with(|s| {
        let mut s = s.borrow_mut();
        let owed = std::mem::take(&mut s.change_owed);
        set_work(&s);
        owed
    });
    if owed {
        let root = tree::read_tree(|t| t.root);
        let listening = tree::read_tree(|t| {
            t.node(root)
                .is_some_and(|n| n.listeners & (1u64 << EV_SELECTIONCHANGE) != 0)
        });
        if listening {
            crate::render::js_dispatch_app(
                root,
                EV_SELECTIONCHANGE,
                &crate::EventPayload::default(),
                window,
                cx,
            );
            moved = true;
        }
    }
    moved
}

fn autoscroll(k: WinKey, last: Point<Pixels>) -> bool {
    let anchor = STATE.with(|s| {
        s.borrow()
            .gesture
            .as_ref()
            .and_then(|g| g.anchor)
            .map(|a| a.0.node)
    });
    let Some(node) = anchor else { return false };
    let scroller = tree::read_tree(|t| {
        let mut cur = node;
        for _ in 0..4096 {
            let n = t.node(cur)?;
            if n.kind == Kind::Element && crate::render::scroll_handle_of(cur).is_some() {
                return Some(cur);
            }
            if n.parent == 0 {
                return None;
            }
            cur = n.parent;
        }
        None
    });
    let Some(id) = scroller else { return false };
    let Some(r) = windows::peek(k, |w| w.bounds.get(&id).copied()).flatten() else {
        return false;
    };
    let y = f32::from(last.y);
    let (top, bottom) = (r.1, r.1 + r.3);
    let dy = if y < top {
        -((top - y) / 2.0 + 2.0).min(40.0)
    } else if y > bottom {
        ((y - bottom) / 2.0 + 2.0).min(40.0)
    } else {
        return false;
    };
    let before = crate::scroll::info(id).map(|i| i.top);
    let after =
        crate::scroll::by_free(id, 0.0, dy, crate::scroll::Behavior::Instant).map(|i| i.top);
    before != after
}

use sk_runtime::redjs::api::Value;
use sk_runtime::{host_fn, JsRuntime};

const UNDEF: u64 = 0xa;

fn num(v: Value) -> u32 {
    v.as_number().unwrap_or(0.0).max(0.0) as u32
}

fn ret_string(s: &str) -> u64 {
    JsRuntime::with(|rt| rt.ctx().string(s).0)
}

fn pos_args(args: &sk_runtime::redjs::api::HostArgs, at: u32) -> Pos {
    Pos::at(num(args.arg(at)), num(args.arg(at + 1)))
}

host_fn!(fn sk_selection_get(_g, _args) {
    match current() {
        Some((a, f)) => ret_string(&format!(
            "{},{},{},{},{},{}",
            a.node, a.offset, u8::from(a.own), f.node, f.offset, u8::from(f.own)
        )),
        None => ret_string(""),
    }
});

host_fn!(fn sk_selection_set(_g, args) {
    let a = pos_args(&args, 0);
    let points = (a.node != 0).then(|| (a, pos_args(&args, 2)));
    Value::boolean(set_from_js(points)).0
});

host_fn!(fn sk_selection_text(_g, _args) {
    ret_string(&selected_text())
});

host_fn!(fn sk_selection_rebase(_g, args) {
    rebase(num(args.arg(0)), num(args.arg(1)));
    UNDEF
});

pub(crate) fn install(rt: &JsRuntime) {
    let fns: &[(&str, u32, *const std::ffi::c_void)] = &[
        ("__sk_selection_get", 0, sk_selection_get as *const _),
        ("__sk_selection_set", 4, sk_selection_set as *const _),
        ("__sk_selection_text", 0, sk_selection_text as *const _),
        ("__sk_selection_rebase", 2, sk_selection_rebase as *const _),
    ];
    for (name, arity, f) in fns {
        rt.register(name, *arity, *f);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(raw: &str, ws: WhiteSpaceMode) -> String {
        collapse(raw, ws).into_iter().map(|x| x.1).collect()
    }

    /// `collapse` is `render::processed_text` with offsets: the two must produce the same string
    /// for every mode, or a highlight lands on the wrong glyphs.
    #[test]
    fn collapse_matches_the_renderer() {
        let modes = [
            WhiteSpaceMode::Normal,
            WhiteSpaceMode::Nowrap,
            WhiteSpaceMode::Pre,
            WhiteSpaceMode::PreWrap,
            WhiteSpaceMode::PreLine,
            WhiteSpaceMode::BreakSpaces,
        ];
        for raw in [
            "plain",
            "  lead and trail  ",
            "a\n\n  b\t\tc",
            "x \n y",
            "émoji 😀  two",
            "",
            "  ",
            "\n",
        ] {
            for mode in modes {
                assert_eq!(
                    text(raw, mode),
                    crate::render::processed_text(raw, mode).as_ref(),
                    "{raw:?} {mode:?}"
                );
            }
        }
    }

    #[test]
    fn a_map_walks_both_ways_through_collapsed_space_and_surrogates() {
        let raw = "  a  😀b ";
        let processed = crate::render::processed_text(raw, WhiteSpaceMode::Normal);
        assert_eq!(processed, " a 😀b ");
        let map = TextMap::new(raw, None, 1, "a 😀b".len(), None);
        assert_eq!(map.display, "a 😀b");
        // DOM: a=2, spaces 3..5, 😀=5..7 (two units), b=7, space=8.
        assert_eq!(map.to_dom(0), 2);
        assert_eq!(map.to_dom(1), 3);
        assert_eq!(map.to_dom(2), 5);
        assert_eq!(map.to_dom(6), 7);
        assert_eq!(map.to_dom(map.display.len()), 8);
        assert_eq!(map.to_display(0), 0, "trimmed lead space -> first glyph");
        assert_eq!(
            map.to_display(4),
            2,
            "inside a collapsed run -> the glyph after it"
        );
        assert_eq!(map.to_display(7), 6);
        assert_eq!(map.to_display(9), map.display.len());
        assert_eq!(map.slice(2, 7), "a 😀");
    }

    #[test]
    fn a_transform_maps_and_a_length_changing_one_stays_monotonic() {
        let map = TextMap::new("abc", None, 0, 3, Some(TextTransform::Uppercase));
        assert_eq!(map.display, "ABC");
        assert_eq!(map.slice(1, 3), "BC");
        let map = TextMap::new(
            "straße",
            None,
            0,
            "straße".len(),
            Some(TextTransform::Uppercase),
        );
        assert_eq!(map.display, "STRASSE");
        let dom: Vec<u32> = (0..=map.display.len()).map(|b| map.to_dom(b)).collect();
        assert!(dom.windows(2).all(|w| w[0] <= w[1]), "{dom:?}");
        assert_eq!(map.to_dom(map.display.len()), 6);
    }

    #[test]
    fn clip_takes_the_part_inside_the_range() {
        let p = [1, 2];
        assert_eq!(
            clip(&p, &[1, 2, 3], &[1, 2, 7], 10),
            (3, 7),
            "both ends inside"
        );
        assert_eq!(clip(&p, &[0], &[5], 10), (0, 10), "around it");
        assert_eq!(
            clip(&p, &[1, 2], &[1, 3], 10),
            (0, 10),
            "(parent, i) .. (parent, i+1)"
        );
        assert_eq!(clip(&p, &[1, 3], &[2], 10), (10, 10), "after it");
        assert_eq!(clip(&p, &[0], &[1, 2], 10), (0, 0), "before it");
        assert_eq!(clip(&p, &[1, 2, 4], &[3], 10), (4, 10));
    }

    fn doc() -> (u32, u32, u32, u32, u32, u32) {
        tree::with_tree(|t| {
            let root = t.create_root();
            let el = |t: &mut Tree, tag: &str, parent: u32| {
                let tag = t.intern(tag);
                let id = t.create_element(tag);
                t.insert(id, parent, 0);
                id
            };
            let txt = |t: &mut Tree, s: &str, parent: u32| {
                let id = t.create_text();
                t.set_text(id, s.to_string());
                t.insert(id, parent, 0);
                id
            };
            let div = el(t, "div", root);
            let p1 = el(t, "p", div);
            let hello = txt(t, "Hello ", p1);
            let b = el(t, "b", p1);
            t.set_element_text(b, "big".into());
            let world = txt(t, " world", p1);
            let p2 = el(t, "p", div);
            t.set_element_text(p2, "Second".into());
            let no = el(t, "span", div);
            t.set_element_text(no, "skip".into());
            let mut cs = sk_css::ComputedStates::default();
            cs.extras.user_select = Some(UserSelect::None);
            t.node_mut(no).unwrap().computed = Some(cs);
            (root, div, hello, b, world, p2)
        })
    }

    #[test]
    fn the_text_of_a_range_breaks_between_paragraphs_and_skips_select_none() {
        let (_, div, hello, _b, world, p2) = doc();
        let all = tree::read_tree(|t| {
            let n = t.node(div).unwrap().children.len() as u32;
            text_between(
                t,
                &key(t, Pos::at(div, 0)).unwrap(),
                &key(t, Pos::at(div, n)).unwrap(),
            )
        });
        assert_eq!(all, "Hello big world\n\nSecond");
        let part = tree::read_tree(|t| {
            text_between(
                t,
                &key(t, Pos::at(hello, 2)).unwrap(),
                &key(t, Pos::own(p2, 3)).unwrap(),
            )
        });
        assert_eq!(part, "llo big world\n\nSec");
        let inside = tree::read_tree(|t| {
            text_between(
                t,
                &key(t, Pos::at(world, 1)).unwrap(),
                &key(t, Pos::at(world, 4)).unwrap(),
            )
        });
        assert_eq!(inside, "wor");
    }

    #[test]
    fn the_painted_parts_follow_the_selection_and_its_changes_touch_only_what_moved() {
        let (_, div, hello, b, world, p2) = doc();
        let sel = Some((Pos::at(hello, 3), Pos::own(p2, 2)));
        let painted = tree::read_tree(|t| painted_of(t, sel));
        assert_eq!(painted.get(&hello), Some(&(3, 6)));
        assert_eq!(painted.get(&b), Some(&(0, 3)));
        assert_eq!(painted.get(&world), Some(&(0, 6)));
        assert_eq!(painted.get(&p2), Some(&(0, 2)));
        assert_eq!(painted.len(), 4, "user-select: none paints nothing");
        assert_eq!(
            tree::read_tree(|t| painted_of(t, Some((Pos::own(p2, 2), Pos::at(hello, 3))))),
            painted
        );

        tree::with_tree(|t| replace(t, sel));
        let revs = |ids: &[u32]| {
            tree::read_tree(|t| ids.iter().map(|&i| t.sub_rev(i)).collect::<Vec<_>>())
        };
        let before = revs(&[hello, b, world, p2]);
        tree::with_tree(|t| replace(t, Some((Pos::at(hello, 3), Pos::own(p2, 4)))));
        let after = revs(&[hello, b, world, p2]);
        assert_eq!(before[1], after[1], "<b> kept its part: not touched");
        assert_ne!(
            before[3], after[3],
            "the second paragraph's part grew: touched"
        );
        assert!(PAINTING.with(|c| c.get()));
        tree::with_tree(|t| replace(t, None));
        assert!(!PAINTING.with(|c| c.get()));
        let _ = div;
        STATE.with(|s| *s.borrow_mut() = State::default());
    }

    #[test]
    fn a_double_click_takes_a_word_and_a_triple_click_the_paragraph() {
        let (_, _, hello, _, world, _) = doc();
        tree::read_tree(|t| {
            assert_eq!(
                word_at(t, Pos::at(world, 3)),
                (Pos::at(world, 1), Pos::at(world, 6))
            );
            assert_eq!(
                word_at(t, Pos::at(world, 0)),
                (Pos::at(world, 0), Pos::at(world, 1)),
                "the space"
            );
            assert_eq!(
                word_at(t, Pos::at(hello, 6)),
                (Pos::at(hello, 5), Pos::at(hello, 6)),
                "at the end"
            );
            let p1 = t.node(hello).unwrap().parent;
            assert_eq!(
                block_at(t, Pos::at(world, 2)),
                (Pos::at(p1, 0), Pos::at(p1, 3))
            );
        });
    }

    #[test]
    fn a_removed_boundary_clears_and_a_shortened_text_clamps() {
        let (_, _, hello, _, world, _) = doc();
        tree::with_tree(|t| replace(t, Some((Pos::at(hello, 1), Pos::at(world, 5)))));
        tree::with_tree(|t| {
            t.set_text(world, "ab".into());
            gc(t);
        });
        assert_eq!(current(), Some((Pos::at(hello, 1), Pos::at(world, 2))));
        tree::with_tree(|t| {
            t.remove(world);
            gc(t);
        });
        assert_eq!(current(), None);
        assert!(!PAINTING.with(|c| c.get()));
        STATE.with(|s| *s.borrow_mut() = State::default());
    }

    #[test]
    fn user_select_all_takes_the_element_whole() {
        let (_, div, hello, b, world, _) = doc();
        tree::with_tree(|t| {
            let mut cs = sk_css::ComputedStates::default();
            cs.extras.user_select = Some(UserSelect::All);
            t.node_mut(b).unwrap().computed = Some(cs);
        });
        let p1 = tree::read_tree(|t| t.node(hello).unwrap().parent);
        let (a, f) = tree::read_tree(|t| take_all(t, Pos::own(b, 1), Pos::at(world, 3)));
        assert_eq!((a, f), (Pos::at(p1, 1), Pos::at(world, 3)));
        let (a, f) = tree::read_tree(|t| take_all(t, Pos::at(world, 3), Pos::own(b, 1)));
        assert_eq!((a, f), (Pos::at(world, 3), Pos::at(p1, 1)));
        let _ = div;
    }
}
