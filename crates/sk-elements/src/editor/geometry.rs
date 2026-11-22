//! Where an editor's text was laid out last frame, in content coordinates (the top-left of the
//! first line is the origin; scrolling and the element's position are applied on the way out).
//! Positions are the model's UTF-16 offsets.

use std::ops::Range;

use gpui::{
    point, px, size, Bounds, Edges, Pixels, Point, SharedString, TransformationMatrix, WrappedLine,
};

/// One hard line as laid out: its box (padding and border included) and the wrapped text in it.
pub(crate) struct LineGeom {
    /// The UTF-16 offset of the line's first character in the document.
    pub start: usize,
    pub text: SharedString,
    pub top: Pixels,
    pub height: Pixels,
    /// Padding plus border: where the text sits inside the line box.
    pub inset: Edges<Pixels>,
    pub wrapped: WrappedLine,
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

    fn row(&self, r: usize) -> gpui::LineBox {
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

    /// `(byte range, top, height, left x, right x)` of each visual row.
    fn row_spans(&self) -> Vec<(Range<usize>, gpui::LineBox, Pixels, Pixels)> {
        let starts = self.row_starts();
        let mut out = Vec::new();
        for (r, &s) in starts.iter().enumerate() {
            let e = starts.get(r + 1).copied().unwrap_or(self.text.len());
            let base = self.x_at(s);
            out.push((s..e, self.row(r), px(0.0), self.x_at(e) - base));
        }
        out
    }
}

/// Last frame's layout and where it was on screen.
#[derive(Default)]
pub struct Geometry {
    pub(crate) lines: Vec<LineGeom>,
    pub(crate) bounds: Option<Bounds<Pixels>>,
    pub(crate) transform: Option<(TransformationMatrix, f32)>,
    pub(crate) line_height: Pixels,
    pub(crate) content_height: Pixels,
    pub(crate) scroll_top: Pixels,
    pub(crate) viewport: (usize, usize),
    /// The document revision the lines were laid out from.
    pub(crate) revision: Option<u64>,
}

impl Geometry {
    fn line_of(&self, pos: usize) -> Option<&LineGeom> {
        let i = self.lines.partition_point(|l| l.start <= pos);
        self.lines.get(i.checked_sub(1)?)
    }

    /// The caret before `pos`: top-left in content coordinates, height and baseline.
    pub fn caret_at(&self, pos: usize) -> Option<(Point<Pixels>, Pixels, Pixels)> {
        let line = self.line_of(pos)?;
        let byte = line.to_utf8(pos.saturating_sub(line.start).min(line.len16()));
        Some(line.caret(byte))
    }

    /// The position closest to a point in content coordinates.
    pub fn index_at(&self, p: Point<Pixels>) -> usize {
        let Some(first) = self.lines.first() else {
            return 0;
        };
        if p.y < first.top {
            return 0;
        }
        let i = self
            .lines
            .partition_point(|l| l.top <= p.y)
            .saturating_sub(1);
        let line = &self.lines[i];
        if p.y >= line.top + line.height && i + 1 == self.lines.len() {
            return line.start + line.len16();
        }
        let byte = line.index_at(p, self.line_height);
        line.start + line.to_utf16(byte)
    }

    /// Rectangles in content coordinates covering `[from, to)`, one per visual row it touches. A
    /// selection running past a line's end shows a sliver for the line break.
    pub fn rects(&self, from: usize, to: usize) -> Vec<Bounds<Pixels>> {
        let mut out = Vec::new();
        for line in &self.lines {
            let end = line.start + line.len16();
            if end < from || line.start > to || (line.start == to && from < to) {
                continue;
            }
            let a = line.to_utf8(from.saturating_sub(line.start));
            let b = line.to_utf8(to.saturating_sub(line.start).min(line.len16()));
            let past_end = to > end;
            for (span, row, _, width) in line.row_spans() {
                let last_row = span.end == line.text.len();
                if span.end < a || span.start > b || (span.start == b && b < line.text.len()) {
                    continue;
                }
                let base = line.x_at(span.start);
                let x0 = line.x_at(a.max(span.start)) - base;
                let mut x1 = if b <= span.end {
                    line.x_at(b) - base
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
                    point(line.inset.left + x0, line.top + line.inset.top + row.top),
                    size((x1 - x0).max(px(0.0)), row.height),
                ));
            }
        }
        out
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
        if y < px(0.0) || y >= self.content_height {
            return None;
        }
        Some(self.index_at(point(at.x, y)))
    }

    pub fn content_height(&self) -> Pixels {
        self.content_height
    }

    pub fn scroll_top(&self) -> Pixels {
        self.scroll_top
    }

    pub fn viewport(&self) -> (usize, usize) {
        self.viewport
    }

    /// Content coordinates to window coordinates (before any transform).
    pub fn to_window(&self, p: Point<Pixels>) -> Point<Pixels> {
        let origin = self.bounds.map_or(Point::default(), |b| b.origin);
        point(origin.x + p.x, origin.y + p.y - self.scroll_top)
    }

    pub fn from_window(&self, p: Point<Pixels>) -> Point<Pixels> {
        let origin = self.bounds.map_or(Point::default(), |b| b.origin);
        point(p.x - origin.x, p.y - origin.y + self.scroll_top)
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
