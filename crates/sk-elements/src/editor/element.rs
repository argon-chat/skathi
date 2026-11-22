//! The gpui element: lays each hard line out on its own (marks, decorations and line blocks become
//! text runs, atoms become inline boxes), places the atoms' elements on their U+FFFC, and paints
//! line boxes, selection, text and caret.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use gpui::{
    fill, point, px, size, App, AvailableSpace, Bounds, ContentMask, DispatchPhase, Edges, Element,
    ElementId, ElementInputHandler, Entity, Font, GlobalElementId, Hsla, InlineBox,
    InspectorElementId, LayoutId, MouseButton, MouseMoveEvent, MouseUpEvent, PaintQuad, Pixels,
    SharedString, Size, Style, TextAlign, TextRun, TextStyle, UnderlineStyle, Window,
};
use sk_editor::ATOM_CHAR;

use super::geometry::LineGeom;
use super::state::EditorState;
use super::{AtomChild, EditorOptions, LineStyle, RunStyle};
use crate::vertical_align::{top_above_baseline, Alignment, LineText, VerticalAlign};

const CARET_WIDTH: Pixels = px(2.0);

pub(crate) fn line_height_for(window: &Window, font_size: Pixels) -> Pixels {
    window
        .text_style()
        .line_height
        .to_pixels(font_size.into(), window.rem_size())
        .round()
        .max(font_size)
}

/// An atom aligned to its row's top or bottom (`vertical-align: top | bottom`), which needs the
/// row's height and so a second shaping.
#[derive(Clone, Copy, PartialEq)]
struct RowAtom {
    run: usize,
    byte: usize,
    width: Pixels,
    height: Pixels,
    top: bool,
}

/// One hard line ready to shape.
#[derive(Clone, PartialEq)]
struct LinePlan {
    start: usize,
    text: SharedString,
    runs: Vec<TextRun>,
    font_size: Pixels,
    line_height: Pixels,
    inset: Edges<Pixels>,
    style: Option<LineStyle>,
    row_atoms: Vec<RowAtom>,
}

/// The byte index each visual row of `wrapped` starts at.
fn row_starts(wrapped: &gpui::WrappedLine) -> Vec<usize> {
    let runs = wrapped.runs();
    let mut out = vec![0];
    for b in wrapped.wrap_boundaries() {
        if let Some(g) = runs.get(b.run_ix).and_then(|r| r.glyphs.get(b.glyph_ix)) {
            out.push(g.index);
        }
    }
    out
}

impl LinePlan {
    fn shape(
        &self,
        runs: &[TextRun],
        width: Pixels,
        window: &mut Window,
    ) -> Option<gpui::WrappedLine> {
        let wrap = (width - self.inset.left - self.inset.right - CARET_WIDTH).max(px(1.0));
        window
            .text_system()
            .shape_text(self.text.clone(), self.font_size, runs, Some(wrap), None)
            .ok()?
            .into_iter()
            .next()
    }

    /// The wrapped line and its rows. Top- and bottom-aligned atoms first hold their row's
    /// strut; once the rows are known, each reserves the extent CSS gives its row: the bottom
    /// ones push the baseline down, the top ones the row's bottom.
    fn layout(
        &self,
        width: Pixels,
        window: &mut Window,
    ) -> Option<(gpui::WrappedLine, Vec<gpui::LineBox>)> {
        let wrapped = self.shape(&self.runs, width, window)?;
        let rows: Vec<gpui::LineBox> = wrapped.line_boxes(self.line_height).into_iter().collect();
        if self.row_atoms.is_empty() || rows.is_empty() {
            return Some((wrapped, rows));
        }
        let starts = row_starts(&wrapped);
        let row_of = |byte: usize| {
            (starts.partition_point(|&s| s <= byte).saturating_sub(1)).min(rows.len() - 1)
        };
        let mut tallest = vec![(px(0.0), px(0.0)); rows.len()];
        for a in &self.row_atoms {
            let t = &mut tallest[row_of(a.byte)];
            if a.top {
                t.0 = t.0.max(a.height);
            } else {
                t.1 = t.1.max(a.height);
            }
        }
        let extents: Vec<(Pixels, Pixels)> = rows
            .iter()
            .zip(&tallest)
            .map(|(row, &(top, bottom))| {
                let (above, below) = (row.baseline, row.height - row.baseline);
                let above = above.max(bottom - below);
                (above, below.max(top - above))
            })
            .collect();
        if rows
            .iter()
            .zip(&extents)
            .all(|(row, &(above, below))| row.height >= above + below)
        {
            return Some((wrapped, rows));
        }
        let mut runs = self.runs.clone();
        for a in &self.row_atoms {
            let (ascent, descent) = extents[row_of(a.byte)];
            runs[a.run].inline_box = Some(InlineBox {
                width: a.width,
                ascent,
                descent,
            });
        }
        let wrapped = self.shape(&runs, width, window)?;
        let rows = wrapped.line_boxes(self.line_height).into_iter().collect();
        Some((wrapped, rows))
    }
}

/// Where an atom's box goes in its row.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Place {
    /// Its top this far above the baseline.
    Above(Pixels),
    Top,
    Bottom,
}

/// The room an atom of `size` reserves in its line, and where its box goes.
fn atom_box(size: Size<Pixels>, align: VerticalAlign, line: &LineText) -> (InlineBox, Place) {
    let h = size.height;
    let place = match align.alignment(line) {
        Alignment::Baseline(shift) => Place::Above(top_above_baseline(shift, h, h)),
        Alignment::Top => Place::Top,
        Alignment::Bottom => Place::Bottom,
    };
    let (above, below) = line.strut();
    let (ascent, descent) = match place {
        Place::Above(a) => (a.max(above), (h - a).max(below)),
        Place::Top | Place::Bottom => (above, below),
    };
    (
        InlineBox {
            width: size.width,
            ascent,
            descent,
        },
        place,
    )
}

pub(crate) struct EditorElement {
    state: Entity<EditorState>,
    options: EditorOptions,
    atoms: Vec<AtomChild>,
}

impl EditorElement {
    pub(crate) fn new(
        state: Entity<EditorState>,
        options: EditorOptions,
        atoms: Vec<AtomChild>,
    ) -> Self {
        Self {
            state,
            options,
            atoms,
        }
    }
}

pub(crate) struct Layout {
    plans: Rc<Vec<LinePlan>>,
    /// Atom id → (index into `atoms`, measured size).
    atom_sizes: HashMap<String, (usize, Size<Pixels>)>,
    places: HashMap<String, Place>,
    placeholder: bool,
}

pub(crate) struct Prepainted {
    selection: Vec<PaintQuad>,
    caret: Option<PaintQuad>,
    boxes: Vec<PaintQuad>,
    placed: Vec<usize>,
    origin: gpui::Point<Pixels>,
}

fn base_font(text_style: &TextStyle) -> Font {
    text_style.font()
}

fn run_for(
    len: usize,
    style: &RunStyle,
    base: &Font,
    text_style: &TextStyle,
    color: Hsla,
    line_size: Pixels,
) -> TextRun {
    let mut font = base.clone();
    if let Some(family) = &style.font_family {
        font.family = family.clone();
    }
    if let Some(weight) = style.font_weight {
        font.weight = weight;
    }
    if let Some(s) = style.font_style {
        font.style = s;
    }
    TextRun {
        len,
        font,
        color: style.color.unwrap_or(color),
        background_color: style.background,
        underline: style.underline,
        strikethrough: style.strikethrough,
        tracking: text_style.letter_spacing,
        word_spacing: text_style.word_spacing,
        text_shadow: text_style.text_shadow.clone(),
        font_size: style.font_size.filter(|s| *s != line_size),
        inline_box: None,
    }
}

/// UTF-8 offset in `text` of a UTF-16 offset.
fn to8(text: &str, pos: usize) -> usize {
    let mut units = 0;
    for (at, c) in text.char_indices() {
        if units >= pos {
            return at;
        }
        units += c.len_utf16();
    }
    text.len()
}

impl EditorElement {
    fn plans(
        &self,
        window: &Window,
        cx: &App,
        atom_sizes: &HashMap<String, (usize, Size<Pixels>)>,
        places: &mut HashMap<String, Place>,
    ) -> (Vec<LinePlan>, bool) {
        let text_style = window.text_style();
        let base = base_font(&text_style);
        let base_size = self.options.font_size;
        let base_lh = line_height_for(window, base_size);
        let state = self.state.read(cx);
        let marked = state.marked_range();
        let shared = state.shared();
        let core = shared.core.borrow();
        let doc = core.doc();
        let opts = &self.options;
        if doc.is_empty() && !opts.placeholder.is_empty() {
            let text: SharedString = opts.placeholder.replace('\n', " ").into();
            let run = run_for(
                text.len(),
                &RunStyle::default(),
                &base,
                &text_style,
                opts.placeholder_color,
                base_size,
            );
            let style = doc.block(0).and_then(|b| opts.lines.get(&b.kind)).cloned();
            let (font_size, line_height, inset) = line_box(style.as_ref(), base_size, base_lh);
            return (
                vec![LinePlan {
                    start: 0,
                    text,
                    runs: vec![run],
                    font_size,
                    line_height,
                    inset,
                    style,
                    row_atoms: Vec::new(),
                }],
                true,
            );
        }
        let decorations: Vec<_> = core.decorations().collect();
        let mut plans = Vec::with_capacity(doc.line_count());
        for n in 0..doc.line_count() {
            let range = doc.line_range(n);
            let text: SharedString = doc.slice(range.start, range.end).into();
            let style = doc.block(n).and_then(|b| opts.lines.get(&b.kind)).cloned();
            let (font_size, line_height, inset) = line_box(style.as_ref(), base_size, base_lh);
            let local = |p: usize| to8(&text, p.clamp(range.start, range.end) - range.start);
            let mut cuts = vec![0, text.len()];
            let marks: Vec<_> = doc
                .marks()
                .iter()
                .filter(|m| m.from < range.end && m.to > range.start)
                .collect();
            for m in &marks {
                cuts.push(local(m.from));
                cuts.push(local(m.to));
            }
            let decos: Vec<_> = decorations
                .iter()
                .filter(|d| d.from < range.end && d.to > range.start)
                .collect();
            for d in &decos {
                cuts.push(local(d.from));
                cuts.push(local(d.to));
            }
            let marked_local = marked
                .clone()
                .filter(|r| r.start >= range.start && r.end <= range.end)
                .map(|r| local(r.start)..local(r.end));
            if let Some(r) = &marked_local {
                cuts.push(r.start);
                cuts.push(r.end);
            }
            for (at, c) in text.char_indices() {
                if c == ATOM_CHAR {
                    cuts.push(at);
                    cuts.push(at + c.len_utf8());
                }
            }
            cuts.sort_unstable();
            cuts.dedup();
            let line_run = style.as_ref().map(|s| s.run.clone()).unwrap_or_default();
            let mut runs = Vec::with_capacity(cuts.len());
            let mut row_atoms = Vec::new();
            for w in cuts.windows(2) {
                let (a, b) = (w[0], w[1]);
                if a >= b {
                    continue;
                }
                let start16 = range.start + text[..a].chars().map(char::len_utf16).sum::<usize>();
                let mut s = RunStyle::default();
                s.over(&line_run);
                for m in &marks {
                    if m.from <= start16 && start16 < m.to {
                        if let Some(ms) = opts.marks.get(&m.kind) {
                            s.over(ms);
                        }
                    }
                }
                for d in &decos {
                    if d.from <= start16 && start16 < d.to {
                        if let Some(ds) = opts.decorations.get(&d.name) {
                            s.over(ds);
                        }
                    }
                }
                let mut run = run_for(b - a, &s, &base, &text_style, opts.text_color, font_size);
                if marked_local
                    .as_ref()
                    .is_some_and(|r| r.start <= a && b <= r.end)
                {
                    run.underline = Some(UnderlineStyle {
                        color: Some(run.color),
                        thickness: px(1.0),
                        wavy: false,
                        offset: px(0.0),
                    });
                }
                if text[a..b].starts_with(ATOM_CHAR) {
                    let atom = doc.atom_at(start16);
                    let (size, align) = atom
                        .and_then(|a| atom_sizes.get(&a.id))
                        .map(|&(i, size)| (size, self.atoms[i].align))
                        .unwrap_or_default();
                    let run_size = run.font_size.unwrap_or(font_size);
                    let run_line_height = if font_size > px(0.0) {
                        line_height * (run_size / font_size)
                    } else {
                        line_height
                    };
                    let line = LineText::measure(&run.font, run_size, run_line_height, window);
                    let (inline_box, place) = atom_box(size, align, &line);
                    if matches!(place, Place::Top | Place::Bottom) {
                        row_atoms.push(RowAtom {
                            run: runs.len(),
                            byte: a,
                            width: size.width,
                            height: size.height,
                            top: place == Place::Top,
                        });
                    }
                    if let Some(atom) = atom {
                        places.insert(atom.id.clone(), place);
                    }
                    run.inline_box = Some(inline_box);
                }
                runs.push(run);
            }
            plans.push(LinePlan {
                start: range.start,
                text,
                runs,
                font_size,
                line_height,
                inset,
                style,
                row_atoms,
            });
        }
        (plans, false)
    }
}

/// The font size, line height and text inset (padding plus border) of a line with `style`.
fn line_box(
    style: Option<&LineStyle>,
    base: Pixels,
    base_lh: Pixels,
) -> (Pixels, Pixels, Edges<Pixels>) {
    let Some(style) = style else {
        return (base, base_lh, Edges::default());
    };
    let size = style.run.font_size.unwrap_or(base);
    let lh = if base > px(0.0) {
        (base_lh * (size / base)).round()
    } else {
        base_lh
    };
    let inset = Edges {
        top: style.padding.top + style.border.top,
        right: style.padding.right + style.border.right,
        bottom: style.padding.bottom + style.border.bottom,
        left: style.padding.left + style.border.left,
    };
    (size, lh, inset)
}

fn total_height(plans: &[LinePlan], width: Pixels, window: &mut Window) -> Pixels {
    let mut h = px(0.0);
    for plan in plans {
        let rows = plan
            .layout(width, window)
            .and_then(|(_, rows)| rows.last().map(|r| r.top + r.height))
            .unwrap_or(plan.line_height);
        h += rows.max(plan.line_height) + plan.inset.top + plan.inset.bottom;
    }
    h
}

impl Element for EditorElement {
    type RequestLayoutState = Layout;
    type PrepaintState = Prepainted;

    fn id(&self) -> Option<ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static core::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, Layout) {
        if self.options.inherit_color {
            let inherited = window.text_style().color;
            self.options.text_color = inherited;
            self.options.caret_color = inherited;
            self.options.placeholder_color = Hsla {
                a: inherited.a * 0.45,
                ..inherited
            };
        }
        let mut atom_sizes = HashMap::new();
        let max = size(AvailableSpace::MaxContent, AvailableSpace::MaxContent);
        for (i, atom) in self.atoms.iter_mut().enumerate() {
            let s = atom.element.layout_as_root(max, window, cx);
            atom_sizes.insert(atom.id.clone(), (i, s));
        }
        let mut places = HashMap::new();
        let (plans, placeholder) = self.plans(window, cx, &atom_sizes, &mut places);
        let plans = Rc::new(plans);
        let mut style = Style::default();
        style.flex_grow = 1.0;
        style.min_size.width = px(0.0).into();
        style.min_size.height = px(0.0).into();
        let measured = plans.clone();
        let cache: Rc<RefCell<Option<(Pixels, Pixels)>>> = Rc::default();
        let layout = window.request_measured_layout(style, move |known, available, window, _| {
            let width = known.width.unwrap_or(match available.width {
                AvailableSpace::Definite(w) => w,
                AvailableSpace::MinContent => px(0.0),
                AvailableSpace::MaxContent => px(1.0e6),
            });
            if let Some((w, h)) = *cache.borrow() {
                if w == width {
                    return size(width, h);
                }
            }
            let h = total_height(&measured, width, window);
            *cache.borrow_mut() = Some((width, h));
            let intrinsic = if known.width.is_some()
                || matches!(available.width, AvailableSpace::Definite(_))
            {
                width
            } else {
                px(0.0)
            };
            size(intrinsic, h)
        });
        (
            layout,
            Layout {
                plans,
                atom_sizes,
                places,
                placeholder,
            },
        )
    }

    fn prepaint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        layout: &mut Layout,
        window: &mut Window,
        cx: &mut App,
    ) -> Prepainted {
        self.state
            .update(cx, |s, cx| s.check_composition(window, cx));
        let shared = self.state.read(cx).shared();
        let (revision, selection) = {
            let core = shared.core.borrow();
            (core.revision(), core.selection())
        };

        let mut lines = Vec::with_capacity(layout.plans.len());
        let mut top = px(0.0);
        for plan in layout.plans.iter() {
            let Some((wrapped, rows)) = plan.layout(bounds.size.width, window) else {
                continue;
            };
            let text_h = rows
                .last()
                .map_or(plan.line_height, |r| r.top + r.height)
                .max(plan.line_height);
            let height = text_h + plan.inset.top + plan.inset.bottom;
            lines.push(LineGeom {
                start: plan.start,
                text: plan.text.clone(),
                top,
                height,
                inset: plan.inset,
                wrapped,
                rows,
            });
            top += height;
        }
        let content_height = top;

        let viewport_changed;
        let origin;
        {
            let mut g = shared.geometry.borrow_mut();
            let follow = g.revision != Some(revision)
                || g.lines.is_empty()
                || g.bounds.map(|b| b.size) != Some(bounds.size);
            g.lines = lines;
            g.revision = Some(revision);
            g.bounds = Some(bounds);
            g.transform = Some((window.element_transform(), window.scale_factor()));
            g.line_height = line_height_for(window, self.options.font_size);
            g.content_height = content_height;
            let max_scroll = (content_height - bounds.size.height).max(px(0.0));
            if follow || g.scroll_top > max_scroll {
                if let Some((at, h, _)) = g.caret_at(selection.head) {
                    if at.y < g.scroll_top {
                        g.scroll_top = at.y;
                    } else if at.y + h > g.scroll_top + bounds.size.height {
                        g.scroll_top = at.y + h - bounds.size.height;
                    }
                }
            }
            g.scroll_top = g.scroll_top.clamp(px(0.0), max_scroll);
            let (s, e) = (g.scroll_top, g.scroll_top + bounds.size.height);
            let first = g
                .lines
                .iter()
                .position(|l| l.top + l.height > s)
                .unwrap_or(0);
            let last = g.lines.iter().rposition(|l| l.top < e).unwrap_or(first);
            let viewport = (first, last + 1);
            viewport_changed = g.viewport != viewport;
            g.viewport = viewport;
            origin = point(bounds.left(), bounds.top() - g.scroll_top);
        }
        if viewport_changed {
            self.state
                .update(cx, |s, cx| s.emit_viewport_change(window, cx));
        }

        let g = shared.geometry.borrow();
        let at = |p: gpui::Point<Pixels>| point(origin.x + p.x, origin.y + p.y);
        let mut boxes = Vec::new();
        for (plan, line) in layout.plans.iter().zip(&g.lines) {
            let Some(style) = &plan.style else { continue };
            let b = Bounds::new(
                at(point(px(0.0), line.top)),
                size(bounds.size.width, line.height),
            );
            if let Some(bg) = style.background {
                boxes.push(fill(b, bg));
            }
            let (w, c) = (&style.border, &style.border_colors);
            for (o, s, color) in [
                (b.origin, size(w.left, b.size.height), c.left),
                (
                    point(b.right() - w.right, b.top()),
                    size(w.right, b.size.height),
                    c.right,
                ),
                (b.origin, size(b.size.width, w.top), c.top),
                (
                    point(b.left(), b.bottom() - w.bottom),
                    size(b.size.width, w.bottom),
                    c.bottom,
                ),
            ] {
                if let Some(color) = color.filter(|_| s.width > px(0.0) && s.height > px(0.0)) {
                    boxes.push(fill(Bounds::new(o, s), color));
                }
            }
        }
        {
            let core = shared.core.borrow();
            for d in core.line_decorations() {
                let Some(bg) = self
                    .options
                    .decorations
                    .get(&d.name)
                    .and_then(|s| s.background)
                else {
                    continue;
                };
                if let Some(line) = g.lines.get(d.line) {
                    let b = Bounds::new(
                        at(point(px(0.0), line.top)),
                        size(bounds.size.width, line.height),
                    );
                    boxes.push(fill(b, bg));
                }
            }
        }

        let selection_quads = if selection.is_empty() || layout.placeholder {
            Vec::new()
        } else {
            g.rects(selection.from(), selection.to())
                .into_iter()
                .map(|r| {
                    fill(
                        Bounds::new(at(r.origin), r.size),
                        self.options.selection_color,
                    )
                })
                .collect()
        };
        let caret = (selection.is_empty() || layout.placeholder)
            .then(|| {
                g.caret_at(if layout.placeholder {
                    0
                } else {
                    selection.head
                })
            })
            .flatten()
            .map(|(p, h, _)| {
                fill(
                    Bounds::new(at(p), size(CARET_WIDTH, h)),
                    self.options.caret_color,
                )
            });

        let mut placed = Vec::new();
        if !layout.placeholder {
            let core = shared.core.borrow();
            for atom in core.doc().atoms() {
                let Some(&(i, s)) = layout.atom_sizes.get(&atom.id) else {
                    continue;
                };
                let (Some((p, row)), Some(place)) =
                    (g.glyph_at(atom.at), layout.places.get(&atom.id))
                else {
                    continue;
                };
                let top = p.y
                    + match *place {
                        Place::Above(a) => row.baseline - a,
                        Place::Top => px(0.0),
                        Place::Bottom => row.height - s.height,
                    };
                let o = at(point(p.x, top));
                self.atoms[i].element.prepaint_at(o, window, cx);
                placed.push(i);
            }
        }
        Prepainted {
            selection: selection_quads,
            caret,
            boxes,
            placed,
            origin,
        }
    }

    fn paint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        layout: &mut Layout,
        pre: &mut Prepainted,
        window: &mut Window,
        cx: &mut App,
    ) {
        let focus = self.state.read(cx).focus_handle();
        window.handle_input(
            &focus,
            ElementInputHandler::new(bounds, self.state.clone()),
            cx,
        );
        let shared = self.state.read(cx).shared();
        let origin = pre.origin;
        let caret_shown = self.state.update(cx, |s, cx| s.caret_shown(window, cx));
        window.with_content_mask(Some(ContentMask::rect(bounds)), |window| {
            for q in pre.boxes.drain(..) {
                window.paint_quad(q);
            }
            for q in pre.selection.drain(..) {
                window.paint_quad(q);
            }
            {
                let g = shared.geometry.borrow();
                for (plan, line) in layout.plans.iter().zip(&g.lines) {
                    let o = point(
                        origin.x + line.inset.left,
                        origin.y + line.top + line.inset.top,
                    );
                    if o.y > bounds.bottom() || o.y + line.height < bounds.top() {
                        continue;
                    }
                    line.wrapped
                        .paint_background(o, plan.line_height, TextAlign::Left, None, window, cx)
                        .ok();
                    line.wrapped
                        .paint(o, plan.line_height, TextAlign::Left, None, window, cx)
                        .ok();
                }
            }
            for &i in &pre.placed {
                self.atoms[i].element.paint(window, cx);
            }
            if caret_shown {
                if let Some(c) = pre.caret.take() {
                    window.paint_quad(c);
                }
            }
        });

        let state = self.state.clone();
        window.on_mouse_event(move |event: &MouseMoveEvent, phase, window, cx| {
            if phase == DispatchPhase::Bubble && event.pressed_button == Some(MouseButton::Left) {
                state.update(cx, |s, cx| s.on_mouse_drag(event, window, cx));
            }
        });
        let state = self.state.clone();
        window.on_mouse_event(move |event: &MouseUpEvent, phase, window, cx| {
            if phase == DispatchPhase::Bubble && event.button == MouseButton::Left {
                state.update(cx, |s, cx| s.on_mouse_up(event, window, cx));
            }
        });
    }
}
