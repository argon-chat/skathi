//! Grapheme-cluster and word boundary scanning, implemented on `std` alone.
//!
//! Adapted from gpui-vue (`crates/gpui-vue-core/src/custom_elements/input.rs`,
//! Apache-2.0), which is itself derived from the native renderer in
//! [remorses/gpuix](https://github.com/remorses/gpuix) @
//! `b8bbff4ef0dc00dc24c3d3f3ca47818e1be15c69` (Apache-2.0) and from
//! [zeronsh/comet](https://github.com/zeronsh/comet) (MIT).
//! See gpui-vue's `THIRD_PARTY_NOTICES.md`.
//!
//! gpui-vue used `unicode-segmentation`; vn-elements is allowed to depend on
//! `gpui` only, so the UAX #29 rules we actually need for caret motion are
//! re-implemented here:
//!
//! * grapheme clusters — CRLF, combining marks, ZWJ sequences, variation
//!   selectors, emoji modifiers and regional-indicator (flag) pairs are kept
//!   together. The `Extend` table below is a pragmatic subset: Latin, Greek,
//!   Cyrillic, Hebrew, Arabic, Devanagari, Thai, Tibetan and every emoji
//!   sequence are covered; exotic Indic/Brahmic conjuncts degrade to per-`char`
//!   motion, which is never *wrong*, only finer-grained than the platform.
//! * words — a simple three-class (word / whitespace / punctuation) scan that
//!   reproduces the Windows caret behaviour for ctrl-left / ctrl-right.

use std::ops::Range;

/// Snap a byte offset to the nearest `char` boundary at or below it, clamped to
/// the length of `text`.
pub fn snap(text: &str, offset: usize) -> usize {
    let mut offset = offset.min(text.len());
    while !text.is_char_boundary(offset) {
        offset -= 1;
    }
    offset
}

fn char_at(text: &str, offset: usize) -> Option<char> {
    text[offset..].chars().next()
}

fn char_before(text: &str, offset: usize) -> Option<char> {
    text[..offset].chars().next_back()
}

fn step_forward(text: &str, offset: usize) -> usize {
    match char_at(text, offset) {
        Some(character) => offset + character.len_utf8(),
        None => text.len(),
    }
}

fn step_backward(text: &str, offset: usize) -> usize {
    match char_before(text, offset) {
        Some(character) => offset - character.len_utf8(),
        None => 0,
    }
}

/// Characters that never start a new grapheme cluster (`Extend`, `SpacingMark`,
/// `ZWJ` and the emoji modifiers), as far as caret motion cares.
fn is_extend(character: char) -> bool {
    matches!(character as u32,
        0x0300..=0x036F     // combining diacritical marks
        | 0x0483..=0x0489
        | 0x0591..=0x05BD | 0x05BF | 0x05C1..=0x05C2 | 0x05C4..=0x05C5 | 0x05C7
        | 0x0610..=0x061A | 0x064B..=0x065F | 0x0670 | 0x06D6..=0x06DC
        | 0x06DF..=0x06E4 | 0x06E7..=0x06E8 | 0x06EA..=0x06ED
        | 0x0711 | 0x0730..=0x074A | 0x07A6..=0x07B0 | 0x07EB..=0x07F3
        | 0x0816..=0x0819 | 0x081B..=0x0823 | 0x0825..=0x0827 | 0x0829..=0x082D
        | 0x0900..=0x0903 | 0x093A..=0x094F | 0x0951..=0x0957 | 0x0962..=0x0963
        | 0x0981..=0x0983 | 0x09BC..=0x09CD | 0x0A01..=0x0A03 | 0x0A3C..=0x0A4D
        | 0x0B01..=0x0B03 | 0x0B3C..=0x0B57 | 0x0C00..=0x0C04 | 0x0C3E..=0x0C56
        | 0x0E31 | 0x0E34..=0x0E3A | 0x0E47..=0x0E4E
        | 0x0EB1 | 0x0EB4..=0x0EBC | 0x0EC8..=0x0ECD
        | 0x0F71..=0x0F84 | 0x0F86..=0x0F87 | 0x0F8D..=0x0FBC
        | 0x102B..=0x103E | 0x1056..=0x1059
        | 0x1712..=0x1714 | 0x1732..=0x1734 | 0x17B4..=0x17D3
        | 0x180B..=0x180D | 0x18A9 | 0x1920..=0x193B
        | 0x1AB0..=0x1AFF | 0x1DC0..=0x1DFF
        | 0x200D                       // zero width joiner
        | 0x20D0..=0x20F0              // combining marks for symbols (incl. keycap)
        | 0x2CEF..=0x2CF1 | 0x302A..=0x302F | 0x3099..=0x309A
        | 0xA66F..=0xA672 | 0xA674..=0xA67D | 0xA69E..=0xA69F
        | 0xFB1E | 0xFE00..=0xFE0F     // variation selectors
        | 0xFE20..=0xFE2F              // combining half marks
        | 0x101FD | 0x102E0 | 0x10376..=0x1037A
        | 0x11000..=0x11002 | 0x11038..=0x11046
        | 0x1F3FB..=0x1F3FF            // emoji skin-tone modifiers
        | 0xE0020..=0xE007F            // tag characters (subdivision flags)
        | 0xE0100..=0xE01EF            // variation selectors supplement
    )
}

fn is_regional_indicator(character: char) -> bool {
    matches!(character as u32, 0x1F1E6..=0x1F1FF)
}

/// Whether a grapheme cluster starts at `offset` (which must be a `char`
/// boundary strictly inside `text`).
fn is_cluster_boundary(text: &str, offset: usize) -> bool {
    let (Some(previous), Some(next)) = (char_before(text, offset), char_at(text, offset)) else {
        return true;
    };
    if previous == '\r' && next == '\n' {
        return false;
    }
    if is_extend(next) {
        return false;
    }
    if previous == '\u{200D}' {
        // A ZWJ glues whatever follows to the cluster (emoji ZWJ sequences).
        return false;
    }
    if is_regional_indicator(previous) && is_regional_indicator(next) {
        // Regional indicators pair up into flags: break only after a full pair.
        let mut count = 0usize;
        let mut scan = offset;
        while let Some(character) = char_before(text, scan) {
            if !is_regional_indicator(character) {
                break;
            }
            count += 1;
            scan -= character.len_utf8();
        }
        return count % 2 == 0;
    }
    true
}

/// The start of the grapheme cluster preceding `offset` (or 0).
pub fn previous_grapheme(text: &str, offset: usize) -> usize {
    let offset = snap(text, offset);
    if offset == 0 {
        return 0;
    }
    let mut index = step_backward(text, offset);
    while index > 0 && !is_cluster_boundary(text, index) {
        index = step_backward(text, index);
    }
    index
}

/// The start of the grapheme cluster following `offset` (or `text.len()`).
pub fn next_grapheme(text: &str, offset: usize) -> usize {
    let offset = snap(text, offset);
    if offset >= text.len() {
        return text.len();
    }
    let mut index = step_forward(text, offset);
    while index < text.len() && !is_cluster_boundary(text, index) {
        index = step_forward(text, index);
    }
    index
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Class {
    Whitespace,
    Word,
    Punctuation,
}

fn classify(character: char) -> Class {
    if character.is_whitespace() {
        Class::Whitespace
    } else if character.is_alphanumeric() || character == '_' {
        Class::Word
    } else {
        Class::Punctuation
    }
}

/// The offset one word to the left, following the Windows convention: skip any
/// whitespace, then skip the run of same-class characters before it.
pub fn previous_word(text: &str, offset: usize) -> usize {
    let mut index = snap(text, offset);
    while let Some(character) = char_before(text, index) {
        if classify(character) != Class::Whitespace {
            break;
        }
        index -= character.len_utf8();
    }
    let Some(class) = char_before(text, index).map(classify) else {
        return index;
    };
    while let Some(character) = char_before(text, index) {
        if classify(character) != class {
            break;
        }
        index -= character.len_utf8();
    }
    index
}

/// The offset one word to the right: skip the run of same-class characters at
/// the caret, then any whitespace that follows it (so the caret lands on the
/// first character of the next word).
pub fn next_word(text: &str, offset: usize) -> usize {
    let mut index = snap(text, offset);
    let Some(class) = char_at(text, index).map(classify) else {
        return text.len();
    };
    while let Some(character) = char_at(text, index) {
        if classify(character) != class {
            break;
        }
        index += character.len_utf8();
    }
    if class != Class::Whitespace {
        while let Some(character) = char_at(text, index) {
            if classify(character) != Class::Whitespace {
                break;
            }
            index += character.len_utf8();
        }
    }
    index
}

/// The run of same-class characters containing `offset` — what a double click
/// selects.
pub fn word_range_at(text: &str, offset: usize) -> Range<usize> {
    let offset = snap(text, offset);
    // At the very end of the text there is no character *at* the offset, so
    // fall back to the one before it (double-clicking past the last word still
    // selects that word).
    let class = match char_at(text, offset).or_else(|| char_before(text, offset)) {
        Some(character) => classify(character),
        None => return 0..0,
    };
    let mut start = offset;
    while let Some(character) = char_before(text, start) {
        if classify(character) != class {
            break;
        }
        start -= character.len_utf8();
    }
    let mut end = offset;
    while let Some(character) = char_at(text, end) {
        if classify(character) != class {
            break;
        }
        end += character.len_utf8();
    }
    start..end
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn combining_marks_move_with_their_base() {
        let text = "e\u{0301}x"; // "éx", decomposed
        assert_eq!(next_grapheme(text, 0), 3);
        assert_eq!(previous_grapheme(text, 3), 0);
        assert_eq!(previous_grapheme(text, text.len()), 3);
    }

    #[test]
    fn emoji_sequences_are_single_clusters() {
        let family = "\u{1F468}\u{200D}\u{1F469}\u{200D}\u{1F467}";
        assert_eq!(next_grapheme(family, 0), family.len());
        assert_eq!(previous_grapheme(family, family.len()), 0);

        let thumbs_up = "\u{1F44D}\u{1F3FD}"; // with skin-tone modifier
        assert_eq!(next_grapheme(thumbs_up, 0), thumbs_up.len());

        let flag = "\u{1F1E9}\u{1F1EA}"; // regional indicator pair
        assert_eq!(next_grapheme(flag, 0), flag.len());
        assert_eq!(previous_grapheme(flag, flag.len()), 0);

        let two_flags = "\u{1F1E9}\u{1F1EA}\u{1F1EB}\u{1F1F7}";
        assert_eq!(next_grapheme(two_flags, 0), 8);
        assert_eq!(previous_grapheme(two_flags, two_flags.len()), 8);
    }

    #[test]
    fn plain_ascii_moves_one_character() {
        assert_eq!(next_grapheme("abc", 0), 1);
        assert_eq!(next_grapheme("abc", 3), 3);
        assert_eq!(previous_grapheme("abc", 0), 0);
        assert_eq!(previous_grapheme("abc", 2), 1);
    }

    #[test]
    fn word_motion_follows_the_windows_convention() {
        let text = "hello  brave world";
        assert_eq!(next_word(text, 0), 7); // start of "brave"
        assert_eq!(next_word(text, 7), 13); // start of "world"
        assert_eq!(next_word(text, 13), text.len());
        assert_eq!(previous_word(text, text.len()), 13);
        assert_eq!(previous_word(text, 13), 7);
        assert_eq!(previous_word(text, 7), 0);
    }

    #[test]
    fn word_motion_treats_punctuation_as_its_own_run() {
        let text = "a.b, c";
        assert_eq!(next_word(text, 0), 1); // "a" -> "."
        assert_eq!(next_word(text, 1), 2); // "." -> "b"
        assert_eq!(next_word(text, 2), 3); // "b" -> ","
        assert_eq!(next_word(text, 3), 5); // "," -> whitespace -> "c"
        assert_eq!(previous_word(text, 5), 3);
    }

    #[test]
    fn double_click_selects_the_run_under_the_caret() {
        let text = "hello brave world";
        assert_eq!(word_range_at(text, 8), 6..11);
        assert_eq!(word_range_at(text, 6), 6..11);
        assert_eq!(word_range_at(text, 5), 5..6); // the space itself
        assert_eq!(word_range_at(text, text.len()), 12..17);
        assert_eq!(word_range_at("", 0), 0..0);
    }

    #[test]
    fn snap_lands_on_char_boundaries() {
        let text = "é🙂";
        assert_eq!(snap(text, 1), 0);
        assert_eq!(snap(text, 2), 2);
        assert_eq!(snap(text, 4), 2);
        assert_eq!(snap(text, 99), text.len());
    }
}
