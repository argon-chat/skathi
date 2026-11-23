//! Where an editor's text is, in content coordinates (the top-left of the first line is the
//! origin; the gutter, scrolling and the element's position are applied on the way out).
//! Positions are the model's UTF-16 offsets. Only the lines last laid out have exact geometry;
//! the others answer from their estimated or last measured boxes.

use std::ops::Range;

use gpui::{
    point, px, size, Bounds, Edges, Pixels, Point, SharedString, TransformationMatrix, WrappedLine,
};

use super::layout::LineIndex;

/// One hard line as laid out: its box (padding and border included) and the wrapped text in it.
pub(crate) struct LineGeom {
    /// The line's number.
    pub n: usize,
    /// The UTF-16 offset of the line's first character in the document.
    pub start: usize,
    pub text: SharedString,
    pub top: Pixels,
    pub height: Pixels,
    /// Padding plus border: where the text sits inside the line box.
    pub inset: Edges<Pixels>,
    pub wrapped: std::rc::Rc<WrappedLine>,
    pub rows: Vec<gpui::LineBox>,
}

impl LineGeom {
    pub fn len16(&self) -> usize {
        self.text.chars().map(char::len_utf16).sum()
    }

    pub fn to_utf8(&self, local16: usize) -> usize {
        let mut units = 0;
        for (at, c) in self.text.char_indices() {
            if units >= local16 {
                return at;
            }
            units += c.len_utf16();
        }
        self.text.len()
    }

    pub fn to_utf16(&self, byte: usize) -> usize {
        self.text[..byte.min(self.text.len())]
            .chars()
            .map(char::len_utf16)
            .sum()
    }

    /// The byte index each visual row starts at.
    fn row_starts(&self) -> Vec<usize> {
        let runs = self.wrapped.runs();
        let mut out: Vec<usize> = vec![0];
        for b in self.wrapped.wrap_boundaries() {
            if let Some(g) = runs.get(b.run_ix).and_then(|r| r.glyphs.get(b.glyph_ix)) {
                out.push(g.index);
            }
        }
        out
    }

    fn x_at(&self, byte: usize) -> Pixels {
        self.wrapped.unwrapped_layout.x_for_index(byte)
    }

    pub fn row(&self, r: usize) -> gpui::LineBox {
        self.rows
            .get(r)
            .or(self.rows.last())
            .copied()
            .unwrap_or_default()
    }

    /// The caret before `byte`: its top-left in content coordinates and the row's height. At a
    /// wrap point the caret ends the row before, as gpui places it.
    pub fn caret(&self, byte: usize) -> (Point<Pixels>, Pixels, Pixels) {
        let starts = self.row_starts();
        let mut r = starts.len() - 1;
        for (i, w) in starts.windows(2).enumerate() {
            if byte <= w[1] {
                r = i;
                break;
            }
        }
        let row = self.row(r);
        let x = self.x_at(byte) - self.x_at(starts[r]);
        (
            point(self.inset.left + x, self.top + self.inset.top + row.top),
            row.height,
            row.baseline,
        )
    }

    pub fn index_at(&self, local: Point<Pixels>, line_height: Pixels) -> usize {
        let p = point(
            local.x - self.inset.left,
            local.y - self.top - self.inset.top,
        );
        let p = point(p.x, p.y.max(px(0.0)));
        match self.wrapped.closest_index_for_position(p, line_height) {
            Ok(i) | Err(i) => i.min(self.text.len()),
        }
    }

    /// `(byte range, row, right x)` of each visual row.
    fn row_spans(&self) -> Vec<(Range<usize>, gpui::LineBox, Pixels)> {
        let starts = self.row_starts();
        let mut out = Vec::new();
        for (r, &s) in starts.iter().enumerate() {
            let e = starts.get(r + 1).copied().unwrap_or(self.text.len());
            let base = self.x_at(s);
            out.push((s..e, self.row(r), self.x_at(e) - base));
        }
        out
    }

    /// Rectangles covering `[from, to)` of this line, one per visual row it touches.
    fn rects(&self, from: usize, to: usize, sliver: bool, out: &mut Vec<Bounds<Pixels>>) {
        let end = self.start + self.len16();
        if end < from || self.start > to || (self.start == to && from < to) {
            return;
        }
        let a = self.to_utf8(from.saturating_sub(self.start));
        let b = self.to_utf8(to.saturating_sub(self.start).min(self.len16()));
        let past_end = sliver && to > end;
        for (span, row, width) in self.row_spans() {
            let last_row = span.end == self.text.len();
            if span.end < a || span.start > b || (span.start == b && b < self.text.len()) {
                continue;
            }
            let base = self.x_at(span.start);
            let x0 = self.x_at(a.max(span.start)) - base;
            let mut x1 = if b <= span.end {
                self.x_at(b) - base
            } else {
                width
            };
            if past_end && last_row {
                x1 += px(4.0);
            }
            if x1 <= x0 && from != to {
                continue;
            }
            out.push(Bounds::new(
                point(self.inset.left + x0, self.top + self.inset.top + row.top),
                size((x1 - x0).max(px(0.0)), row.height),
            ));
        }
    }
}

/// The editor's layout as of the last frame, the line index behind it, and where it was on screen.
#[derive(Default)]
pub struct Geometry {
    /// The lines laid out last frame, by line number.
    pub(crate) lines: Vec<LineGeom>,
    pub(crate) index: LineIndex,
    pub(crate) bounds: Option<Bounds<Pixels>>,
    pub(crate) transform: Option<(TransformationMatrix, f32)>,
    pub(crate) line_height: Pixels,
    /// An average advance, for positions on lines not laid out.
    pub(crate) char_width: Pixels,
    pub(crate) scroll_top: Pixels,
    pub(crate) scroll_left: Pixels,
    /// The gutter's width: where the text area starts.
    pub(crate) text_left: Pixels,
    pub(crate) wrap: bool,
    pub(crate) viewport: (usize, usize),
    /// The document revision the lines were laid out from.
    pub(crate) revision: Option<u64>,
    pub(crate) head: Option<usize>,
    /// Lines to lay out next frame though they are off screen, for a geometry read.
    pub(crate) want: Vec<usize>,
    /// The height last reported to layout.
    pub(crate) reported_height: Option<Pixels>,
    pub(crate) caches: super::element::FrameCaches,
}

impl Geometry {
    fn laid(&self, n: usize) -> Option<&LineGeom> {
        self.lines
            .binary_search_by_key(&n, |l| l.n)
            .ok()
            .map(|i| &self.lines[i])
    }

    fn line_of(&self, pos: usize) -> Option<&LineGeom> {
        self.laid(self.index.line_at(pos))
    }

    pub fn lines_of(&self, from: usize, to: usize) -> Range<usize> {
        self.index.line_at(from)..self.index.line_at(to) + 1
    }

    /// The line at content height `y`.
    pub fn line_at_y(&self, y: Pixels) -> usize {
        self.index.heights.find(f32::from(y))
    }

    /// Asks for `lines` to be laid out next frame; `false` when they all are already.
    pub fn request(&mut self, lines: Range<usize>) -> bool {
        let missing: Vec<usize> = lines.filter(|&n| self.laid(n).is_none()).collect();
        let any = !missing.is_empty();
        self.want.extend(missing);
        any
    }

    pub fn is_shown(&self) -> bool {
        self.bounds.is_some()
    }

    pub fn set_scroll(&mut self, top: Option<Pixels>, left: Option<Pixels>) {
        if let Some(top) = top {
            self.scroll_top = top.clamp(px(0.0), self.max_scroll_top());
        }
        if let Some(left) = left {
            self.scroll_left = left.clamp(px(0.0), self.max_scroll_left());
        }
    }

    pub fn client_size(&self) -> (Pixels, Pixels) {
        self.bounds
            .map_or((px(0.0), px(0.0)), |b| (b.size.width, b.size.height))
    }

    pub fn gutter_width(&self) -> Pixels {
        self.text_left
    }

    /// An estimated caret on a line not laid out: rows of the average advance.
    fn estimated_caret(&self, pos: usize) -> Option<(Point<Pixels>, Pixels, Pixels)> {
        if self.index.count() == 0 {
            return None;
        }
        let n = self.index.line_at(pos);
        let col = pos.saturating_sub(self.index.range(n).start) as f32;
        let x = self.char_width * col;
        let lh = self.line_height.max(px(1.0));
        let room = self
            .bounds
            .filter(|_| self.wrap)
            .map(|b| (b.size.width - self.text_left).max(px(1.0)));
        let (x, row) = match room {
            Some(room) if x > room => {
                let row = (x / room).floor();
                (x - room * row, row)
            }
            _ => (x, 0.0),
        };
        let top = px(self.index.top(n)) + lh * row;
        Some((point(x, top), lh, lh * 0.8))
    }

    /// The caret before `pos`: top-left in content coordinates, height and baseline.
    pub fn caret_at(&self, pos: usize) -> Option<(Point<Pixels>, Pixels, Pixels)> {
        match self.line_of(pos) {
            Some(line) => {
                let byte = line.to_utf8(pos.saturating_sub(line.start).min(line.len16()));
                Some(line.caret(byte))
            }
            None => self.estimated_caret(pos),
        }
    }

    /// The left edge of the character at `pos` in content coordinates, on the top of its row,
    /// and that row. Unlike a caret, a character at a wrap point is on the row it starts.
    pub fn glyph_at(&self, pos: usize) -> Option<(Point<Pixels>, gpui::LineBox)> {
        let line = self.line_of(pos)?;
        let byte = line.to_utf8(pos.saturating_sub(line.start).min(line.len16()));
        let starts = line.row_starts();
        let r = starts.partition_point(|&s| s <= byte).saturating_sub(1);
        let row = line.row(r);
        let x = line.x_at(byte) - line.x_at(starts[r]);
        Some((
            point(line.inset.left + x, line.top + line.inset.top + row.top),
            row,
        ))
    }

    /// The position closest to a point in content coordinates.
    pub fn index_at(&self, p: Point<Pixels>) -> usize {
        let count = self.index.count();
        if count == 0 || p.y < px(0.0) {
            return 0;
        }
        let n = self.index.heights.find(f32::from(p.y));
        let range = self.index.range(n);
        if n + 1 == count && f32::from(p.y) >= self.index.heights.total() {
            return range.end;
        }
        match self.laid(n) {
            Some(line) => {
                let byte = line.index_at(p, self.line_height);
                line.start + line.to_utf16(byte)
            }
            None => {
                let cw = self.char_width.max(px(1.0));
                let col = (p.x / cw).round().max(0.0) as usize;
                (range.start + col).min(range.end)
            }
        }
    }

    /// Rectangles in content coordinates covering `[from, to)`, one per visual row it touches. A
    /// selection running past a line's end shows a sliver for the line break. Lines not laid out
    /// get one estimated rectangle.
    pub fn rects(&self, from: usize, to: usize) -> Vec<Bounds<Pixels>> {
        let mut out = Vec::new();
        if self.index.count() == 0 {
            return out;
        }
        for n in self.lines_of(from, to) {
            match self.laid(n) {
                Some(line) => line.rects(from, to, true, &mut out),
                None => {
                    let r = self.index.range(n);
                    let (a, b) = (from.max(r.start), to.min(r.end));
                    let x0 = self.char_width * (a - r.start) as f32;
                    let x1 = self.char_width * b.saturating_sub(r.start) as f32;
                    out.push(Bounds::new(
                        point(x0, px(self.index.top(n))),
                        size((x1 - x0).max(px(0.0)), px(self.index.height(n))),
                    ));
                }
            }
        }
        out
    }

    /// The rows of `[from, to)` on laid out lines only: what painting the range covers, the
    /// line-break sliver with `sliver`. Also whether the first rectangle starts the range and
    /// the last one ends it.
    pub(crate) fn painted_rects(
        &self,
        from: usize,
        to: usize,
        sliver: bool,
    ) -> (Vec<Bounds<Pixels>>, bool, bool) {
        let mut out = Vec::new();
        let lines = self.lines_of(from, to);
        let mut starts = false;
        let mut ends = false;
        for line in &self.lines {
            if !lines.contains(&line.n) {
                continue;
            }
            let before = out.len();
            line.rects(from, to, sliver, &mut out);
            if out.len() > before {
                if before == 0 {
                    starts = line.start <= from;
                }
                ends = line.start + line.len16() >= to;
            }
        }
        (out, starts, ends)
    }

    /// The start or end of the visual row `pos` is on.
    pub fn row_edge(&self, pos: usize, forward: bool) -> Option<usize> {
        let line = self.line_of(pos)?;
        let byte = line.to_utf8(pos - line.start);
        for (span, ..) in line.row_spans() {
            if byte >= span.start && byte <= span.end {
                let edge = if forward { span.end } else { span.start };
                return Some(line.start + line.to_utf16(edge));
            }
        }
        None
    }

    /// The position one visual row up or down from `pos`, keeping its x; `None` past the edge.
    pub fn row_step(&self, pos: usize, down: bool) -> Option<usize> {
        let (at, height, _) = self.caret_at(pos)?;
        let y = if down {
            at.y + height + px(1.0)
        } else {
            at.y - px(1.0)
        };
        if y < px(0.0) || y >= self.content_height() {
            return None;
        }
        Some(self.index_at(point(at.x, y)))
    }

    pub fn content_height(&self) -> Pixels {
        px(self.index.heights.total())
    }

    pub fn scroll_top(&self) -> Pixels {
        self.scroll_top
    }

    pub fn scroll_left(&self) -> Pixels {
        self.scroll_left
    }

    pub(crate) fn text_width(&self) -> Pixels {
        self.bounds
            .map_or(px(0.0), |b| (b.size.width - self.text_left).max(px(0.0)))
    }

    /// The widest line plus room for the caret: how far the text area scrolls sideways.
    pub fn content_width(&mut self) -> Pixels {
        if self.wrap {
            return self.text_width();
        }
        (px(self.index.max_width()) + px(2.0)).max(self.text_width())
    }

    pub fn max_scroll_top(&self) -> Pixels {
        let viewport = self.bounds.map_or(px(0.0), |b| b.size.height);
        (self.content_height() - viewport).max(px(0.0))
    }

    pub fn max_scroll_left(&mut self) -> Pixels {
        (self.content_width() - self.text_width()).max(px(0.0))
    }

    pub fn viewport(&self) -> (usize, usize) {
        self.viewport
    }

    /// Content coordinates to window coordinates (before any transform).
    pub fn to_window(&self, p: Point<Pixels>) -> Point<Pixels> {
        let origin = self.bounds.map_or(Point::default(), |b| b.origin);
        point(
            origin.x + self.text_left + p.x - self.scroll_left,
            origin.y + p.y - self.scroll_top,
        )
    }

    pub fn from_window(&self, p: Point<Pixels>) -> Point<Pixels> {
        let origin = self.bounds.map_or(Point::default(), |b| b.origin);
        point(
            p.x - origin.x - self.text_left + self.scroll_left,
            p.y - origin.y + self.scroll_top,
        )
    }

    /// A window-space rectangle as the page sees it, after the element's CSS transform.
    pub fn client_bounds(&self, b: Bounds<Pixels>) -> Bounds<Pixels> {
        self.transform_bounds(b).unwrap_or(b)
    }

    /// A point in window coordinates into content coordinates.
    pub fn client_to_content(&self, p: Point<Pixels>) -> Option<Point<Pixels>> {
        Some(self.from_window(self.untransform(p)?))
    }

    /// A window point (pointer events, IME) into layout space, undoing a CSS transform.
    pub(crate) fn untransform(&self, p: Point<Pixels>) -> Option<Point<Pixels>> {
        let Some((matrix, scale)) = self.transform else {
            return Some(p);
        };
        let inv = matrix.inverse()?;
        let d = inv.apply(point(
            px(f32::from(p.x) * scale),
            px(f32::from(p.y) * scale),
        ));
        let out = point(px(f32::from(d.x) / scale), px(f32::from(d.y) / scale));
        (f32::from(out.x).is_finite() && f32::from(out.y).is_finite()).then_some(out)
    }

    pub(crate) fn transform_bounds(&self, b: Bounds<Pixels>) -> Option<Bounds<Pixels>> {
        let Some((matrix, scale)) = self.transform else {
            return Some(b);
        };
        matrix.inverse()?;
        let map = |p: Point<Pixels>| {
            let d = matrix.apply(point(
                px(f32::from(p.x) * scale),
                px(f32::from(p.y) * scale),
            ));
            point(px(f32::from(d.x) / scale), px(f32::from(d.y) / scale))
        };
        let corners = [
            map(b.origin),
            map(point(b.right(), b.top())),
            map(point(b.left(), b.bottom())),
            map(b.bottom_right()),
        ];
        let min = corners
            .iter()
            .fold(point(px(f32::MAX), px(f32::MAX)), |m, c| {
                point(m.x.min(c.x), m.y.min(c.y))
            });
        let max = corners
            .iter()
            .fold(point(px(f32::MIN), px(f32::MIN)), |m, c| {
                point(m.x.max(c.x), m.y.max(c.y))
            });
        Some(Bounds::from_corners(min, max))
    }
}
