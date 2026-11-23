//! What the editor knows about every line without laying it out: where it starts, how tall and
//! wide it is (measured once it has been laid out, estimated until then), and indexes of the
//! marks and decorations by position, so a frame only touches the lines it shows.

use std::collections::HashMap;
use std::ops::Range;
use std::rc::Rc;

use gpui::{px, Pixels, TabSize};
use sk_editor::Document;

use super::LineStyle;

/// Line heights with prefix sums (a Fenwick tree), so a line's top and the line at a height are
/// `O(log n)` while single heights change.
#[derive(Default)]
pub(crate) struct Heights {
    vals: Vec<f32>,
    tree: Vec<f64>,
}

impl Heights {
    pub fn new(vals: Vec<f32>) -> Self {
        let n = vals.len();
        let mut tree = vec![0.0f64; n + 1];
        for (i, &v) in vals.iter().enumerate() {
            tree[i + 1] += f64::from(v);
            let parent = (i + 1) + ((i + 1) & (i + 1).wrapping_neg());
            if parent <= n {
                tree[parent] += tree[i + 1];
            }
        }
        Self { vals, tree }
    }

    pub fn get(&self, i: usize) -> f32 {
        self.vals.get(i).copied().unwrap_or(0.0)
    }

    /// `true` when the height changed.
    pub fn set(&mut self, i: usize, h: f32) -> bool {
        let Some(old) = self.vals.get_mut(i) else {
            return false;
        };
        if (*old - h).abs() < 0.01 {
            return false;
        }
        let delta = f64::from(h) - f64::from(*old);
        *old = h;
        let mut k = i + 1;
        while k < self.tree.len() {
            self.tree[k] += delta;
            k += k & k.wrapping_neg();
        }
        true
    }

    /// The sum of the heights before line `i`.
    pub fn top(&self, i: usize) -> f32 {
        let mut k = i.min(self.vals.len());
        let mut sum = 0.0;
        while k > 0 {
            sum += self.tree[k];
            k -= k & k.wrapping_neg();
        }
        sum as f32
    }

    pub fn total(&self) -> f32 {
        self.top(self.vals.len())
    }

    /// The line `y` falls on; the first or last line beyond the ends.
    pub fn find(&self, y: f32) -> usize {
        let n = self.vals.len();
        if n == 0 || y <= 0.0 {
            return 0;
        }
        let mut pos = 0;
        let mut rest = f64::from(y);
        let mut step = n.next_power_of_two();
        while step > 0 {
            let next = pos + step;
            if next <= n && self.tree[next] <= rest {
                pos = next;
                rest -= self.tree[next];
            }
            step >>= 1;
        }
        pos.min(n - 1)
    }
}

/// What line heights and widths depend on besides the text.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct LayoutKey {
    pub wrap_width: Option<Pixels>,
    pub font_size: Pixels,
    pub line_height: Pixels,
    pub char_width: Pixels,
    pub tab_size: TabSize,
    pub styles: u64,
}

/// A guess at a line's box before it is shaped.
pub(crate) struct Estimate {
    pub base_size: Pixels,
    pub base_lh: Pixels,
    pub char_width: Pixels,
    pub tab_size: TabSize,
    pub lines: Rc<HashMap<String, LineStyle>>,
    /// The room text wraps in, after the caret; `None` when lines do not wrap.
    pub wrap_width: Option<Pixels>,
}

impl Estimate {
    /// `(height, width)` of a line with this text and block.
    pub fn line(&self, text: &str, kind: Option<&str>) -> (f32, f32) {
        let style = kind.and_then(|k| self.lines.get(k));
        let (size, lh, inset) = super::element::line_box(style, self.base_size, self.base_lh);
        let scale = if self.base_size > px(0.0) {
            size / self.base_size
        } else {
            1.0
        };
        let tab_cols = match self.tab_size {
            TabSize::Spaces(n) => n.max(0.0),
            TabSize::Length(l) if self.char_width > px(0.0) => l / self.char_width,
            TabSize::Length(_) => 0.0,
        };
        let mut cols = 0.0f32;
        if text.is_ascii() && !text.contains('\t') {
            cols = text.len() as f32;
        } else {
            for c in text.chars() {
                if c == '\t' {
                    cols = if tab_cols > 0.0 {
                        ((cols / tab_cols).floor() + 1.0) * tab_cols
                    } else {
                        cols
                    };
                } else {
                    cols += 1.0;
                }
            }
        }
        let width = cols * f32::from(self.char_width) * scale;
        let rows = match self.wrap_width {
            Some(w) => {
                let room = f32::from(w - inset.left - inset.right).max(1.0);
                (width / room).ceil().max(1.0)
            }
            None => 1.0,
        };
        (
            rows * f32::from(lh) + f32::from(inset.top + inset.bottom),
            width + f32::from(inset.left + inset.right),
        )
    }
}

fn hash_line(text: &str, kind: Option<&str>) -> u64 {
    const K: u64 = 0x517c_c1b7_2722_0a95;
    let mut h: u64 = text.len() as u64;
    let mut chunks = text.as_bytes().chunks_exact(8);
    for c in &mut chunks {
        let w = u64::from_le_bytes(c.try_into().unwrap_or([0; 8]));
        h = (h.rotate_left(5) ^ w).wrapping_mul(K);
    }
    for &b in chunks.remainder() {
        h = (h.rotate_left(5) ^ u64::from(b)).wrapping_mul(K);
    }
    if let Some(kind) = kind {
        for &b in kind.as_bytes() {
            h = (h.rotate_left(5) ^ u64::from(b) ^ 0xff00).wrapping_mul(K);
        }
    }
    h
}

/// Every line's start, height and width.
#[derive(Default)]
pub(crate) struct LineIndex {
    revision: Option<u64>,
    key: Option<LayoutKey>,
    starts: Vec<usize>,
    len16: usize,
    hashes: Vec<u64>,
    measured: Vec<bool>,
    pub heights: Heights,
    widths: Vec<f32>,
    max_width: Option<f32>,
}

impl LineIndex {
    pub fn count(&self) -> usize {
        self.starts.len()
    }

    /// The line `pos` is on; a position at a `\n` belongs to the line it ends.
    pub fn line_at(&self, pos: usize) -> usize {
        self.starts.partition_point(|&s| s <= pos).saturating_sub(1)
    }

    pub fn range(&self, n: usize) -> Range<usize> {
        let start = self.starts.get(n).copied().unwrap_or(self.len16);
        let end = self
            .starts
            .get(n + 1)
            .map_or(self.len16, |&next| next.saturating_sub(1));
        start..end.max(start)
    }

    pub fn top(&self, n: usize) -> f32 {
        self.heights.top(n)
    }

    pub fn height(&self, n: usize) -> f32 {
        self.heights.get(n)
    }

    #[cfg(test)]
    fn is_measured(&self, n: usize) -> bool {
        self.measured.get(n).copied().unwrap_or(false)
    }

    /// Catches up with the document and the layout inputs. Lines whose text and block did not
    /// change keep what was measured of them, as long as the inputs stayed the same.
    pub fn sync(&mut self, doc: &Document, revision: u64, key: &LayoutKey, est: &Estimate) {
        let same_key = self.key.as_ref() == Some(key);
        if same_key && self.revision == Some(revision) {
            return;
        }
        let n = doc.line_count();
        let mut starts = Vec::with_capacity(n);
        let mut hashes = Vec::with_capacity(n);
        let mut heights = Vec::with_capacity(n);
        let mut widths = Vec::with_capacity(n);
        let mut at = 0usize;
        for (i, text) in doc.text().split('\n').enumerate() {
            starts.push(at);
            at += if text.is_ascii() {
                text.len()
            } else {
                sk_editor::utf16_len(text)
            } + 1;
            let kind = doc.block(i).map(|b| b.kind.as_str());
            hashes.push(hash_line(text, kind));
            let (h, w) = est.line(text, kind);
            heights.push(h);
            widths.push(w);
        }
        let mut measured = vec![false; starts.len()];
        if same_key && !self.hashes.is_empty() {
            let (old, new) = (self.hashes.len(), hashes.len());
            let prefix = self
                .hashes
                .iter()
                .zip(&hashes)
                .take_while(|(a, b)| a == b)
                .count();
            let suffix = self
                .hashes
                .iter()
                .rev()
                .zip(hashes.iter().rev())
                .take(old.min(new) - prefix)
                .take_while(|(a, b)| a == b)
                .count();
            let keep =
                |i: usize, j: usize, heights: &mut [f32], widths: &mut [f32], m: &mut [bool]| {
                    if self.measured[j] {
                        heights[i] = self.heights.get(j);
                        widths[i] = self.widths[j];
                        m[i] = true;
                    }
                };
            for i in 0..prefix {
                keep(i, i, &mut heights, &mut widths, &mut measured);
            }
            for k in 0..suffix {
                keep(
                    new - 1 - k,
                    old - 1 - k,
                    &mut heights,
                    &mut widths,
                    &mut measured,
                );
            }
        }
        self.len16 = doc.len();
        self.starts = starts;
        self.hashes = hashes;
        self.measured = measured;
        self.heights = Heights::new(heights);
        self.widths = widths;
        self.max_width = None;
        self.revision = Some(revision);
        self.key = Some(key.clone());
    }

    /// Records line `n` as laid out; `true` when its height changed.
    pub fn measure(&mut self, n: usize, height: f32, width: f32) -> bool {
        if n >= self.starts.len() {
            return false;
        }
        self.measured[n] = true;
        if (self.widths[n] - width).abs() > 0.01 {
            if self
                .max_width
                .is_some_and(|m| width > m || self.widths[n] >= m)
            {
                self.max_width = None;
            }
            self.widths[n] = width;
        }
        self.heights.set(n, height)
    }

    /// The widest line, measured or estimated.
    pub fn max_width(&mut self) -> f32 {
        *self
            .max_width
            .get_or_insert_with(|| self.widths.iter().copied().fold(0.0, f32::max))
    }
}

/// Ranges sorted by start with the running maximum of their ends, so the ones overlapping a span
/// are found without a scan of all. Each keeps its original order, which decides precedence.
pub(crate) struct RangeIndex {
    items: Vec<(usize, usize, usize)>,
    max_to: Vec<usize>,
}

impl RangeIndex {
    pub fn new(ranges: impl Iterator<Item = (usize, usize)>) -> Self {
        let mut items: Vec<(usize, usize, usize)> = ranges
            .enumerate()
            .map(|(order, (from, to))| (from, to, order))
            .collect();
        items.sort_by_key(|&(from, to, order)| (from, to, order));
        let mut max = 0;
        let max_to = items
            .iter()
            .map(|&(_, to, _)| {
                max = max.max(to);
                max
            })
            .collect();
        Self { items, max_to }
    }

    /// The original indexes of the ranges overlapping `[from, to)`, in original order.
    pub fn overlapping(&self, from: usize, to: usize) -> Vec<usize> {
        let start = self.max_to.partition_point(|&m| m <= from);
        let mut out: Vec<usize> = self.items[start..]
            .iter()
            .take_while(|&&(f, _, _)| f < to)
            .filter(|&&(_, t, _)| t > from)
            .map(|&(_, _, order)| order)
            .collect();
        out.sort_unstable();
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn heights_find_and_top_agree_after_updates() {
        let mut h = Heights::new(vec![10.0, 20.0, 30.0, 40.0, 50.0]);
        assert_eq!(h.total(), 150.0);
        assert_eq!(h.top(3), 60.0);
        assert_eq!(h.find(0.0), 0);
        assert_eq!(h.find(29.9), 1);
        assert_eq!(h.find(30.0), 2);
        assert_eq!(h.find(1000.0), 4);
        assert!(h.set(1, 5.0));
        assert_eq!(h.top(3), 45.0);
        assert_eq!(h.find(15.0), 2);
        assert_eq!(h.total(), 135.0);
        for y in 0..135 {
            let i = h.find(y as f32);
            assert!(
                h.top(i) <= y as f32 && (y as f32) < h.top(i + 1),
                "y={y} i={i}"
            );
        }
    }

    #[test]
    fn a_long_range_early_does_not_hide_later_overlaps() {
        let ix = RangeIndex::new([(0, 100), (5, 6), (20, 30), (40, 41)].into_iter());
        assert_eq!(ix.overlapping(25, 26), vec![0, 2]);
        assert_eq!(ix.overlapping(6, 20), vec![0]);
        assert_eq!(ix.overlapping(100, 200), Vec::<usize>::new());
    }

    fn est(wrap: Option<f32>) -> Estimate {
        Estimate {
            base_size: px(10.0),
            base_lh: px(14.0),
            char_width: px(5.0),
            tab_size: TabSize::Spaces(4.0),
            lines: Rc::default(),
            wrap_width: wrap.map(px),
        }
    }

    fn key() -> LayoutKey {
        LayoutKey {
            wrap_width: None,
            font_size: px(10.0),
            line_height: px(14.0),
            char_width: px(5.0),
            tab_size: TabSize::INITIAL,
            styles: 0,
        }
    }

    #[test]
    fn an_edit_keeps_the_measured_heights_of_the_lines_around_it() {
        let mut doc = Document::new();
        doc.set_value(sk_editor::EditorValue::plain("a\nb\nc\nd"));
        let mut ix = LineIndex::default();
        ix.sync(&doc, 1, &key(), &est(None));
        for n in 0..4 {
            ix.measure(n, 30.0 + n as f32, 0.0);
        }
        doc.replace_text_raw(2, 3, "B\nBB");
        ix.sync(&doc, 2, &key(), &est(None));
        assert_eq!(ix.count(), 5);
        assert_eq!(ix.height(0), 30.0);
        assert!(!ix.is_measured(1) && !ix.is_measured(2));
        assert_eq!((ix.height(3), ix.height(4)), (32.0, 33.0));
        assert_eq!(ix.range(2), 4..6);
    }

    #[test]
    fn estimates_wrap_by_width_and_count_tabs_to_their_stops() {
        let e = est(Some(50.0));
        assert_eq!(e.line("0123456789", None), (14.0, 50.0));
        assert_eq!(e.line("0123456789a", None).0, 28.0);
        assert_eq!(est(None).line("a\tb", None).1, 25.0);
    }
}
