//! The gpui element. It lays out only the hard lines on screen, a margin around them, the caret's
//! line and any a geometry read asked for (marks, decorations and line blocks become text runs,
//! atoms become inline boxes); every other line has an estimated or last measured height in the
//! [`LineIndex`](super::layout::LineIndex). It places the atoms' elements on their U+FFFC and
//! paints the gutter, line boxes, rounded backgrounds, selection, text and caret.

use std::cell::RefCell;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::hash::{Hash, Hasher};
use std::ops::Range;
use std::rc::Rc;

use gpui::{
    fill, point, px, size, App, AvailableSpace, Bounds, ContentMask, Corners, DispatchPhase, Edges,
    Element, ElementId, ElementInputHandler, Entity, Font, GlobalElementId, Hsla, InlineBox,
    InspectorElementId, LayoutId, MouseButton, MouseMoveEvent, MouseUpEvent, PaintQuad, Pixels,
    ShapedLine, SharedString, Size, Style, TabSize, TextAlign, TextRun, TextStyle, TextWrap,
    UnderlineStyle, Window,
};
use sk_editor::{Decoration, Document, LineDecoration, ATOM_CHAR};

use super::geometry::{Geometry, LineGeom};
use super::layout::{Estimate, LayoutKey, RangeIndex};
use super::state::EditorState;
use super::{AtomChild, EditorOptions, LineStyle, RunStyle};
use crate::vertical_align::{top_above_baseline, Alignment, LineText, VerticalAlign};

const CARET_WIDTH: Pixels = px(2.0);

/// Text whose average advance stands in for lines not laid out yet.
const SAMPLE: &str = "The quick brown fox jumps over the lazy dog 0123456789 (x) = [y];";

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
        width: Option<Pixels>,
        rules: TextWrap,
        window: &mut Window,
    ) -> Option<gpui::WrappedLine> {
        let wrap = width
            .map(|width| (width - self.inset.left - self.inset.right - CARET_WIDTH).max(px(1.0)));
        window
            .text_system()
            .shape_text_with(self.text.clone(), self.font_size, runs, wrap, None, rules)
            .ok()?
            .into_iter()
            .next()
    }

    /// The wrapped line and its rows. Top- and bottom-aligned atoms first hold their row's
    /// strut; once the rows are known, each reserves the extent CSS gives its row: the bottom
    /// ones push the baseline down, the top ones the row's bottom.
    fn layout(
        &self,
        width: Option<Pixels>,
        rules: TextWrap,
        window: &mut Window,
    ) -> Option<(gpui::WrappedLine, Vec<gpui::LineBox>)> {
        let wrapped = self.shape(&self.runs, width, rules, window)?;
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
        let wrapped = self.shape(&runs, width, rules, window)?;
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

/// Indexes of the document's marks and the app's decorations, rebuilt when either changes.
#[derive(Default)]
pub(crate) struct FrameCaches {
    marks: Option<(u64, Rc<RangeIndex>)>,
    decorations: Option<((u64, u64), Rc<(Vec<Decoration>, RangeIndex)>)>,
    line_decorations: Option<((u64, u64), Rc<LineDecorations>)>,
    gutter_text: Option<((u64, u64, u64), Pixels)>,
}

/// Line decorations by line, in layer order.
struct LineDecorations {
    list: Vec<LineDecoration>,
    by_line: HashMap<usize, Vec<usize>>,
}

impl LineDecorations {
    fn on(&self, line: usize) -> impl Iterator<Item = &LineDecoration> {
        self.by_line
            .get(&line)
            .into_iter()
            .flatten()
            .map(|&i| &self.list[i])
    }
}

impl FrameCaches {
    fn marks(&mut self, doc: &Document, revision: u64) -> Rc<RangeIndex> {
        match &self.marks {
            Some((r, ix)) if *r == revision => ix.clone(),
            _ => {
                let ix = Rc::new(RangeIndex::new(doc.marks().iter().map(|m| (m.from, m.to))));
                self.marks = Some((revision, ix.clone()));
                ix
            }
        }
    }

    fn decorations(
        &mut self,
        core: &sk_editor::Editor,
        key: (u64, u64),
    ) -> Rc<(Vec<Decoration>, RangeIndex)> {
        match &self.decorations {
            Some((k, d)) if *k == key => d.clone(),
            _ => {
                let list: Vec<Decoration> = core.decorations().cloned().collect();
                let ix = RangeIndex::new(list.iter().map(|d| (d.from, d.to)));
                let d = Rc::new((list, ix));
                self.decorations = Some((key, d.clone()));
                d
            }
        }
    }

    fn line_decorations(
        &mut self,
        core: &sk_editor::Editor,
        key: (u64, u64),
    ) -> Rc<LineDecorations> {
        match &self.line_decorations {
            Some((k, d)) if *k == key => d.clone(),
            _ => {
                let list: Vec<LineDecoration> = core.line_decorations().cloned().collect();
                let mut by_line: HashMap<usize, Vec<usize>> = HashMap::new();
                for (i, d) in list.iter().enumerate() {
                    by_line.entry(d.line).or_default().push(i);
                }
                let d = Rc::new(LineDecorations { list, by_line });
                self.line_decorations = Some((key, d.clone()));
                d
            }
        }
    }
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

/// The gutter's column and where its parts go in it.
#[derive(Clone)]
pub(crate) struct GutterLayout {
    width: Pixels,
    numbers: Option<(Font, Pixels, Hsla, Pixels, TextAlign)>,
    text_x: Pixels,
    text_font: (Font, Pixels, Hsla),
}

pub(crate) struct Layout {
    /// Atom id → (index into `atoms`, measured size).
    atom_sizes: HashMap<String, (usize, Size<Pixels>)>,
    placeholder: bool,
    gutter: Option<GutterLayout>,
    inputs: Rc<EstimateInputs>,
}

/// Everything an estimate needs but the width.
pub(crate) struct EstimateInputs {
    base_size: Pixels,
    base_lh: Pixels,
    char_width: Pixels,
    tab_size: TabSize,
    lines: Rc<HashMap<String, LineStyle>>,
    wrap: bool,
    styles: u64,
}

impl EstimateInputs {
    fn for_width(&self, text_width: Pixels) -> (Estimate, LayoutKey) {
        let wrap_width = self.wrap.then(|| (text_width - CARET_WIDTH).max(px(1.0)));
        (
            Estimate {
                base_size: self.base_size,
                base_lh: self.base_lh,
                char_width: self.char_width,
                tab_size: self.tab_size,
                lines: self.lines.clone(),
                wrap_width,
            },
            LayoutKey {
                wrap_width,
                font_size: self.base_size,
                line_height: self.base_lh,
                char_width: self.char_width,
                tab_size: self.tab_size,
                styles: self.styles,
            },
        )
    }
}

pub(crate) struct Prepainted {
    plans: Vec<LinePlan>,
    selection: Vec<PaintQuad>,
    caret: Option<PaintQuad>,
    boxes: Vec<PaintQuad>,
    rounded: Vec<PaintQuad>,
    gutter_quads: Vec<PaintQuad>,
    gutter_text: Vec<(ShapedLine, gpui::Point<Pixels>, Pixels)>,
    placed: Vec<usize>,
    origin: gpui::Point<Pixels>,
    text_area: Bounds<Pixels>,
    gutter_area: Option<Bounds<Pixels>>,
}

fn base_font(text_style: &TextStyle) -> Font {
    text_style.font()
}

fn styled_font(base: &Font, style: &RunStyle) -> Font {
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
    font
}

fn run_for(
    len: usize,
    style: &RunStyle,
    base: &Font,
    text_style: &TextStyle,
    color: Hsla,
    line_size: Pixels,
) -> TextRun {
    TextRun {
        len,
        font: styled_font(base, style),
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
    if text.is_ascii() {
        return pos.min(text.len());
    }
    let mut units = 0;
    for (at, c) in text.char_indices() {
        if units >= pos {
            return at;
        }
        units += c.len_utf16();
    }
    text.len()
}

fn to16(text: &str, byte: usize) -> usize {
    let byte = byte.min(text.len());
    if text.is_ascii() {
        return byte;
    }
    text[..byte].chars().map(char::len_utf16).sum()
}

/// The font size, line height and text inset (padding plus border) of a line with `style`.
pub(crate) fn line_box(
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

/// A background with rounded corners over a range, painted as quads under the text.
struct Rounded {
    from: usize,
    to: usize,
    color: Hsla,
    radius: Corners<Pixels>,
}

/// What laying a line out reads.
struct PlanCtx<'a> {
    doc: &'a Document,
    text_style: TextStyle,
    base: Font,
    base_size: Pixels,
    base_lh: Pixels,
    opts: &'a EditorOptions,
    marks: Rc<RangeIndex>,
    decorations: Rc<(Vec<Decoration>, RangeIndex)>,
    marked: Option<Range<usize>>,
    atom_sizes: &'a HashMap<String, (usize, Size<Pixels>)>,
    aligns: Vec<VerticalAlign>,
    placeholder: bool,
    rules: TextWrap,
    width: Option<Pixels>,
}

/// One line laid out.
struct Laid {
    plan: LinePlan,
    wrapped: Rc<gpui::WrappedLine>,
    rows: Vec<gpui::LineBox>,
    height: Pixels,
    width: Pixels,
}

fn without_background(style: &RunStyle) -> std::borrow::Cow<'_, RunStyle> {
    if style.rounded() {
        let mut s = style.clone();
        s.background = None;
        std::borrow::Cow::Owned(s)
    } else {
        std::borrow::Cow::Borrowed(style)
    }
}

impl PlanCtx<'_> {
    fn placeholder_plan(&self) -> LinePlan {
        let opts = self.opts;
        let text: SharedString = opts.placeholder.replace('\n', " ").into();
        let run = run_for(
            text.len(),
            &RunStyle::default(),
            &self.base,
            &self.text_style,
            opts.placeholder_color,
            self.base_size,
        );
        let style = self
            .doc
            .block(0)
            .and_then(|b| opts.lines.get(&b.kind))
            .cloned();
        let (font_size, line_height, inset) =
            line_box(style.as_ref(), self.base_size, self.base_lh);
        LinePlan {
            start: 0,
            text,
            runs: vec![run],
            font_size,
            line_height,
            inset,
            style,
            row_atoms: Vec::new(),
        }
    }

    fn plan(
        &self,
        n: usize,
        range: Range<usize>,
        window: &Window,
        places: &mut HashMap<String, Place>,
        rounded: &mut Vec<Rounded>,
        seen: &mut HashSet<(bool, usize)>,
    ) -> LinePlan {
        if self.placeholder {
            return self.placeholder_plan();
        }
        let (doc, opts) = (self.doc, self.opts);
        let text: SharedString = doc.slice(range.start, range.end).into();
        let style = doc.block(n).and_then(|b| opts.lines.get(&b.kind)).cloned();
        let (font_size, line_height, inset) =
            line_box(style.as_ref(), self.base_size, self.base_lh);
        let local = |p: usize| to8(&text, p.clamp(range.start, range.end) - range.start);
        let mut cuts = vec![0, text.len()];
        let all_marks = doc.marks();
        let marks: Vec<_> = self
            .marks
            .overlapping(range.start, range.end.max(range.start + 1))
            .into_iter()
            .map(|i| (i, &all_marks[i]))
            .filter(|(_, m)| m.from < range.end.max(range.start + 1) && m.to > range.start)
            .collect();
        for (i, m) in &marks {
            cuts.push(local(m.from));
            cuts.push(local(m.to));
            if let Some(ms) = opts.marks.get(&m.kind).filter(|s| s.rounded()) {
                if seen.insert((false, *i)) {
                    rounded.push(Rounded {
                        from: m.from,
                        to: m.to,
                        color: ms.background.unwrap_or_default(),
                        radius: ms.radius.unwrap_or_default(),
                    });
                }
            }
        }
        let (all_decos, deco_ix) = &*self.decorations;
        let decos: Vec<_> = deco_ix
            .overlapping(range.start, range.end.max(range.start + 1))
            .into_iter()
            .map(|i| (i, &all_decos[i]))
            .collect();
        for (i, d) in &decos {
            cuts.push(local(d.from));
            cuts.push(local(d.to));
            if let Some(ds) = opts.decorations.get(&d.name).filter(|s| s.rounded()) {
                if seen.insert((true, *i)) {
                    rounded.push(Rounded {
                        from: d.from,
                        to: d.to,
                        color: ds.background.unwrap_or_default(),
                        radius: ds.radius.unwrap_or_default(),
                    });
                }
            }
        }
        let marked_local = self
            .marked
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
            let start16 = range.start + to16(&text, a);
            let mut s = RunStyle::default();
            s.over(&line_run);
            for (_, m) in &marks {
                if m.from <= start16 && start16 < m.to {
                    if let Some(ms) = opts.marks.get(&m.kind) {
                        s.over(&without_background(ms));
                    }
                }
            }
            for (_, d) in &decos {
                if d.from <= start16 && start16 < d.to {
                    if let Some(ds) = opts.decorations.get(&d.name) {
                        s.over(&without_background(ds));
                    }
                }
            }
            let mut run = run_for(
                b - a,
                &s,
                &self.base,
                &self.text_style,
                opts.text_color,
                font_size,
            );
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
                    .and_then(|a| self.atom_sizes.get(&a.id))
                    .map(|&(i, size)| (size, self.aligns[i]))
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
        LinePlan {
            start: range.start,
            text,
            runs,
            font_size,
            line_height,
            inset,
            style,
            row_atoms,
        }
    }

    fn lay(&self, plan: LinePlan, window: &mut Window) -> Option<Laid> {
        let (wrapped, rows) = plan.layout(self.width, self.rules, window)?;
        let text_h = rows
            .last()
            .map_or(plan.line_height, |r| r.top + r.height)
            .max(plan.line_height);
        let height = text_h + plan.inset.top + plan.inset.bottom;
        let width = if self.width.is_none() {
            wrapped.unwrapped_layout.width + plan.inset.left + plan.inset.right
        } else {
            px(0.0)
        };
        Some(Laid {
            plan,
            wrapped: Rc::new(wrapped),
            rows,
            height,
            width,
        })
    }
}

/// A hash of the styles line heights depend on, so a change measures them afresh.
fn styles_hash(opts: &EditorOptions, text_style: &TextStyle) -> u64 {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    opts.font_epoch.hash(&mut h);
    text_style.font_family.hash(&mut h);
    text_style.font_weight.hash(&mut h);
    f32::from(text_style.letter_spacing).to_bits().hash(&mut h);
    f32::from(text_style.word_spacing).to_bits().hash(&mut h);
    let run = |r: &RunStyle, h: &mut std::collections::hash_map::DefaultHasher| {
        r.font_family.hash(h);
        r.font_weight.hash(h);
        r.font_size.map(|s| f32::from(s).to_bits()).hash(h);
    };
    let mut lines: Vec<_> = opts.lines.iter().collect();
    lines.sort_by(|a, b| a.0.cmp(b.0));
    for (name, s) in lines {
        name.hash(&mut h);
        run(&s.run, &mut h);
        for e in [s.padding, s.border] {
            for v in [e.top, e.right, e.bottom, e.left] {
                f32::from(v).to_bits().hash(&mut h);
            }
        }
    }
    let mut marks: Vec<_> = opts.marks.iter().collect();
    marks.sort_by(|a, b| a.0.cmp(b.0));
    for (name, s) in marks {
        name.hash(&mut h);
        run(s, &mut h);
    }
    h.finish()
}

impl EditorElement {
    fn gutter_layout(
        &self,
        shared: &super::EditorShared,
        text_style: &TextStyle,
        base: &Font,
        window: &mut Window,
    ) -> Option<GutterLayout> {
        let g = &self.options.gutter;
        if g.hidden {
            return None;
        }
        let core = shared.core.borrow();
        let key = (core.revision(), core.decoration_revision());
        let decorations = shared
            .geometry
            .borrow_mut()
            .caches
            .line_decorations(&core, key);
        let has_text = decorations.list.iter().any(|d| d.gutter.is_some());
        if !g.declared && g.numbers.is_none() && !has_text {
            return None;
        }
        let base_size = self.options.font_size;
        let dim = Hsla {
            a: self.options.text_color.a * 0.5,
            ..self.options.text_color
        };
        let text_size = g.run.font_size.unwrap_or(base_size);
        let text_font = styled_font(base, &g.run);
        let text_color = g.run.color.unwrap_or(dim);
        let shape_width = |text: &str, font: &Font, size: Pixels, window: &mut Window| {
            let run = TextRun {
                len: text.len(),
                font: font.clone(),
                color: text_color,
                tracking: text_style.letter_spacing,
                word_spacing: text_style.word_spacing,
                ..Default::default()
            };
            window
                .text_system()
                .shape_line(text.to_owned().into(), size, &[run], None)
                .width
        };
        let mut x = g.padding.left;
        let numbers = g.numbers.as_ref().map(|n| {
            let mut style = g.run.clone();
            style.over(&n.run);
            let font = styled_font(base, &style);
            let size = style.font_size.unwrap_or(base_size);
            let color = style.color.unwrap_or(dim);
            let digits = core.doc().line_count().max(1).to_string().len().max(2);
            let width = shape_width(&"0".repeat(digits), &font, size, window);
            x += width;
            (font, size, color, width, n.align)
        });
        let mut text_x = x;
        if has_text {
            let style_key = {
                let mut h = std::collections::hash_map::DefaultHasher::new();
                text_font.family.hash(&mut h);
                text_font.weight.hash(&mut h);
                f32::from(text_size).to_bits().hash(&mut h);
                h.finish()
            };
            let cache_key = (key.0, key.1, style_key);
            let cached = shared
                .geometry
                .borrow()
                .caches
                .gutter_text
                .filter(|(k, _)| *k == cache_key)
                .map(|(_, w)| w);
            let widest = match cached {
                Some(w) => w,
                None => {
                    let mut texts: Vec<&str> = decorations
                        .list
                        .iter()
                        .filter_map(|d| d.gutter.as_deref())
                        .collect();
                    texts.sort_by_key(|t| std::cmp::Reverse(t.chars().count()));
                    texts.dedup();
                    let w = texts
                        .iter()
                        .take(8)
                        .map(|t| shape_width(t, &text_font, text_size, window))
                        .fold(px(0.0), Pixels::max);
                    shared.geometry.borrow_mut().caches.gutter_text = Some((cache_key, w));
                    w
                }
            };
            if numbers.is_some() {
                text_x += shape_width(" ", &text_font, text_size, window);
            }
            x = text_x + widest;
        }
        let natural = x + g.padding.right + g.border_right;
        let scale = window.scale_factor();
        // Whole device pixels, so the border and the text area's clip do not share one.
        let width = px((f32::from(
            g.width
                .unwrap_or(natural)
                .max(g.min_width.unwrap_or_default()),
        ) * scale)
            .ceil()
            / scale);
        Some(GutterLayout {
            width,
            numbers,
            text_x,
            text_font: (text_font, text_size, text_color),
        })
    }
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
        let shared = self.state.read(cx).shared();
        let text_style = window.text_style();
        let base = base_font(&text_style);
        let base_size = self.options.font_size;
        let base_lh = line_height_for(window, base_size);
        let placeholder =
            shared.core.borrow().doc().is_empty() && !self.options.placeholder.is_empty();
        let sample = TextRun {
            len: SAMPLE.len(),
            font: base.clone(),
            tracking: text_style.letter_spacing,
            word_spacing: text_style.word_spacing,
            ..Default::default()
        };
        let char_width = window
            .text_system()
            .shape_line(SAMPLE.into(), base_size, &[sample], None)
            .width
            / SAMPLE.len() as f32;
        let gutter = self.gutter_layout(&shared, &text_style, &base, window);
        let inputs = Rc::new(EstimateInputs {
            base_size,
            base_lh,
            char_width,
            tab_size: text_style.tab_size,
            lines: self.options.lines.clone(),
            wrap: self.options.wrap,
            styles: styles_hash(&self.options, &text_style),
        });

        let mut style = Style::default();
        style.flex_grow = 1.0;
        style.min_size.width = px(0.0).into();
        style.min_size.height = px(0.0).into();
        let gutter_w = gutter.as_ref().map_or(px(0.0), |g| g.width);
        let measure_inputs = inputs.clone();
        let wrap = self.options.wrap;
        let cache: Rc<RefCell<Option<(Pixels, Pixels)>>> = Rc::default();
        let layout = window.request_measured_layout(style, move |known, available, _, _| {
            let definite = known.width.or(match available.width {
                AvailableSpace::Definite(w) => Some(w),
                _ => None,
            });
            let core = shared.core.borrow();
            let mut g = shared.geometry.borrow_mut();
            if let Some(width) = definite {
                if let Some((w, h)) = *cache.borrow() {
                    if w == width {
                        return size(width, h);
                    }
                }
                let (est, key) = measure_inputs.for_width((width - gutter_w).max(px(0.0)));
                g.index.sync(core.doc(), core.revision(), &key, &est);
                let h = g.content_height();
                g.reported_height = Some(h);
                *cache.borrow_mut() = Some((width, h));
                return size(width, h);
            }
            if g.index.count() == 0 {
                let probe = match available.width {
                    AvailableSpace::MinContent => px(0.0),
                    _ => px(1.0e6),
                };
                let (est, key) = measure_inputs.for_width(probe);
                g.index.sync(core.doc(), core.revision(), &key, &est);
            }
            let width = if wrap {
                px(0.0)
            } else {
                px(g.index.max_width()) + gutter_w + CARET_WIDTH
            };
            size(width, g.content_height())
        });
        (
            layout,
            Layout {
                atom_sizes,
                placeholder,
                gutter,
                inputs,
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
        self.prepaint_inner(bounds, layout, window, cx)
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
        self.paint_inner(bounds, layout, pre, window, cx);
    }
}

/// The visible part of `bounds` in content y, after scrolling.
fn visible_span(
    bounds: Bounds<Pixels>,
    mask: Bounds<Pixels>,
    scroll_top: Pixels,
) -> Option<(Pixels, Pixels)> {
    let top = bounds.top().max(mask.top());
    let bottom = bounds.bottom().min(mask.bottom());
    let (left, right) = (
        bounds.left().max(mask.left()),
        bounds.right().min(mask.right()),
    );
    (bottom > top && right > left).then(|| {
        (
            top - bounds.top() + scroll_top,
            bottom - bounds.top() + scroll_top,
        )
    })
}

fn rebuild_lines(g: &mut Geometry, laid: &BTreeMap<usize, Laid>) {
    g.lines = laid
        .iter()
        .map(|(&n, l)| LineGeom {
            n,
            start: l.plan.start,
            text: l.plan.text.clone(),
            top: px(g.index.top(n)),
            height: l.height,
            inset: l.plan.inset,
            wrapped: l.wrapped.clone(),
            rows: l.rows.clone(),
        })
        .collect();
}

impl EditorElement {
    fn prepaint_inner(
        &mut self,
        bounds: Bounds<Pixels>,
        layout: &mut Layout,
        window: &mut Window,
        cx: &mut App,
    ) -> Prepainted {
        self.state
            .update(cx, |s, cx| s.check_composition(window, cx));
        let (shared, marked) = {
            let state = self.state.read(cx);
            (state.shared(), state.marked_range())
        };
        let core = shared.core.borrow();
        let doc = core.doc();
        let revision = core.revision();
        let deco_key = (revision, core.decoration_revision());
        let selection = core.selection();
        let opts = &self.options;
        let text_style = window.text_style();
        let gutter_w = layout.gutter.as_ref().map_or(px(0.0), |g| g.width);
        let text_w = (bounds.size.width - gutter_w).max(px(0.0));
        let (est, key) = layout.inputs.for_width(text_w);

        let mut g = shared.geometry.borrow_mut();
        g.index.sync(doc, revision, &key, &est);
        let follow = g.revision != Some(revision)
            || g.lines.is_empty()
            || g.bounds.map(|b| b.size) != Some(bounds.size)
            || g.head != Some(selection.head);
        g.revision = Some(revision);
        g.head = Some(selection.head);
        g.bounds = Some(bounds);
        g.transform = Some((window.element_transform(), window.scale_factor()));
        g.line_height = layout.inputs.base_lh;
        g.char_width = layout.inputs.char_width;
        g.text_left = gutter_w;
        g.wrap = opts.wrap;

        let marks = g.caches.marks(doc, revision);
        let decorations = g.caches.decorations(&core, deco_key);
        let line_decorations = g.caches.line_decorations(&core, deco_key);
        let ctx = PlanCtx {
            doc,
            base: base_font(&text_style),
            text_style: text_style.clone(),
            base_size: layout.inputs.base_size,
            base_lh: layout.inputs.base_lh,
            opts,
            marks,
            decorations,
            marked,
            atom_sizes: &layout.atom_sizes,
            aligns: self.atoms.iter().map(|a| a.align).collect(),
            placeholder: layout.placeholder,
            rules: TextWrap {
                tab_size: text_style.tab_size,
                ..TextWrap::BREAK_WORD
            },
            width: opts.wrap.then_some(text_w),
        };

        let mut places = HashMap::new();
        let mut rounded = Vec::new();
        let mut seen = HashSet::new();
        let mut laid: BTreeMap<usize, Laid> = BTreeMap::new();
        let count = g.index.count();
        let lay = |n: usize,
                   g: &mut Geometry,
                   laid: &mut BTreeMap<usize, Laid>,
                   places: &mut HashMap<String, Place>,
                   rounded: &mut Vec<Rounded>,
                   seen: &mut HashSet<(bool, usize)>,
                   window: &mut Window| {
            if n >= count || laid.contains_key(&n) {
                return;
            }
            let range = g.index.range(n);
            let plan = ctx.plan(n, range, window, places, rounded, seen);
            if let Some(l) = ctx.lay(plan, window) {
                g.index.measure(n, f32::from(l.height), f32::from(l.width));
                laid.insert(n, l);
            }
        };

        let head_line = g.index.line_at(selection.head);
        let mut wanted = std::mem::take(&mut g.want);
        wanted.push(head_line);
        for n in wanted {
            lay(
                n,
                &mut g,
                &mut laid,
                &mut places,
                &mut rounded,
                &mut seen,
                window,
            );
        }

        let mask = window.content_mask().bounds;
        let view_h = bounds.size.height;
        // An editor as tall as its text scrolls with an ancestor it cannot anchor; a line above
        // the visible part measured there would move the text under it, so none is.
        let own_scroll = g.reported_height.is_some_and(|h| view_h + px(0.5) < h);
        if !own_scroll {
            g.scroll_top = px(0.0);
        }
        let anchor = {
            let n = g.index.heights.find(f32::from(g.scroll_top));
            (n, g.scroll_top - px(g.index.top(n)))
        };
        let mut visible = None;
        for _ in 0..4 {
            if own_scroll {
                let (n, off) = anchor;
                g.scroll_top = px(g.index.top(n)) + off.min(px(g.index.height(n)));
            }
            rebuild_lines(&mut g, &laid);
            if follow && !layout.placeholder {
                if let Some((at, h, _)) = g.caret_at(selection.head) {
                    if at.y < g.scroll_top {
                        g.scroll_top = at.y;
                    } else if at.y + h > g.scroll_top + view_h {
                        g.scroll_top = at.y + h - view_h;
                    }
                    if !opts.wrap {
                        if at.x < g.scroll_left {
                            g.scroll_left = at.x;
                        } else if at.x + CARET_WIDTH > g.scroll_left + text_w {
                            g.scroll_left = at.x + CARET_WIDTH - text_w;
                        }
                    }
                }
            }
            g.scroll_top = if own_scroll {
                g.scroll_top.clamp(px(0.0), g.max_scroll_top())
            } else {
                px(0.0)
            };
            let max_left = g.max_scroll_left();
            g.scroll_left = g.scroll_left.clamp(px(0.0), max_left);
            visible = visible_span(bounds, mask, g.scroll_top);
            let Some((top, bottom)) = visible else {
                break;
            };
            let margin = ((bottom - top) * 0.5).max(px(64.0));
            let above = if own_scroll { margin } else { px(0.0) };
            let mut n = g.index.heights.find(f32::from(top - above));
            let mut added = false;
            while n < count && px(g.index.top(n)) < bottom + margin {
                if !laid.contains_key(&n) {
                    lay(
                        n,
                        &mut g,
                        &mut laid,
                        &mut places,
                        &mut rounded,
                        &mut seen,
                        window,
                    );
                    added = true;
                }
                n += 1;
            }
            if !added {
                break;
            }
        }
        rebuild_lines(&mut g, &laid);

        let viewport = match visible {
            Some((top, bottom)) if count > 0 => {
                let first = g.index.heights.find(f32::from(top));
                let last = g.index.heights.find(f32::from(bottom - px(0.01)));
                (first, last + 1)
            }
            _ => (0, 0),
        };
        let viewport_changed = g.viewport != viewport;
        g.viewport = viewport;
        let height = g.content_height();
        let relayout = g
            .reported_height
            .is_some_and(|h| (h - height).abs() > px(0.5));
        if relayout {
            g.reported_height = Some(height);
        }
        let origin = point(
            bounds.left() + gutter_w - g.scroll_left,
            bounds.top() - g.scroll_top,
        );
        let at = |p: gpui::Point<Pixels>| point(origin.x + p.x, origin.y + p.y);
        let content_w = g.content_width();
        let text_area = Bounds::from_corners(
            point(bounds.left() + gutter_w, bounds.top()),
            bounds.bottom_right(),
        );

        let plans: Vec<LinePlan> = laid.values().map(|l| l.plan.clone()).collect();
        let mut boxes = Vec::new();
        for (plan, line) in plans.iter().zip(&g.lines) {
            let b = Bounds::new(at(point(px(0.0), line.top)), size(content_w, line.height));
            if let Some(style) = &plan.style {
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
            for d in line_decorations.on(line.n) {
                if let Some(bg) = opts.decorations.get(&d.name).and_then(|s| s.background) {
                    boxes.push(fill(b, bg));
                }
            }
        }

        let mut rounded_quads = Vec::new();
        if !layout.placeholder {
            for r in &rounded {
                let (rects, starts, ends) = g.painted_rects(r.from, r.to, false);
                let last = rects.len().saturating_sub(1);
                for (i, rect) in rects.into_iter().enumerate() {
                    let left = i == 0 && starts;
                    let right = i == last && ends;
                    let radii = Corners {
                        top_left: if left { r.radius.top_left } else { px(0.0) },
                        bottom_left: if left { r.radius.bottom_left } else { px(0.0) },
                        top_right: if right { r.radius.top_right } else { px(0.0) },
                        bottom_right: if right {
                            r.radius.bottom_right
                        } else {
                            px(0.0)
                        },
                    };
                    rounded_quads.push(
                        fill(Bounds::new(at(rect.origin), rect.size), r.color).corner_radii(radii),
                    );
                }
            }
        }

        let selection_quads = if selection.is_empty() || layout.placeholder {
            Vec::new()
        } else {
            g.painted_rects(selection.from(), selection.to(), true)
                .0
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

        let mut gutter_quads = Vec::new();
        let mut gutter_text = Vec::new();
        let gutter_area = layout.gutter.as_ref().map(|gl| {
            let area = Bounds::new(bounds.origin, size(gl.width, bounds.size.height));
            let gs = &opts.gutter;
            if let Some(bg) = gs.background {
                gutter_quads.push(fill(area, bg));
            }
            if let Some(color) = gs.border_color.filter(|_| gs.border_right > px(0.0)) {
                gutter_quads.push(fill(
                    Bounds::new(
                        point(area.right() - gs.border_right, area.top()),
                        size(gs.border_right, area.size.height),
                    ),
                    color,
                ));
            }
            let cell_w = gl.width - gs.border_right;
            for line in &g.lines {
                let y = origin.y + line.top;
                if y > bounds.bottom() || y + line.height < bounds.top() {
                    continue;
                }
                let row = line.row(0);
                let row_top = y + line.inset.top + row.top;
                let mut texts: Vec<(&str, Option<&RunStyle>)> = Vec::new();
                for d in line_decorations.on(line.n) {
                    let cell = gs.cells.get(&d.name);
                    if let Some(bg) = cell.and_then(|c| c.background) {
                        gutter_quads.push(fill(
                            Bounds::new(point(area.left(), y), size(cell_w, line.height)),
                            bg,
                        ));
                    }
                    if let Some(text) = d.gutter.as_deref().filter(|t| !t.is_empty()) {
                        texts.push((text, cell));
                    }
                }
                if let Some((font, font_size, color, width, align)) = &gl.numbers {
                    if !layout.placeholder || line.n == 0 {
                        let label = (line.n + 1).to_string();
                        let shaped =
                            shape_label(&label, font, *font_size, *color, &text_style, window);
                        let x = match align {
                            TextAlign::Left => gs.padding.left,
                            TextAlign::Center => gs.padding.left + (*width - shaped.width) / 2.0,
                            _ => gs.padding.left + *width - shaped.width,
                        };
                        gutter_text.push((shaped, point(area.left() + x, row_top), row.height));
                    }
                }
                let mut x = gl.text_x;
                for (text, cell) in texts {
                    let (font, font_size, color) = &gl.text_font;
                    let mut style = RunStyle::default();
                    if let Some(cell) = cell {
                        style.over(cell);
                    }
                    let font = styled_font(font, &style);
                    let color = style.color.unwrap_or(*color);
                    let shaped = shape_label(
                        text,
                        &font,
                        style.font_size.unwrap_or(*font_size),
                        color,
                        &text_style,
                        window,
                    );
                    let w = shaped.width;
                    gutter_text.push((shaped, point(area.left() + x, row_top), row.height));
                    x += w + gl.text_font.1 * 0.3;
                }
            }
            area
        });
        drop(g);
        drop(core);

        if viewport_changed {
            self.state
                .update(cx, |s, cx| s.emit_viewport_change(window, cx));
        }
        if relayout {
            self.state.update(cx, |_, cx| cx.notify());
        }

        let targets: Vec<(usize, gpui::Point<Pixels>)> = if layout.placeholder {
            Vec::new()
        } else {
            let g = shared.geometry.borrow();
            let core = shared.core.borrow();
            core.doc()
                .atoms()
                .iter()
                .filter_map(|atom| {
                    let &(i, s) = layout.atom_sizes.get(&atom.id)?;
                    let (p, row) = g.glyph_at(atom.at)?;
                    let top = p.y
                        + match *places.get(&atom.id)? {
                            Place::Above(a) => row.baseline - a,
                            Place::Top => px(0.0),
                            Place::Bottom => row.height - s.height,
                        };
                    Some((i, at(point(p.x, top))))
                })
                .collect()
        };
        let mut placed = Vec::with_capacity(targets.len());
        for (i, o) in targets {
            self.atoms[i].element.prepaint_at(o, window, cx);
            placed.push(i);
        }
        Prepainted {
            plans,
            selection: selection_quads,
            caret,
            boxes,
            rounded: rounded_quads,
            gutter_quads,
            gutter_text,
            placed,
            origin,
            text_area,
            gutter_area,
        }
    }

    fn paint_inner(
        &mut self,
        bounds: Bounds<Pixels>,
        _layout: &mut Layout,
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
            if let Some(area) = pre.gutter_area {
                window.with_content_mask(Some(ContentMask::rect(area)), |window| {
                    for q in pre.gutter_quads.drain(..) {
                        window.paint_quad(q);
                    }
                    for (line, o, h) in pre.gutter_text.drain(..) {
                        line.paint(o, h, TextAlign::Left, None, window, cx).ok();
                    }
                });
            }
            window.with_content_mask(Some(ContentMask::rect(pre.text_area)), |window| {
                for q in pre.boxes.drain(..) {
                    window.paint_quad(q);
                }
                for q in pre.rounded.drain(..) {
                    window.paint_quad(q);
                }
                for q in pre.selection.drain(..) {
                    window.paint_quad(q);
                }
                {
                    let g = shared.geometry.borrow();
                    for (plan, line) in pre.plans.iter().zip(&g.lines) {
                        let o = point(
                            origin.x + line.inset.left,
                            origin.y + line.top + line.inset.top,
                        );
                        if o.y > bounds.bottom() || o.y + line.height < bounds.top() {
                            continue;
                        }
                        line.wrapped
                            .paint_background(
                                o,
                                plan.line_height,
                                TextAlign::Left,
                                None,
                                window,
                                cx,
                            )
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

fn shape_label(
    text: &str,
    font: &Font,
    font_size: Pixels,
    color: Hsla,
    text_style: &TextStyle,
    window: &mut Window,
) -> ShapedLine {
    let run = TextRun {
        len: text.len(),
        font: font.clone(),
        color,
        tracking: text_style.letter_spacing,
        word_spacing: text_style.word_spacing,
        ..Default::default()
    };
    window
        .text_system()
        .shape_line(text.to_owned().into(), font_size, &[run], None)
}
