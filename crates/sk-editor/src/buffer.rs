//! The text store behind a [`Document`](crate::Document). Positions are UTF-16 code units, the unit
//! the JS API speaks; [`SmallBuffer`] is a plain string with a line index, enough for a composer.

use std::ops::Range;

/// What a document needs of its text. Positions are UTF-16 code units; a line excludes its `\n`.
pub trait TextBuffer: Default {
    fn len_utf16(&self) -> usize;
    fn line_count(&self) -> usize;
    /// The UTF-16 range of line `n` (clamped to the last line), without its `\n`.
    fn line_range(&self, n: usize) -> Range<usize>;
    /// The line `pos` is on; a position at a `\n` belongs to the line it ends.
    fn line_at(&self, pos: usize) -> usize;
    fn slice(&self, range: Range<usize>) -> String;
    fn replace(&mut self, range: Range<usize>, text: &str);
    /// The whole text as UTF-8.
    fn text(&self) -> &str;
    /// UTF-8 byte offset of a UTF-16 position (rounded down to a `char` boundary).
    fn to_utf8(&self, pos: usize) -> usize;
    fn to_utf16(&self, byte: usize) -> usize;
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SmallBuffer {
    text: String,
    len16: usize,
    /// `(utf8, utf16)` start of every line after the first.
    breaks: Vec<(usize, usize)>,
}

impl SmallBuffer {
    pub fn new(text: &str) -> Self {
        let mut b = Self::default();
        b.replace(0..0, text);
        b
    }

    fn reindex(&mut self) {
        self.breaks.clear();
        let mut u16 = 0;
        for (at, c) in self.text.char_indices() {
            u16 += c.len_utf16();
            if c == '\n' {
                self.breaks.push((at + 1, u16));
            }
        }
        self.len16 = u16;
    }

    fn line_start(&self, n: usize) -> (usize, usize) {
        if n == 0 {
            (0, 0)
        } else {
            self.breaks[n - 1]
        }
    }
}

impl TextBuffer for SmallBuffer {
    fn len_utf16(&self) -> usize {
        self.len16
    }

    fn line_count(&self) -> usize {
        self.breaks.len() + 1
    }

    fn line_range(&self, n: usize) -> Range<usize> {
        let n = n.min(self.breaks.len());
        let start = self.line_start(n).1;
        let end = self.breaks.get(n).map_or(self.len16, |&(_, next)| next - 1);
        start..end
    }

    fn line_at(&self, pos: usize) -> usize {
        self.breaks.partition_point(|&(_, start)| start <= pos)
    }

    fn slice(&self, range: Range<usize>) -> String {
        let a = self.to_utf8(range.start);
        let b = self.to_utf8(range.end.max(range.start));
        self.text[a..b].to_owned()
    }

    fn replace(&mut self, range: Range<usize>, text: &str) {
        let a = self.to_utf8(range.start);
        let b = self.to_utf8(range.end.max(range.start));
        self.text.replace_range(a..b, text);
        self.reindex();
    }

    fn text(&self) -> &str {
        &self.text
    }

    fn to_utf8(&self, pos: usize) -> usize {
        let pos = pos.min(self.len16);
        let line = self.line_at(pos);
        let (mut byte, mut u16) = self.line_start(line);
        for c in self.text[byte..].chars() {
            if u16 + c.len_utf16() > pos {
                break;
            }
            u16 += c.len_utf16();
            byte += c.len_utf8();
        }
        byte
    }

    fn to_utf16(&self, byte: usize) -> usize {
        let byte = byte.min(self.text.len());
        let line = self.breaks.partition_point(|&(start, _)| start <= byte);
        let (start8, mut u16) = self.line_start(line);
        for (at, c) in self.text[start8..].char_indices() {
            if start8 + at >= byte {
                break;
            }
            u16 += c.len_utf16();
        }
        u16
    }
}

pub fn utf16_len(s: &str) -> usize {
    s.chars().map(char::len_utf16).sum()
}

/// The byte offset in `s` of UTF-16 offset `pos`; inside a surrogate pair rounds down.
pub(crate) fn byte_at(s: &str, pos: usize) -> usize {
    let mut units = 0;
    for (at, c) in s.char_indices() {
        if units + c.len_utf16() > pos {
            return at;
        }
        units += c.len_utf16();
    }
    s.len()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lines_and_offsets_count_utf16_units() {
        let b = SmallBuffer::new("я🙂\nab\n\nz");
        assert_eq!(b.len_utf16(), 9);
        assert_eq!(b.line_count(), 4);
        assert_eq!(b.line_range(0), 0..3);
        assert_eq!(b.line_range(1), 4..6);
        assert_eq!(b.line_range(2), 7..7);
        assert_eq!(b.line_range(3), 8..9);
        assert_eq!(b.line_at(3), 0, "the \\n belongs to the line it ends");
        assert_eq!(b.line_at(4), 1);
        assert_eq!(b.line_at(9), 3);
        assert_eq!(b.to_utf8(1), 2);
        assert_eq!(b.to_utf8(2), 2, "inside a surrogate pair rounds down");
        assert_eq!(b.to_utf8(3), 6);
        assert_eq!(b.to_utf16(6), 3);
        assert_eq!(b.to_utf16(b.text().len()), 9);
        assert_eq!(b.slice(1..6), "🙂\nab");
    }

    #[test]
    fn replace_keeps_the_index_current() {
        let mut b = SmallBuffer::new("ab\ncd");
        b.replace(1..4, "X\nY\n");
        assert_eq!(b.text(), "aX\nY\nd");
        assert_eq!(b.line_count(), 3);
        assert_eq!(b.line_range(2), 5..6);
        b.replace(0..6, "");
        assert_eq!((b.text(), b.line_count(), b.len_utf16()), ("", 1, 0));
    }
}
