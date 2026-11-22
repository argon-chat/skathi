//! Line, word and character differences over interned tokens: Myers, patience and histogram.

use std::collections::HashMap;
use std::hash::{BuildHasherDefault, Hasher};

use super::{fold, word};

pub(super) const NEWLINE: u16 = b'\n' as u16;
const RETURN: u16 = b'\r' as u16;

/// Tokens that occur more often than this in the old range are not anchors for the histogram
/// algorithm, as in git.
const MAX_CHAIN: usize = 64;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum Edit {
    Equal,
    Delete,
    Insert,
}

impl Edit {
    pub(super) fn name(self) -> &'static str {
        match self {
            Edit::Equal => "equal",
            Edit::Delete => "delete",
            Edit::Insert => "insert",
        }
    }
}

/// Runs of one kind of edit, in order; a delete always comes before the insert next to it.
#[derive(Default, Debug)]
pub(super) struct Script(pub(super) Vec<(Edit, usize)>);

impl Script {
    pub(super) fn push(&mut self, edit: Edit, count: usize) {
        if count == 0 {
            return;
        }
        match self.0.last_mut() {
            Some((last, run)) if *last == edit => *run += count,
            _ => self.0.push((edit, count)),
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum Algorithm {
    Histogram,
    Myers,
    Patience,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum Whitespace {
    None,
    Trailing,
    Amount,
    All,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum Inline {
    None,
    Words,
    Chars,
}

#[derive(Clone, Copy, Debug)]
pub(super) struct Options {
    pub(super) algorithm: Algorithm,
    pub(super) whitespace: Whitespace,
    pub(super) ignore_case: bool,
    pub(super) ignore_line_endings: bool,
    pub(super) context: usize,
    pub(super) inline: Inline,
}

impl Default for Options {
    fn default() -> Options {
        Options {
            algorithm: Algorithm::Histogram,
            whitespace: Whitespace::None,
            ignore_case: false,
            ignore_line_endings: false,
            context: 3,
            inline: Inline::None,
        }
    }
}

impl Options {
    fn plain(&self) -> bool {
        self.whitespace == Whitespace::None && !self.ignore_case && !self.ignore_line_endings
    }
}

/// A text as the pieces a difference is made of, each a range of its units.
pub(super) struct Pieces<'a> {
    pub(super) units: &'a [u16],
    pub(super) ranges: Vec<(usize, usize)>,
    /// The last line has no `\n` after it.
    pub(super) open_end: bool,
}

fn paired(units: &[u16], i: usize) -> bool {
    (0xD800..0xDC00).contains(&units[i])
        && units
            .get(i + 1)
            .is_some_and(|low| (0xDC00..0xE000).contains(low))
}

fn char_at(units: &[u16], i: usize) -> (char, usize) {
    let width = 1 + paired(units, i) as usize;
    let c = char::decode_utf16(units[i..i + width].iter().copied())
        .next()
        .and_then(Result::ok)
        .unwrap_or(char::REPLACEMENT_CHARACTER);
    (c, width)
}

/// Marks that belong to the character before them: combining marks, variation selectors, emoji
/// modifiers, and the joiner with what follows it.
fn joins(c: char) -> bool {
    matches!(c as u32,
        0x0300..=0x036F | 0x1AB0..=0x1AFF | 0x1DC0..=0x1DFF | 0x20D0..=0x20FF | 0xFE20..=0xFE2F
        | 0xFE00..=0xFE0F | 0x200D | 0x1F3FB..=0x1F3FF | 0xE0020..=0xE007F)
}

fn space(c: char) -> bool {
    c != '\n' && c.is_whitespace()
}

impl<'a> Pieces<'a> {
    pub(super) fn lines(units: &'a [u16]) -> Pieces<'a> {
        let mut ranges = Vec::new();
        let mut start = 0;
        for (i, unit) in units.iter().enumerate() {
            if *unit == NEWLINE {
                ranges.push((start, i));
                start = i + 1;
            }
        }
        let open_end = start < units.len();
        if open_end {
            ranges.push((start, units.len()));
        }
        Pieces {
            units,
            ranges,
            open_end,
        }
    }

    pub(super) fn points(units: &'a [u16]) -> Pieces<'a> {
        let mut ranges = Vec::with_capacity(units.len());
        let mut i = 0;
        while i < units.len() {
            let width = 1 + paired(units, i) as usize;
            ranges.push((i, i + width));
            i += width;
        }
        Pieces {
            units,
            ranges,
            open_end: false,
        }
    }

    /// Runs of letters and digits, runs of spaces, each line break, and every other character on
    /// its own.
    pub(super) fn words(units: &'a [u16]) -> Pieces<'a> {
        let mut ranges = Vec::new();
        let mut i = 0;
        while i < units.len() {
            let start = i;
            let (c, width) = char_at(units, i);
            i += width;
            let class = |c: char| {
                if word(c) {
                    1
                } else if space(c) {
                    2
                } else {
                    0
                }
            };
            let kind = class(c);
            let mut joined = c == '\u{200D}';
            while i < units.len() {
                let (next, width) = char_at(units, i);
                let same = kind != 0 && class(next) == kind;
                if !(same || joins(next) || joined) || next == '\n' {
                    break;
                }
                joined = next == '\u{200D}';
                i += width;
            }
            ranges.push((start, i));
        }
        Pieces {
            units,
            ranges,
            open_end: false,
        }
    }

    pub(super) fn len(&self) -> usize {
        self.ranges.len()
    }

    pub(super) fn piece(&self, i: usize) -> &'a [u16] {
        let (start, end) = self.ranges[i];
        &self.units[start..end]
    }

    pub(super) fn span(&self, from: usize, count: usize) -> &'a [u16] {
        if count == 0 {
            return &[];
        }
        &self.units[self.ranges[from].0..self.ranges[from + count - 1].1]
    }

    pub(super) fn unterminated(&self, i: usize) -> bool {
        self.open_end && i + 1 == self.ranges.len()
    }
}

#[derive(Default)]
struct Fx(u64);

impl Hasher for Fx {
    fn write(&mut self, bytes: &[u8]) {
        let mut chunks = bytes.chunks_exact(8);
        for chunk in &mut chunks {
            self.add(u64::from_le_bytes(chunk.try_into().unwrap()));
        }
        let rest = chunks.remainder();
        if !rest.is_empty() {
            let mut last = [0u8; 8];
            last[..rest.len()].copy_from_slice(rest);
            self.add(u64::from_le_bytes(last));
        }
    }

    fn write_u8(&mut self, n: u8) {
        self.add(n as u64);
    }

    fn write_usize(&mut self, n: usize) {
        self.add(n as u64);
    }

    fn finish(&self) -> u64 {
        self.0
    }
}

impl Fx {
    fn add(&mut self, word: u64) {
        self.0 = (self.0.rotate_left(5) ^ word).wrapping_mul(0x51_7c_c1_b7_27_22_0a_95);
    }
}

type FxMap<K, V> = HashMap<K, V, BuildHasherDefault<Fx>>;

fn ascii_space(unit: u16) -> bool {
    matches!(unit, 0x20 | 0x09 | 0x0B | 0x0C | 0x0D)
}

/// The piece as the options compare it.
fn normalized(piece: &[u16], o: &Options, out: &mut Vec<u16>) {
    out.clear();
    let mut piece = piece;
    if o.ignore_line_endings && piece.last() == Some(&RETURN) {
        piece = &piece[..piece.len() - 1];
    }
    match o.whitespace {
        Whitespace::None => out.extend_from_slice(piece),
        Whitespace::Trailing => {
            let end = piece
                .iter()
                .rposition(|u| !ascii_space(*u))
                .map_or(0, |i| i + 1);
            out.extend_from_slice(&piece[..end]);
        }
        Whitespace::Amount => {
            let end = piece
                .iter()
                .rposition(|u| !ascii_space(*u))
                .map_or(0, |i| i + 1);
            let mut in_space = false;
            for unit in &piece[..end] {
                if ascii_space(*unit) {
                    if !in_space {
                        out.push(b' ' as u16);
                    }
                    in_space = true;
                } else {
                    out.push(*unit);
                    in_space = false;
                }
            }
        }
        Whitespace::All => out.extend(piece.iter().filter(|u| !ascii_space(**u))),
    }
    if o.ignore_case {
        let folded: Vec<u16> = char::decode_utf16(out.iter().copied())
            .map(|c| fold(c.unwrap_or(char::REPLACEMENT_CHARACTER)))
            .flat_map(|c| {
                let mut buf = [0u16; 2];
                let n = c.encode_utf16(&mut buf).len();
                buf.into_iter().take(n)
            })
            .collect();
        *out = folded;
    }
}

/// Each piece as a number, equal pieces under equal numbers, and how many numbers there are.
/// A line that ends the text without `\n` is not the same line as one with it.
pub(super) fn intern(sides: &[&Pieces], o: &Options) -> (Vec<Vec<u32>>, usize) {
    if o.plain() {
        let mut ids: FxMap<(&[u16], bool), u32> = FxMap::default();
        let out = sides
            .iter()
            .map(|p| {
                (0..p.len())
                    .map(|i| {
                        let next = ids.len() as u32;
                        *ids.entry((p.piece(i), p.unterminated(i))).or_insert(next)
                    })
                    .collect()
            })
            .collect();
        return (out, ids.len());
    }
    let mut ids: FxMap<(Vec<u16>, bool), u32> = FxMap::default();
    let mut buf = Vec::new();
    let out = sides
        .iter()
        .map(|p| {
            (0..p.len())
                .map(|i| {
                    normalized(p.piece(i), o, &mut buf);
                    let next = ids.len() as u32;
                    *ids.entry((std::mem::take(&mut buf), p.unterminated(i)))
                        .or_insert(next)
                })
                .collect()
        })
        .collect();
    (out, ids.len())
}

/// Which tokens of each side are not in the common subsequence.
struct Marks {
    old: Vec<bool>,
    new: Vec<bool>,
}

impl Marks {
    fn delete(&mut self, at: usize, count: usize) {
        self.old[at..at + count].fill(true);
    }

    fn insert(&mut self, at: usize, count: usize) {
        self.new[at..at + count].fill(true);
    }
}

/// The furthest-reaching paths from both ends meet in the middle of a shortest script (Myers,
/// "An O(ND) Difference Algorithm", the linear-space refinement): `(x, y, u, v)` is that snake.
fn middle_snake(a: &[u32], b: &[u32]) -> (usize, usize, usize, usize) {
    let (n, m) = (a.len() as isize, b.len() as isize);
    let delta = n - m;
    let odd = delta & 1 != 0;
    let max = (n + m + 1) / 2 + 1;
    let off = max + 1;
    let mut forward = vec![0isize; 2 * max as usize + 3];
    let mut backward = vec![0isize; 2 * max as usize + 3];
    let at = |k: isize| (off + k) as usize;
    for d in 0..=max {
        let mut k = -d;
        while k <= d {
            let mut x = if k == -d || (k != d && forward[at(k - 1)] < forward[at(k + 1)]) {
                forward[at(k + 1)]
            } else {
                forward[at(k - 1)] + 1
            };
            let mut y = x - k;
            let (x0, y0) = (x, y);
            while x < n && y < m && a[x as usize] == b[y as usize] {
                x += 1;
                y += 1;
            }
            forward[at(k)] = x;
            if odd && (delta - k).abs() < d && x + backward[at(delta - k)] >= n {
                return (x0 as usize, y0 as usize, x as usize, y as usize);
            }
            k += 2;
        }
        let mut k = -d;
        while k <= d {
            let mut x = if k == -d || (k != d && backward[at(k - 1)] < backward[at(k + 1)]) {
                backward[at(k + 1)]
            } else {
                backward[at(k - 1)] + 1
            };
            let mut y = x - k;
            let (x0, y0) = (x, y);
            while x < n && y < m && a[(n - 1 - x) as usize] == b[(m - 1 - y) as usize] {
                x += 1;
                y += 1;
            }
            backward[at(k)] = x;
            if !odd && (delta - k).abs() <= d && x + forward[at(delta - k)] >= n {
                return (
                    (n - x) as usize,
                    (m - y) as usize,
                    (n - x0) as usize,
                    (m - y0) as usize,
                );
            }
            k += 2;
        }
    }
    unreachable!("two sequences always have an edit script")
}

/// The ranges with their common prefix and suffix taken off, or `None` when one is then empty
/// and the rest is marked.
fn trim(
    a: &[u32],
    b: &[u32],
    (mut alo, mut ahi): (usize, usize),
    (mut blo, mut bhi): (usize, usize),
    marks: &mut Marks,
) -> Option<((usize, usize), (usize, usize))> {
    while alo < ahi && blo < bhi && a[alo] == b[blo] {
        alo += 1;
        blo += 1;
    }
    while alo < ahi && blo < bhi && a[ahi - 1] == b[bhi - 1] {
        ahi -= 1;
        bhi -= 1;
    }
    if alo == ahi || blo == bhi {
        marks.delete(alo, ahi - alo);
        marks.insert(blo, bhi - blo);
        return None;
    }
    Some(((alo, ahi), (blo, bhi)))
}

fn myers(a: &[u32], b: &[u32], old: (usize, usize), new: (usize, usize), marks: &mut Marks) {
    let Some(((alo, ahi), (blo, bhi))) = trim(a, b, old, new, marks) else {
        return;
    };
    let (x, y, u, v) = middle_snake(&a[alo..ahi], &b[blo..bhi]);
    myers(a, b, (alo, alo + x), (blo, blo + y), marks);
    myers(a, b, (alo + u, ahi), (blo + v, bhi), marks);
}

const NONE: u32 = u32::MAX;

/// Per token, its first place in the old range being looked at and how many it has there; per
/// place, the next one with the same token. Emptied after each range.
struct Table {
    first: Vec<u32>,
    count: Vec<u32>,
    next: Vec<u32>,
}

fn histogram(a: &[u32], b: &[u32], tokens: usize, marks: &mut Marks) {
    let mut table = Table {
        first: vec![NONE; tokens],
        count: vec![0; tokens],
        next: vec![NONE; a.len()],
    };
    let mut work = vec![((0, a.len()), (0, b.len()))];
    while let Some((old, new)) = work.pop() {
        let Some(((alo, ahi), (blo, bhi))) = trim(a, b, old, new, marks) else {
            continue;
        };
        for i in (alo..ahi).rev() {
            let t = a[i] as usize;
            table.next[i] = table.first[t];
            table.first[t] = i as u32;
            table.count[t] += 1;
        }
        let count_of = |i: usize| table.count[a[i] as usize] as usize;
        let mut best: Option<(usize, usize, usize, usize)> = None;
        let mut best_count = MAX_CHAIN + 1;
        let mut common = false;
        let mut j = blo;
        while j < bhi {
            let mut next = j + 1;
            let t = b[j] as usize;
            let places = table.count[t] as usize;
            common |= places > 0;
            if places > 0 && places <= MAX_CHAIN && places <= best_count {
                let mut at = table.first[t];
                while at != NONE {
                    let i = at as usize;
                    at = table.next[i];
                    let mut count = places;
                    let (mut start_a, mut start_b) = (i, j);
                    while start_a > alo && start_b > blo && a[start_a - 1] == b[start_b - 1] {
                        start_a -= 1;
                        start_b -= 1;
                        count = count.min(count_of(start_a));
                    }
                    let (mut end_a, mut end_b) = (i + 1, j + 1);
                    while end_a < ahi && end_b < bhi && a[end_a] == b[end_b] {
                        count = count.min(count_of(end_a));
                        end_a += 1;
                        end_b += 1;
                    }
                    let longer = best.is_none_or(|(s, e, _, _)| end_a - start_a > e - s);
                    if count < best_count || (count == best_count && longer) {
                        best = Some((start_a, end_a, start_b, end_b));
                        best_count = count;
                    }
                    next = next.max(end_b);
                }
            }
            j = next;
        }
        for i in alo..ahi {
            let t = a[i] as usize;
            table.first[t] = NONE;
            table.count[t] = 0;
        }
        match best {
            Some((start_a, end_a, start_b, end_b)) => {
                work.push(((alo, start_a), (blo, start_b)));
                work.push(((end_a, ahi), (end_b, bhi)));
            }
            None if !common => {
                marks.delete(alo, ahi - alo);
                marks.insert(blo, bhi - blo);
            }
            None => myers(a, b, (alo, ahi), (blo, bhi), marks),
        }
    }
}

/// The longest increasing run of `values`, as indices into it.
fn longest_increasing(values: &[usize]) -> Vec<usize> {
    let mut tails: Vec<usize> = Vec::new();
    let mut before = vec![usize::MAX; values.len()];
    for (i, v) in values.iter().enumerate() {
        let at = tails.partition_point(|t| values[*t] < *v);
        if at > 0 {
            before[i] = tails[at - 1];
        }
        if at == tails.len() {
            tails.push(i);
        } else {
            tails[at] = i;
        }
    }
    let mut out = Vec::with_capacity(tails.len());
    let mut at = tails.last().copied().unwrap_or(usize::MAX);
    while at != usize::MAX {
        out.push(at);
        at = before[at];
    }
    out.reverse();
    out
}

fn patience(a: &[u32], b: &[u32], tokens: usize, marks: &mut Marks) {
    let mut counts = vec![(0u32, 0u32, 0u32); tokens];
    let mut work = vec![((0, a.len()), (0, b.len()))];
    while let Some((old, new)) = work.pop() {
        let Some(((alo, ahi), (blo, bhi))) = trim(a, b, old, new, marks) else {
            continue;
        };
        for i in alo..ahi {
            counts[a[i] as usize].0 += 1;
        }
        for j in blo..bhi {
            let entry = &mut counts[b[j] as usize];
            entry.1 += 1;
            entry.2 = j as u32;
        }
        let mut pairs = Vec::new();
        for i in alo..ahi {
            let (in_a, in_b, at) = counts[a[i] as usize];
            if in_a == 1 && in_b == 1 {
                pairs.push((i, at as usize));
            }
        }
        for i in alo..ahi {
            counts[a[i] as usize] = (0, 0, 0);
        }
        for j in blo..bhi {
            counts[b[j] as usize] = (0, 0, 0);
        }
        if pairs.is_empty() {
            myers(a, b, (alo, ahi), (blo, bhi), marks);
            continue;
        }
        let anchors = longest_increasing(&pairs.iter().map(|p| p.1).collect::<Vec<_>>());
        let (mut from_a, mut from_b) = (alo, blo);
        for k in anchors {
            let (i, j) = pairs[k];
            work.push(((from_a, i), (from_b, j)));
            (from_a, from_b) = (i + 1, j + 1);
        }
        work.push(((from_a, ahi), (from_b, bhi)));
    }
}

/// A run of marked tokens in one side and the place of its partner run in the other, as git's
/// `xdl_change_compact` walks them.
#[derive(Clone, Copy)]
struct Group {
    start: usize,
    end: usize,
}

fn group_first(changed: &[bool]) -> Group {
    let mut end = 0;
    while end < changed.len() && changed[end] {
        end += 1;
    }
    Group { start: 0, end }
}

fn group_next(changed: &[bool], g: &mut Group) -> bool {
    if g.end == changed.len() {
        return false;
    }
    g.start = g.end + 1;
    g.end = g.start;
    while g.end < changed.len() && changed[g.end] {
        g.end += 1;
    }
    true
}

fn group_previous(changed: &[bool], g: &mut Group) -> bool {
    if g.start == 0 {
        return false;
    }
    g.end = g.start - 1;
    g.start = g.end;
    while g.start > 0 && changed[g.start - 1] {
        g.start -= 1;
    }
    true
}

fn slide_down(tokens: &[u32], changed: &mut [bool], g: &mut Group) -> bool {
    if g.end < tokens.len() && tokens[g.start] == tokens[g.end] {
        changed[g.start] = false;
        changed[g.end] = true;
        g.start += 1;
        g.end += 1;
        while g.end < tokens.len() && changed[g.end] {
            g.end += 1;
        }
        return true;
    }
    false
}

fn slide_up(tokens: &[u32], changed: &mut [bool], g: &mut Group) -> bool {
    if g.start > 0 && tokens[g.start - 1] == tokens[g.end - 1] {
        g.start -= 1;
        g.end -= 1;
        changed[g.start] = true;
        changed[g.end] = false;
        while g.start > 0 && changed[g.start - 1] {
            g.start -= 1;
        }
        return true;
    }
    false
}

/// Moves each run of changes as far down as it goes, unless a place higher up lines it up with a
/// change on the other side; runs that meet merge. The script stays as short.
fn compact(tokens: &[u32], changed: &mut [bool], other: &[bool]) {
    let mut g = group_first(changed);
    let mut go = group_first(other);
    loop {
        if g.end != g.start {
            let mut size;
            let mut matching;
            let mut earliest;
            loop {
                size = g.end - g.start;
                matching = None;
                while slide_up(tokens, changed, &mut g) {
                    group_previous(other, &mut go);
                }
                earliest = g.end;
                if go.end > go.start {
                    matching = Some(g.end);
                }
                while slide_down(tokens, changed, &mut g) {
                    group_next(other, &mut go);
                    if go.end > go.start {
                        matching = Some(g.end);
                    }
                }
                if size == g.end - g.start {
                    break;
                }
            }
            if g.end != earliest && matching.is_some() {
                while go.end == go.start {
                    slide_up(tokens, changed, &mut g);
                    group_previous(other, &mut go);
                }
            }
        }
        if !group_next(changed, &mut g) {
            break;
        }
        group_next(other, &mut go);
    }
}

fn script_of_marks(marks: &Marks) -> Script {
    let (old, new) = (&marks.old, &marks.new);
    let (mut i, mut j) = (0, 0);
    let mut script = Script::default();
    while i < old.len() || j < new.len() {
        let from = i;
        while i < old.len() && old[i] {
            i += 1;
        }
        script.push(Edit::Delete, i - from);
        let from = j;
        while j < new.len() && new[j] {
            j += 1;
        }
        script.push(Edit::Insert, j - from);
        let mut equal = 0;
        while i < old.len() && j < new.len() && !old[i] && !new[j] {
            i += 1;
            j += 1;
            equal += 1;
        }
        script.push(Edit::Equal, equal);
    }
    script
}

/// The script that turns `a` into `b`; `tokens` is one more than the largest token.
pub(super) fn diff(a: &[u32], b: &[u32], tokens: usize, algorithm: Algorithm) -> Script {
    let mut marks = Marks {
        old: vec![false; a.len()],
        new: vec![false; b.len()],
    };
    match algorithm {
        Algorithm::Myers => myers(a, b, (0, a.len()), (0, b.len()), &mut marks),
        Algorithm::Histogram => histogram(a, b, tokens, &mut marks),
        Algorithm::Patience => patience(a, b, tokens, &mut marks),
    }
    compact(a, &mut marks.old, &marks.new);
    compact(b, &mut marks.new, &marks.old);
    script_of_marks(&marks)
}

/// A change as the ranges of pieces it replaces, the end not included.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) struct Change {
    pub(super) old: (usize, usize),
    pub(super) new: (usize, usize),
}

pub(super) fn changes_of(script: &Script) -> Vec<Change> {
    let mut out: Vec<Change> = Vec::new();
    let (mut old, mut new) = (0, 0);
    let mut last_equal = true;
    for (edit, count) in &script.0 {
        match edit {
            Edit::Equal => {
                old += count;
                new += count;
                last_equal = true;
                continue;
            }
            Edit::Delete | Edit::Insert if last_equal => out.push(Change {
                old: (old, old),
                new: (new, new),
            }),
            _ => {}
        }
        last_equal = false;
        let change = out.last_mut().unwrap();
        if *edit == Edit::Delete {
            old += count;
            change.old.1 = old;
        } else {
            new += count;
            change.new.1 = new;
        }
    }
    out
}

pub(super) fn diff_pieces(a: &Pieces, b: &Pieces, o: &Options) -> Script {
    let (ids, tokens) = intern(&[a, b], o);
    diff(&ids[0], &ids[1], tokens, o.algorithm)
}

pub(super) struct Line {
    pub(super) edit: Edit,
    /// Its index in the old text for `Equal` and `Delete`, in the new one for `Insert`.
    pub(super) index: usize,
}

pub(super) struct Hunk {
    pub(super) old_start: usize,
    pub(super) old_count: usize,
    pub(super) new_start: usize,
    pub(super) new_count: usize,
    pub(super) lines: Vec<Line>,
}

/// The changed lines with `context` unchanged ones around them; changes closer than that share a hunk.
pub(super) fn hunks(script: &Script, context: usize) -> Vec<Hunk> {
    let mut lines = Vec::new();
    let (mut old, mut new) = (0, 0);
    for (edit, count) in &script.0 {
        for _ in 0..*count {
            lines.push((*edit, old, new));
            match edit {
                Edit::Equal => (old, new) = (old + 1, new + 1),
                Edit::Delete => old += 1,
                Edit::Insert => new += 1,
            }
        }
    }
    let changed: Vec<usize> = (0..lines.len())
        .filter(|i| lines[*i].0 != Edit::Equal)
        .collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i < changed.len() {
        let mut j = i;
        while j + 1 < changed.len() && changed[j + 1] - changed[j] - 1 <= context.saturating_mul(2)
        {
            j += 1;
        }
        let from = changed[i].saturating_sub(context);
        let to = changed[j]
            .saturating_add(context)
            .saturating_add(1)
            .min(lines.len());
        let part = &lines[from..to];
        out.push(Hunk {
            old_start: part[0].1,
            new_start: part[0].2,
            old_count: part.iter().filter(|l| l.0 != Edit::Insert).count(),
            new_count: part.iter().filter(|l| l.0 != Edit::Delete).count(),
            lines: part
                .iter()
                .map(|(edit, old, new)| Line {
                    edit: *edit,
                    index: if *edit == Edit::Insert { *new } else { *old },
                })
                .collect(),
        });
        i = j + 1;
    }
    out
}

/// Inline refinement is skipped for a change longer than this on either side, in UTF-16 units:
/// past it a highlight inside the lines says little and costs a lot.
const MAX_INLINE: [usize; 2] = [16 * 1024, 4 * 1024];

/// The parts that differ inside a replaced span, as `[oldFrom, oldTo, newFrom, newTo]` in units of
/// the whole texts.
fn inline_of(
    old: &[u16],
    new: &[u16],
    (old_at, new_at): (usize, usize),
    o: &Options,
) -> Vec<[usize; 4]> {
    let chars = o.inline == Inline::Chars;
    if old.len() > MAX_INLINE[chars as usize] || new.len() > MAX_INLINE[chars as usize] {
        return Vec::new();
    }
    let (a, b) = if chars {
        (Pieces::points(old), Pieces::points(new))
    } else {
        (Pieces::words(old), Pieces::words(new))
    };
    let words = Options {
        algorithm: if chars {
            Algorithm::Myers
        } else {
            Algorithm::Histogram
        },
        ignore_line_endings: false,
        ..*o
    };
    let (ids, tokens) = intern(&[&a, &b], &words);
    let script = diff(&ids[0], &ids[1], tokens, words.algorithm);
    changes_of(&script)
        .into_iter()
        .map(|c| {
            let edge = |p: &Pieces, (from, to): (usize, usize)| {
                let start = p.ranges.get(from).map_or(p.units.len(), |r| r.0);
                let end = if to > from { p.ranges[to - 1].1 } else { start };
                (start, end)
            };
            let (old_from, old_to) = edge(&a, c.old);
            let (new_from, new_to) = edge(&b, c.new);
            [
                old_at + old_from,
                old_at + old_to,
                new_at + new_from,
                new_at + new_to,
            ]
        })
        .collect()
}

pub(super) struct LineChange {
    pub(super) change: Change,
    pub(super) inline: Option<Vec<[usize; 4]>>,
}

/// The line changes between two texts, refined inside the lines when the options ask for it.
pub(super) fn line_changes(old: &[u16], new: &[u16], o: &Options) -> Vec<LineChange> {
    let (a, b) = (Pieces::lines(old), Pieces::lines(new));
    let script = diff_pieces(&a, &b, o);
    changes_of(&script)
        .into_iter()
        .map(|change| {
            let inline = (o.inline != Inline::None).then(|| {
                let (os, oe) = change.old;
                let (ns, ne) = change.new;
                if os == oe || ns == ne {
                    return Vec::new();
                }
                let old_at = a.ranges[os].0;
                let new_at = b.ranges[ns].0;
                inline_of(
                    a.span(os, oe - os),
                    b.span(ns, ne - ns),
                    (old_at, new_at),
                    o,
                )
            });
            LineChange { change, inline }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rng(mut seed: u64) -> impl FnMut(u64) -> u64 {
        move |below| {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            seed % below
        }
    }

    fn longest_common(a: &[u32], b: &[u32]) -> usize {
        let mut table = vec![vec![0usize; b.len() + 1]; a.len() + 1];
        for i in 0..a.len() {
            for j in 0..b.len() {
                table[i + 1][j + 1] = if a[i] == b[j] {
                    table[i][j] + 1
                } else {
                    table[i][j + 1].max(table[i + 1][j])
                };
            }
        }
        table[a.len()][b.len()]
    }

    /// The script rebuilt into `b`, and how many tokens it keeps.
    fn replay(a: &[u32], b: &[u32], script: &Script) -> usize {
        let (mut i, mut j, mut equal) = (0, 0, 0);
        let mut rebuilt = Vec::new();
        let mut last = Edit::Equal;
        for (edit, count) in &script.0 {
            assert!(
                !(last == Edit::Insert && *edit == Edit::Delete),
                "a delete after an insert"
            );
            last = *edit;
            match edit {
                Edit::Equal => {
                    assert_eq!(a[i..i + count], b[j..j + count]);
                    rebuilt.extend_from_slice(&a[i..i + count]);
                    (i, j, equal) = (i + count, j + count, equal + count);
                }
                Edit::Delete => i += count,
                Edit::Insert => {
                    rebuilt.extend_from_slice(&b[j..j + count]);
                    j += count;
                }
            }
        }
        assert_eq!((i, rebuilt.as_slice()), (a.len(), b));
        equal
    }

    #[test]
    fn every_algorithm_turns_the_old_into_the_new_and_myers_is_the_shortest() {
        let mut next = rng(0x2545_f491_4f6c_dd1d);
        for round in 0..4000 {
            let letters = 2 + round % 7;
            let a: Vec<u32> = (0..next(40)).map(|_| next(letters) as u32).collect();
            let b: Vec<u32> = (0..next(40)).map(|_| next(letters) as u32).collect();
            let tokens = letters as usize;
            let best = longest_common(&a, &b);
            assert_eq!(
                replay(&a, &b, &diff(&a, &b, tokens, Algorithm::Myers)),
                best,
                "{a:?} -> {b:?}"
            );
            for algorithm in [Algorithm::Histogram, Algorithm::Patience] {
                replay(&a, &b, &diff(&a, &b, tokens, algorithm));
            }
        }
    }

    fn text_lines(script: &Script, a: &[&str], b: &[&str]) -> Vec<String> {
        let (mut i, mut j) = (0, 0);
        let mut out = Vec::new();
        for (edit, count) in &script.0 {
            for _ in 0..*count {
                match edit {
                    Edit::Equal => {
                        out.push(format!(" {}", a[i]));
                        i += 1;
                        j += 1;
                    }
                    Edit::Delete => {
                        out.push(format!("-{}", a[i]));
                        i += 1;
                    }
                    Edit::Insert => {
                        out.push(format!("+{}", b[j]));
                        j += 1;
                    }
                }
            }
        }
        out
    }

    fn line_diff(a: &[&str], b: &[&str], algorithm: Algorithm) -> Vec<String> {
        let mut ids = HashMap::new();
        let mut number = |side: &[&str]| -> Vec<u32> {
            side.iter()
                .map(|l| {
                    let next = ids.len() as u32;
                    *ids.entry(l.to_string()).or_insert(next)
                })
                .collect()
        };
        let (x, y) = (number(a), number(b));
        text_lines(&diff(&x, &y, ids.len(), algorithm), a, b)
    }

    #[test]
    fn histogram_keeps_a_new_function_whole_instead_of_matching_its_braces() {
        let old = [
            "fn a() {",
            "    one();",
            "}",
            "",
            "fn c() {",
            "    three();",
            "}",
        ];
        let new = [
            "fn a() {",
            "    one();",
            "}",
            "",
            "fn b() {",
            "    two();",
            "}",
            "",
            "fn c() {",
            "    three();",
            "}",
        ];
        let lines = line_diff(&old, &new, Algorithm::Histogram);
        let added: Vec<&str> = lines
            .iter()
            .filter(|l| l.starts_with('+'))
            .map(|l| &l[1..])
            .collect();
        assert_eq!(added, ["fn b() {", "    two();", "}", ""]);
    }

    #[test]
    fn a_change_slides_to_line_up_with_its_partner() {
        let lines = line_diff(&["foo", "foo"], &["bar", "foo"], Algorithm::Myers);
        assert_eq!(lines, ["-foo", "+bar", " foo"]);
        let lines = line_diff(&["a", "b", "a", "b"], &["a", "b"], Algorithm::Myers);
        assert_eq!(lines, [" a", " b", "-a", "-b"]);
    }

    fn units(text: &str) -> Vec<u16> {
        text.encode_utf16().collect()
    }

    fn words(text: &str) -> Vec<String> {
        let u = units(text);
        let p = Pieces::words(&u);
        (0..p.len())
            .map(|i| String::from_utf16(p.piece(i)).unwrap())
            .collect()
    }

    #[test]
    fn words_split_at_letters_spaces_and_each_mark() {
        assert_eq!(
            words("let x_1 = f(a,b);  \n\nok"),
            [
                "let", " ", "x_1", " ", "=", " ", "f", "(", "a", ",", "b", ")", ";", "  ", "\n",
                "\n", "ok"
            ]
        );
        assert_eq!(words("café 👍🏽 👨‍👩‍👧!"), ["café", " ", "👍🏽", " ", "👨‍👩‍👧", "!"]);
        assert_eq!(words("e\u{301}x"), ["e\u{301}x"]);
    }

    #[test]
    fn options_decide_which_lines_are_equal() {
        let changes = |a: &str, b: &str, o: Options| line_changes(&units(a), &units(b), &o).len();
        let base = Options::default();
        assert_eq!(changes("a b\r\nc\n", "a  b\nC\n", base), 1);
        let endings = Options {
            ignore_line_endings: true,
            ..base
        };
        assert_eq!(changes("a\r\nb\r\n", "a\nb\n", endings), 0);
        assert_eq!(changes("a\r\nb\r\n", "a\nb\n", base), 1);
        let amount = Options {
            whitespace: Whitespace::Amount,
            ..base
        };
        assert_eq!(changes("a b  \n", "a \t b\n", amount), 0);
        assert_eq!(changes("ab\n", "a b\n", amount), 1);
        let all = Options {
            whitespace: Whitespace::All,
            ..base
        };
        assert_eq!(changes("ab\n", "a b\n", all), 0);
        let trailing = Options {
            whitespace: Whitespace::Trailing,
            ..base
        };
        assert_eq!(changes("a  \n", "a\n", trailing), 0);
        assert_eq!(changes(" a\n", "a\n", trailing), 1);
        let case = Options {
            ignore_case: true,
            ..base
        };
        assert_eq!(changes("Привет\n", "пРИВЕТ\n", case), 0);
        assert_eq!(changes("x", "x\n", case), 1);
    }

    #[test]
    fn inline_changes_are_utf16_offsets_into_the_whole_texts() {
        let old = "same\nlet 😀 = old;\n";
        let new = "same\nlet 😀 = new;\n";
        let o = Options {
            inline: Inline::Words,
            ..Options::default()
        };
        let changes = line_changes(&units(old), &units(new), &o);
        assert_eq!(changes.len(), 1);
        assert_eq!(
            changes[0].change,
            Change {
                old: (1, 2),
                new: (1, 2)
            }
        );
        let inline = changes[0].inline.as_ref().unwrap();
        assert_eq!(inline, &[[14, 17, 14, 17]]);
        let (o16, n16) = (units(old), units(new));
        assert_eq!(String::from_utf16(&o16[14..17]).unwrap(), "old");
        assert_eq!(String::from_utf16(&n16[14..17]).unwrap(), "new");
        let chars = Options {
            inline: Inline::Chars,
            ..Options::default()
        };
        let c = line_changes(&units("ab😀c\n"), &units("ab😁c\n"), &chars);
        assert_eq!(c[0].inline.as_deref().unwrap(), &[[2, 4, 2, 4]]);
    }

    #[test]
    fn fifty_thousand_lines_with_scattered_changes_diff_fast() {
        let mut next = rng(7);
        let old: Vec<String> = (0..50_000)
            .map(|i| {
                format!(
                    "    let value_{i} = compute({}, {});",
                    next(1000),
                    next(1000)
                )
            })
            .collect();
        let mut new = old.clone();
        for _ in 0..500 {
            let at = next(new.len() as u64) as usize;
            match next(3) {
                0 => new[at] = format!("    changed({});", next(100_000)),
                1 => {
                    new.remove(at);
                }
                _ => new.insert(at, format!("    inserted({});", next(100_000))),
            }
        }
        let (old, new) = (
            units(&(old.join("\n") + "\n")),
            units(&(new.join("\n") + "\n")),
        );
        for algorithm in [Algorithm::Histogram, Algorithm::Myers, Algorithm::Patience] {
            let o = Options {
                algorithm,
                inline: Inline::Words,
                ..Options::default()
            };
            let started = std::time::Instant::now();
            let changes = line_changes(&old, &new, &o);
            let took = started.elapsed();
            eprintln!("{algorithm:?}: {} changes in {took:?}", changes.len());
            assert!(changes.len() >= 400);
            let limit = if cfg!(debug_assertions) { 20.0 } else { 1.0 };
            assert!(took.as_secs_f64() < limit, "{algorithm:?} took {took:?}");
        }
    }
}
