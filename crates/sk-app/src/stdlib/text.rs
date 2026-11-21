//! `skathi:text`: building and reading strings, differences, many-word search and glob patterns.
//! Strings stay UTF-16 here, as the engine has them, so an index means what it means in JS.

use std::collections::HashMap;
use std::ffi::c_void;
use std::sync::OnceLock;

use aho_corasick::{AhoCorasick, MatchKind};
use sk_runtime::host;
use sk_runtime::redjs::capi::JSClassRef;
use sk_runtime::redjs::webapi::util::*;

use super::support::{self, *};

/// The longest string the engine makes.
const MAX_UNITS: usize = i32::MAX as usize;
const NEWLINE: u16 = b'\n' as u16;
const RETURN: u16 = b'\r' as u16;

unsafe fn length(g: *mut c_void, array: u64) -> usize {
    to_number(g, get(g, array, "length")).unwrap_or(0.0) as usize
}

unsafe fn units(g: *mut c_void, v: u64, what: &str) -> Js<Vec<u16>> {
    if kind(g, v) != KIND_STRING {
        return type_error(g, &format!("{what} must be a string"));
    }
    read_string16(g, v).ok_or(Thrown)
}

/// `String(v)`: what `+` would append.
unsafe fn shown(g: *mut c_void, v: u64) -> Js<Vec<u16>> {
    if kind(g, v) == KIND_STRING {
        return read_string16(g, v).ok_or(Thrown);
    }
    let text = call(g, get(g, host::global(g), "String"), UNDEF, &[v]).map_err(|_| Thrown)?;
    read_string16(g, text).ok_or(Thrown)
}

unsafe fn list(g: *mut c_void, v: u64, what: &str) -> Js<usize> {
    if !to_bool(
        g,
        call(
            g,
            get(g, get(g, host::global(g), "Array"), "isArray"),
            UNDEF,
            &[v],
        )
        .map_err(|_| Thrown)?,
    ) {
        return type_error(g, &format!("{what} must be an array"));
    }
    Ok(length(g, v))
}

unsafe fn flag(g: *mut c_void, bag: u64, name: &str) -> bool {
    to_bool(g, option(g, bag, name))
}

struct Buffer {
    units: Vec<u16>,
}

static BUILDER: OnceLock<usize> = OnceLock::new();
static WRITER: OnceLock<usize> = OnceLock::new();

fn builder_class() -> JSClassRef {
    host_class::<Buffer>(&BUILDER, "StringBuilder")
}

fn writer_class() -> JSClassRef {
    host_class::<Buffer>(&WRITER, "StringWriter")
}

/// A `StringBuilder` or a `StringWriter`: one buffer under two sets of names.
unsafe fn buffer<'a>(g: *mut c_void, this: u64) -> Js<&'a mut Buffer> {
    if let Some(state) = host_data::<Buffer>(g, builder_class(), this) {
        return Ok(state);
    }
    match host_data::<Buffer>(g, writer_class(), this) {
        Some(state) => Ok(state),
        None => type_error(g, "this is not a StringBuilder or a StringWriter"),
    }
}

unsafe fn grow(g: *mut c_void, state: &Buffer, by: usize) -> Js<()> {
    if state.units.len() + by > MAX_UNITS {
        return range_error(
            g,
            "the text would be longer than a string can be (2^31 - 1)",
        );
    }
    Ok(())
}

unsafe fn push(g: *mut c_void, this: u64, v: u64) -> Js<()> {
    let text = shown(g, v)?;
    let state = buffer(g, this)?;
    grow(g, state, text.len())?;
    state.units.extend_from_slice(&text);
    Ok(())
}

unsafe fn push_all(g: *mut c_void, this: u64, parts: u64, separator: u64, what: &str) -> Js<()> {
    let count = list(g, parts, &format!("{what}: the parts"))?;
    let separator = if separator == UNDEF {
        Vec::new()
    } else {
        units(g, separator, &format!("{what}: the separator"))?
    };
    let mut joined = Vec::new();
    for i in 0..count {
        if i > 0 {
            joined.extend_from_slice(&separator);
        }
        joined.extend_from_slice(&shown(g, get_i(g, parts, i as u32))?);
    }
    let state = buffer(g, this)?;
    grow(g, state, joined.len())?;
    state.units.extend_from_slice(&joined);
    Ok(())
}

std_fn!(fn builder_construct(g, args) {
    let initial = args.arg(0).0;
    let units = if initial == UNDEF { Vec::new() } else { units(g, initial, "StringBuilder: the initial text")? };
    Ok(host_create(g, builder_class(), prototype(g, "StringBuilder"), Buffer { units }))
});

std_fn!(fn writer_construct(g, _args) {
    Ok(host_create(g, writer_class(), prototype(g, "StringWriter"), Buffer { units: Vec::new() }))
});

std_fn!(fn buffer_append(g, args) {
    let this = args.this().0;
    push(g, this, args.arg(0).0)?;
    Ok(this)
});

std_fn!(fn buffer_append_line(g, args) {
    let this = args.this().0;
    if args.arg(0).0 != UNDEF {
        push(g, this, args.arg(0).0)?;
    }
    let state = buffer(g, this)?;
    grow(g, state, 1)?;
    state.units.push(NEWLINE);
    Ok(this)
});

std_fn!(fn buffer_append_all(g, args) {
    let this = args.this().0;
    push_all(g, this, args.arg(0).0, args.arg(1).0, "appendAll")?;
    Ok(this)
});

std_fn!(fn buffer_insert(g, args) {
    let this = args.this().0;
    let at = count(g, args.arg(0).0, "StringBuilder.insert: the index")?;
    let text = units(g, args.arg(1).0, "StringBuilder.insert: the text")?;
    let state = buffer(g, this)?;
    if at > state.units.len() {
        return range_error(g, &format!("StringBuilder.insert: the index {at} is past the length {}", state.units.len()));
    }
    grow(g, state, text.len())?;
    state.units.splice(at..at, text);
    Ok(this)
});

std_fn!(fn buffer_clear(g, args) {
    buffer(g, args.this().0)?.units.clear();
    Ok(UNDEF)
});

std_fn!(fn buffer_length(g, args) { Ok(enc_f64(buffer(g, args.this().0)?.units.len() as f64)) });
std_fn!(fn buffer_to_string(g, args) { Ok(js_string16(g, &buffer(g, args.this().0)?.units)) });

std_fn!(fn writer_write(g, args) {
    push(g, args.this().0, args.arg(0).0)?;
    Ok(UNDEF)
});

std_fn!(fn writer_write_line(g, args) {
    let this = args.this().0;
    if args.arg(0).0 != UNDEF {
        push(g, this, args.arg(0).0)?;
    }
    let state = buffer(g, this)?;
    grow(g, state, 1)?;
    state.units.push(NEWLINE);
    Ok(UNDEF)
});

std_fn!(fn writer_write_all(g, args) {
    push_all(g, args.this().0, args.arg(0).0, args.arg(1).0, "writeAll")?;
    Ok(UNDEF)
});

struct Reader {
    units: Vec<u16>,
    at: usize,
}

host_state!(reader, reader_new, Reader, READER, "StringReader");

impl Reader {
    /// The code point at the position: two units for a surrogate pair, one otherwise.
    fn point(&self) -> Option<&[u16]> {
        let first = *self.units.get(self.at)?;
        let paired = (0xD800..0xDC00).contains(&first)
            && self
                .units
                .get(self.at + 1)
                .is_some_and(|low| (0xDC00..0xE000).contains(low));
        Some(&self.units[self.at..self.at + 1 + paired as usize])
    }

    /// The line at the position without its `\n`, `\r\n` or `\r`.
    fn line(&mut self) -> Option<&[u16]> {
        if self.at >= self.units.len() {
            return None;
        }
        let start = self.at;
        let rest = &self.units[start..];
        let len = rest
            .iter()
            .position(|u| *u == NEWLINE || *u == RETURN)
            .unwrap_or(rest.len());
        let mut next = start + len;
        if rest.get(len) == Some(&RETURN) && rest.get(len + 1) == Some(&NEWLINE) {
            next += 2;
        } else if len < rest.len() {
            next += 1;
        }
        self.at = next;
        Some(&self.units[start..start + len])
    }
}

std_fn!(fn reader_construct(g, args) {
    let units = units(g, args.arg(0).0, "StringReader: the text")?;
    Ok(reader_new(g, Reader { units, at: 0 }))
});

std_fn!(fn reader_peek(g, args) {
    Ok(reader(g, args.this().0)?.point().map_or(UNDEF, |point| js_string16(g, point)))
});

std_fn!(fn reader_read(g, args) {
    let state = reader(g, args.this().0)?;
    let Some(point) = state.point() else {
        return Ok(UNDEF);
    };
    let value = js_string16(g, point);
    let taken = point.len();
    state.at += taken;
    Ok(value)
});

std_fn!(fn reader_read_line(g, args) {
    Ok(reader(g, args.this().0)?.line().map_or(NULL, |line| js_string16(g, line)))
});

std_fn!(fn reader_read_lines(g, args) {
    let this = args.this().0;
    let max = if args.arg(0).0 == UNDEF { usize::MAX } else { count(g, args.arg(0).0, "StringReader.readLines: the limit")? };
    let lines = new_array(g);
    let mut taken = 0;
    while taken < max {
        let Some(line) = reader(g, this)?.line().map(<[u16]>::to_vec) else {
            break;
        };
        set_i(g, lines, taken as u32, js_string16(g, &line));
        taken += 1;
    }
    Ok(lines)
});

std_fn!(fn reader_read_to_end(g, args) {
    let state = reader(g, args.this().0)?;
    let rest = js_string16(g, &state.units[state.at..]);
    state.at = state.units.len();
    Ok(rest)
});

std_fn!(fn reader_position(g, args) { Ok(enc_f64(reader(g, args.this().0)?.at as f64)) });
std_fn!(fn reader_length(g, args) { Ok(enc_f64(reader(g, args.this().0)?.units.len() as f64)) });

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Edit {
    Equal,
    Delete,
    Insert,
}

impl Edit {
    fn name(self) -> &'static str {
        match self {
            Edit::Equal => "equal",
            Edit::Delete => "delete",
            Edit::Insert => "insert",
        }
    }
}

/// Runs of one kind of edit, in order.
#[derive(Default)]
struct Script(Vec<(Edit, usize)>);

impl Script {
    fn push(&mut self, edit: Edit, count: usize) {
        if count == 0 {
            return;
        }
        match self.0.last_mut() {
            Some((last, run)) if *last == edit => *run += count,
            _ => self.0.push((edit, count)),
        }
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

fn shortest_script(a: &[u32], b: &[u32], script: &mut Script) {
    let prefix = a.iter().zip(b).take_while(|(x, y)| x == y).count();
    let (a, b) = (&a[prefix..], &b[prefix..]);
    let suffix = a
        .iter()
        .rev()
        .zip(b.iter().rev())
        .take_while(|(x, y)| x == y)
        .count();
    let (a, b) = (&a[..a.len() - suffix], &b[..b.len() - suffix]);
    script.push(Edit::Equal, prefix);
    if a.is_empty() || b.is_empty() {
        script.push(Edit::Delete, a.len());
        script.push(Edit::Insert, b.len());
    } else {
        let (x, y, u, v) = middle_snake(a, b);
        shortest_script(&a[..x], &b[..y], script);
        script.push(Edit::Equal, u - x);
        shortest_script(&a[u..], &b[v..], script);
    }
    script.push(Edit::Equal, suffix);
}

/// A text as the pieces a difference is made of, each a range of its units.
struct Pieces<'a> {
    units: &'a [u16],
    ranges: Vec<(usize, usize)>,
    /// The last line has no `\n` after it.
    open_end: bool,
}

impl<'a> Pieces<'a> {
    fn lines(units: &'a [u16]) -> Pieces<'a> {
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

    fn points(units: &'a [u16]) -> Pieces<'a> {
        let mut ranges = Vec::with_capacity(units.len());
        let mut i = 0;
        while i < units.len() {
            let paired = (0xD800..0xDC00).contains(&units[i])
                && units
                    .get(i + 1)
                    .is_some_and(|low| (0xDC00..0xE000).contains(low));
            ranges.push((i, i + 1 + paired as usize));
            i += 1 + paired as usize;
        }
        Pieces {
            units,
            ranges,
            open_end: false,
        }
    }

    fn piece(&self, i: usize) -> &'a [u16] {
        let (start, end) = self.ranges[i];
        &self.units[start..end]
    }

    fn span(&self, from: usize, count: usize) -> &'a [u16] {
        if count == 0 {
            return &[];
        }
        &self.units[self.ranges[from].0..self.ranges[from + count - 1].1]
    }

    fn unterminated(&self, i: usize) -> bool {
        self.open_end && i + 1 == self.ranges.len()
    }
}

/// A line that ends the text without `\n` is not the same line as one with it.
fn script_of<'a>(a: &Pieces<'a>, b: &Pieces<'a>) -> Script {
    fn number<'a>(pieces: &Pieces<'a>, ids: &mut HashMap<(&'a [u16], bool), u32>) -> Vec<u32> {
        (0..pieces.ranges.len())
            .map(|i| {
                let next = ids.len() as u32;
                *ids.entry((pieces.piece(i), pieces.unterminated(i)))
                    .or_insert(next)
            })
            .collect()
    }
    let mut ids = HashMap::new();
    let (left, right) = (number(a, &mut ids), number(b, &mut ids));
    let mut script = Script::default();
    shortest_script(&left, &right, &mut script);
    script
}

struct Line {
    edit: Edit,
    /// Its index in the old text for `Equal` and `Delete`, in the new one for `Insert`.
    index: usize,
}

struct Hunk {
    old_start: usize,
    old_count: usize,
    new_start: usize,
    new_count: usize,
    lines: Vec<Line>,
}

/// The changed lines with `context` unchanged ones around them; changes closer than that share a hunk.
fn hunks(script: &Script, context: usize) -> Vec<Hunk> {
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

unsafe fn context_of(g: *mut c_void, bag: u64, what: &str) -> Js<usize> {
    let given = option(g, bag, "context");
    if given == UNDEF {
        return Ok(3);
    }
    count(g, given, &format!("{what}: context"))
}

unsafe fn two_texts(g: *mut c_void, old: u64, new: u64, what: &str) -> Js<(Vec<u16>, Vec<u16>)> {
    Ok((
        units(g, old, &format!("{what}: the old text"))?,
        units(g, new, &format!("{what}: the new text"))?,
    ))
}

std_fn!(fn diff_lines(g, args) {
    let (old, new) = two_texts(g, args.arg(0).0, args.arg(1).0, "TextDiff.lines")?;
    let context = context_of(g, args.arg(2).0, "TextDiff.lines")?;
    let (a, b) = (Pieces::lines(&old), Pieces::lines(&new));
    let result = new_array(g);
    for (i, hunk) in hunks(&script_of(&a, &b), context).iter().enumerate() {
        let lines = new_array(g);
        let object = new_object(g, UNDEF);
        define(g, object, "oldStart", enc_f64(hunk.old_start as f64 + 1.0), 0);
        define(g, object, "oldLines", enc_f64(hunk.old_count as f64), 0);
        define(g, object, "newStart", enc_f64(hunk.new_start as f64 + 1.0), 0);
        define(g, object, "newLines", enc_f64(hunk.new_count as f64), 0);
        define(g, object, "lines", lines, 0);
        set_i(g, result, i as u32, object);
        for (j, line) in hunk.lines.iter().enumerate() {
            let side = if line.edit == Edit::Insert { &b } else { &a };
            let entry = new_object(g, UNDEF);
            define(g, entry, "op", js_string(g, line.edit.name()), 0);
            define(g, entry, "text", js_string16(g, side.piece(line.index)), 0);
            set_i(g, lines, j as u32, entry);
        }
    }
    Ok(result)
});

std_fn!(fn diff_chars(g, args) {
    let (old, new) = two_texts(g, args.arg(0).0, args.arg(1).0, "TextDiff.chars")?;
    let (a, b) = (Pieces::points(&old), Pieces::points(&new));
    let result = new_array(g);
    let (mut at_old, mut at_new) = (0, 0);
    for (i, (edit, count)) in script_of(&a, &b).0.iter().enumerate() {
        let text = if *edit == Edit::Insert { b.span(at_new, *count) } else { a.span(at_old, *count) };
        let entry = new_object(g, UNDEF);
        define(g, entry, "op", js_string(g, edit.name()), 0);
        define(g, entry, "text", js_string16(g, text), 0);
        set_i(g, result, i as u32, entry);
        if *edit != Edit::Insert {
            at_old += count;
        }
        if *edit != Edit::Delete {
            at_new += count;
        }
    }
    Ok(result)
});

fn unified(a: &Pieces, b: &Pieces, context: usize, names: Option<(&[u16], &[u16])>) -> Vec<u16> {
    let hunks = hunks(&script_of(a, b), context);
    let mut out: Vec<u16> = Vec::new();
    let ascii = |out: &mut Vec<u16>, text: &str| out.extend(text.encode_utf16());
    if hunks.is_empty() {
        return out;
    }
    if let Some((old, new)) = names {
        ascii(&mut out, "--- ");
        out.extend_from_slice(old);
        ascii(&mut out, "\n+++ ");
        out.extend_from_slice(new);
        ascii(&mut out, "\n");
    }
    // An empty range is named by the line before it.
    let range = |start: usize, count: usize| {
        format!("{},{count}", if count == 0 { start } else { start + 1 })
    };
    for hunk in &hunks {
        ascii(
            &mut out,
            &format!(
                "@@ -{} +{} @@\n",
                range(hunk.old_start, hunk.old_count),
                range(hunk.new_start, hunk.new_count)
            ),
        );
        for line in &hunk.lines {
            let (mark, side) = match line.edit {
                Edit::Equal => (" ", a),
                Edit::Delete => ("-", a),
                Edit::Insert => ("+", b),
            };
            ascii(&mut out, mark);
            out.extend_from_slice(side.piece(line.index));
            ascii(&mut out, "\n");
            if side.unterminated(line.index) {
                ascii(&mut out, "\\ No newline at end of file\n");
            }
        }
    }
    out
}

std_fn!(fn diff_unified(g, args) {
    let (old, new) = two_texts(g, args.arg(0).0, args.arg(1).0, "TextDiff.unified")?;
    let bag = args.arg(2).0;
    let context = context_of(g, bag, "TextDiff.unified")?;
    let (old_name, new_name) = (option(g, bag, "oldName"), option(g, bag, "newName"));
    let names = if old_name == UNDEF && new_name == UNDEF {
        None
    } else {
        Some((
            units(g, old_name, "TextDiff.unified: oldName")?,
            units(g, new_name, "TextDiff.unified: newName")?,
        ))
    };
    let text = unified(
        &Pieces::lines(&old),
        &Pieces::lines(&new),
        context,
        names.as_ref().map(|(o, n)| (o.as_slice(), n.as_slice())),
    );
    Ok(js_string16(g, &text))
});

/// One character for one character, so that a position in the folded text is a position in the original.
fn fold(c: char) -> char {
    let mut lower = c.to_lowercase();
    match (lower.next(), lower.next()) {
        (Some(one), None) => one,
        _ => c,
    }
}

fn word(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

struct Search {
    automaton: AhoCorasick,
    ignore_case: bool,
    whole_word: bool,
}

host_state!(search, search_new, Search, SEARCH, "TextSearch");

/// The text as the automaton reads it, and for each of its characters the UTF-8 length here and
/// the UTF-16 length in the original, packed in a byte.
struct Haystack {
    text: String,
    widths: Vec<u8>,
}

impl Haystack {
    fn new(units: &[u16], ignore_case: bool) -> Haystack {
        let mut text = String::with_capacity(units.len());
        let mut widths = Vec::with_capacity(units.len());
        for decoded in char::decode_utf16(units.iter().copied()) {
            let original = decoded.unwrap_or(char::REPLACEMENT_CHARACTER);
            let wide = original.len_utf16();
            let c = if ignore_case {
                fold(original)
            } else {
                original
            };
            text.push(c);
            widths.push(c.len_utf8() as u8 | (wide as u8) << 4);
        }
        Haystack { text, widths }
    }

    fn bounded(&self, start: usize, end: usize) -> bool {
        !self.text[..start].chars().next_back().is_some_and(word)
            && !self.text[end..].chars().next().is_some_and(word)
    }
}

/// Byte offsets into a `Haystack`, asked for in ascending order, as offsets in UTF-16 units.
struct Offsets<'a> {
    widths: std::slice::Iter<'a, u8>,
    byte: usize,
    unit: usize,
}

impl Offsets<'_> {
    fn unit_at(&mut self, byte: usize) -> usize {
        while self.byte < byte {
            let Some(width) = self.widths.next() else {
                break;
            };
            self.byte += (width & 15) as usize;
            self.unit += (width >> 4) as usize;
        }
        self.unit
    }
}

impl Search {
    /// Matches as `(start, end, pattern)` in bytes of the haystack, leftmost first and the longest
    /// of those that start together, none overlapping another.
    fn matches(&self, haystack: &Haystack) -> Vec<(usize, usize, usize)> {
        if !self.whole_word {
            return self
                .automaton
                .find_iter(&haystack.text)
                .map(|m| (m.start(), m.end(), m.pattern().as_usize()))
                .collect();
        }
        let mut found: Vec<(usize, usize, usize)> = self
            .automaton
            .find_overlapping_iter(&haystack.text)
            .map(|m| (m.start(), m.end(), m.pattern().as_usize()))
            .filter(|(start, end, _)| haystack.bounded(*start, *end))
            .collect();
        found.sort_by(|a, b| a.0.cmp(&b.0).then(b.1.cmp(&a.1)).then(a.2.cmp(&b.2)));
        let mut end = 0;
        found.retain(|m| {
            let keep = m.0 >= end;
            if keep {
                end = m.1;
            }
            keep
        });
        found
    }
}

std_fn!(fn search_construct(g, args) {
    let given = args.arg(0).0;
    let what = "TextSearch: the patterns";
    let count = list(g, given, what)?;
    let bag = args.arg(1).0;
    let (ignore_case, whole_word) = (flag(g, bag, "ignoreCase"), flag(g, bag, "wholeWord"));
    let mut patterns = Vec::with_capacity(count);
    for i in 0..count {
        let pattern = string(g, get_i(g, given, i as u32), &format!("{what}: pattern {i}"))?;
        if pattern.is_empty() {
            return type_error(g, &format!("{what}: pattern {i} is empty, which would match everywhere"));
        }
        patterns.push(if ignore_case { pattern.chars().map(fold).collect() } else { pattern });
    }
    // An overlapping search, which whole words need, is only made by the standard kind.
    let match_kind = if whole_word { MatchKind::Standard } else { MatchKind::LeftmostLongest };
    match AhoCorasick::builder().match_kind(match_kind).build(&patterns) {
        Ok(automaton) => Ok(search_new(g, Search { automaton, ignore_case, whole_word })),
        Err(error) => range_error(g, &format!("{what}: {error}")),
    }
});

std_fn!(fn search_find_all(g, args) {
    let state = search(g, args.this().0)?;
    let text = units(g, args.arg(0).0, "TextSearch.findAll: the text")?;
    let haystack = Haystack::new(&text, state.ignore_case);
    let mut offsets = Offsets { widths: haystack.widths.iter(), byte: 0, unit: 0 };
    let result = new_array(g);
    for (i, (start, end, pattern)) in state.matches(&haystack).into_iter().enumerate() {
        let index = offsets.unit_at(start);
        let length = offsets.unit_at(end) - index;
        let entry = new_object(g, UNDEF);
        define(g, entry, "index", enc_f64(index as f64), 0);
        define(g, entry, "length", enc_f64(length as f64), 0);
        define(g, entry, "pattern", enc_f64(pattern as f64), 0);
        set_i(g, result, i as u32, entry);
    }
    Ok(result)
});

std_fn!(fn search_test(g, args) {
    let state = search(g, args.this().0)?;
    let text = units(g, args.arg(0).0, "TextSearch.test: the text")?;
    let haystack = Haystack::new(&text, state.ignore_case);
    if state.whole_word {
        return Ok(enc_bool(!state.matches(&haystack).is_empty()));
    }
    Ok(enc_bool(state.automaton.is_match(&haystack.text)))
});

#[derive(Debug, PartialEq)]
enum Token {
    Char(char),
    /// `?`
    One,
    /// `*`
    Segment,
    /// `**` as a whole segment; `through` when a `/` followed it and was taken with it.
    Segments {
        through: bool,
    },
    Class {
        negated: bool,
        ranges: Vec<(char, char)>,
    },
    Separator,
}

const MAX_ALTERNATIVES: usize = 1024;

/// `a{b,c}d` as `abd` and `acd`. A brace without a comma, or without its pair, is itself.
fn braces(pattern: &[char], out: &mut Vec<Vec<char>>) -> Result<(), String> {
    let mut i = 0;
    while i < pattern.len() {
        match pattern[i] {
            '\\' => i += 1,
            '[' => {
                if let Some(end) = class_end(pattern, i) {
                    i = end;
                }
            }
            '{' => {
                let (mut depth, mut commas, mut j) = (0, vec![], i);
                let mut close = None;
                while j < pattern.len() {
                    match pattern[j] {
                        '\\' => j += 1,
                        '{' => depth += 1,
                        ',' if depth == 1 => commas.push(j),
                        '}' => {
                            depth -= 1;
                            if depth == 0 {
                                close = Some(j);
                                break;
                            }
                        }
                        _ => {}
                    }
                    j += 1;
                }
                if let (Some(close), false) = (close, commas.is_empty()) {
                    let mut starts = vec![i + 1];
                    starts.extend(commas.iter().map(|c| c + 1));
                    let mut ends = commas.clone();
                    ends.push(close);
                    for (start, end) in starts.into_iter().zip(ends) {
                        let mut one = pattern[..i].to_vec();
                        one.extend_from_slice(&pattern[start..end]);
                        one.extend_from_slice(&pattern[close + 1..]);
                        braces(&one, out)?;
                    }
                    return Ok(());
                }
            }
            _ => {}
        }
        i += 1;
    }
    if out.len() == MAX_ALTERNATIVES {
        return Err(format!(
            "the braces expand to more than {MAX_ALTERNATIVES} patterns"
        ));
    }
    out.push(pattern.to_vec());
    Ok(())
}

/// The index of the `]` that closes the class opened at `open`.
fn class_end(pattern: &[char], open: usize) -> Option<usize> {
    let mut i = open + 1;
    if matches!(pattern.get(i), Some('!' | '^')) {
        i += 1;
    }
    // A `]` right after the opening is a member.
    if pattern.get(i) == Some(&']') {
        i += 1;
    }
    while i < pattern.len() {
        match pattern[i] {
            '\\' => i += 1,
            ']' => return Some(i),
            _ => {}
        }
        i += 1;
    }
    None
}

fn tokens(pattern: &[char]) -> Vec<Token> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < pattern.len() {
        match pattern[i] {
            '\\' if i + 1 < pattern.len() => {
                out.push(Token::Char(pattern[i + 1]));
                i += 1;
            }
            '?' => out.push(Token::One),
            '/' => out.push(Token::Separator),
            '*' => {
                let stars = pattern[i..].iter().take_while(|c| **c == '*').count();
                let after = pattern.get(i + stars);
                let whole = (i == 0 || pattern[i - 1] == '/') && matches!(after, None | Some('/'));
                if stars >= 2 && whole {
                    let through = after.is_some();
                    out.push(Token::Segments { through });
                    i += stars - 1 + through as usize;
                } else {
                    out.push(Token::Segment);
                    i += stars - 1;
                }
            }
            '[' => match class_end(pattern, i) {
                Some(end) => {
                    let mut at = i + 1;
                    let negated = matches!(pattern[at], '!' | '^');
                    at += negated as usize;
                    let mut ranges = Vec::new();
                    while at < end {
                        if pattern[at] == '\\' && at + 1 < end {
                            at += 1;
                        }
                        let low = pattern[at];
                        if at + 2 < end && pattern[at + 1] == '-' {
                            ranges.push((low, pattern[at + 2]));
                            at += 3;
                        } else {
                            ranges.push((low, low));
                            at += 1;
                        }
                    }
                    out.push(Token::Class { negated, ranges });
                    i = end;
                }
                None => out.push(Token::Char('[')),
            },
            c => out.push(Token::Char(c)),
        }
        i += 1;
    }
    out
}

struct Glob {
    alternatives: Vec<Vec<Token>>,
    negated: bool,
    ignore_case: bool,
}

host_state!(glob, glob_new, Glob, GLOB, "Glob");

impl Glob {
    fn new(pattern: &str, ignore_case: bool) -> Result<Glob, String> {
        let mut chars: Vec<char> = pattern.chars().collect();
        let marks = chars.iter().take_while(|c| **c == '!').count();
        chars.drain(..marks);
        if ignore_case {
            chars = chars.into_iter().map(fold).collect();
        }
        let mut expanded = Vec::new();
        braces(&chars, &mut expanded)?;
        Ok(Glob {
            alternatives: expanded.iter().map(|p| tokens(p)).collect(),
            negated: marks % 2 == 1,
            ignore_case,
        })
    }

    fn matches(&self, path: &str) -> bool {
        let path: Vec<char> = path
            .chars()
            .map(|c| if cfg!(windows) && c == '\\' { '/' } else { c })
            .map(|c| if self.ignore_case { fold(c) } else { c })
            .collect();
        let hit = self.alternatives.iter().any(|tokens| {
            let mut failed = vec![false; (tokens.len() + 1) * (path.len() + 1)];
            matches_from(tokens, &path, 0, 0, &mut failed)
        });
        hit != self.negated
    }
}

/// `failed` remembers the pairs that did not match, so that stars do not retry them.
fn matches_from(tokens: &[Token], path: &[char], t: usize, p: usize, failed: &mut [bool]) -> bool {
    let key = t * (path.len() + 1) + p;
    if failed[key] {
        return false;
    }
    let Some(token) = tokens.get(t) else {
        return p == path.len();
    };
    let here = path.get(p).copied();
    let matched = match token {
        Token::Char(c) => here == Some(*c) && matches_from(tokens, path, t + 1, p + 1, failed),
        Token::Separator => here == Some('/') && matches_from(tokens, path, t + 1, p + 1, failed),
        Token::One => {
            here.is_some_and(|c| c != '/') && matches_from(tokens, path, t + 1, p + 1, failed)
        }
        Token::Class { negated, ranges } => {
            here.is_some_and(|c| {
                c != '/' && ranges.iter().any(|(low, high)| (*low..=*high).contains(&c)) != *negated
            }) && matches_from(tokens, path, t + 1, p + 1, failed)
        }
        Token::Segment => {
            let reach = path[p..]
                .iter()
                .position(|c| *c == '/')
                .unwrap_or(path.len() - p);
            (0..=reach).any(|take| matches_from(tokens, path, t + 1, p + take, failed))
        }
        Token::Segments { through: false } => {
            (p..=path.len()).any(|next| matches_from(tokens, path, t + 1, next, failed))
        }
        Token::Segments { through: true } => {
            matches_from(tokens, path, t + 1, p, failed)
                || (p..path.len())
                    .any(|i| path[i] == '/' && matches_from(tokens, path, t + 1, i + 1, failed))
        }
    };
    if !matched {
        failed[key] = true;
    }
    matched
}

std_fn!(fn glob_construct(g, args) {
    let pattern = string(g, args.arg(0).0, "Glob: the pattern")?;
    let sensitive = option(g, args.arg(1).0, "caseSensitive");
    let ignore_case = sensitive != UNDEF && !to_bool(g, sensitive);
    match Glob::new(&pattern, ignore_case) {
        Ok(state) => Ok(glob_new(g, state)),
        Err(error) => range_error(g, &format!("Glob: {error}")),
    }
});

std_fn!(fn glob_match(g, args) {
    let path = string(g, args.arg(0).0, "Glob.match: the path")?;
    Ok(enc_bool(glob(g, args.this().0)?.matches(&path)))
});

std_fn!(fn glob_filter(g, args) {
    let this = args.this().0;
    let paths = args.arg(0).0;
    let count = list(g, paths, "Glob.filter: the paths")?;
    let result = new_array(g);
    let mut kept = 0;
    for i in 0..count {
        let value = get_i(g, paths, i as u32);
        let path = string(g, value, &format!("Glob.filter: path {i}"))?;
        if glob(g, this)?.matches(&path) {
            set_i(g, result, kept, value);
            kept += 1;
        }
    }
    Ok(result)
});

pub(super) unsafe fn install(g: *mut c_void, std: u64) {
    let module = new_object(g, UNDEF);

    let builder = support::class(
        g,
        &ClassSpec {
            name: "StringBuilder",
            arity: 0,
            construct: Some(builder_construct),
            parent_proto: 0,
            methods: &[
                ("append", 1, buffer_append),
                ("appendLine", 0, buffer_append_line),
                ("appendAll", 1, buffer_append_all),
                ("insert", 2, buffer_insert),
                ("clear", 0, buffer_clear),
                ("toString", 0, buffer_to_string),
            ],
            getters: &[("length", buffer_length)],
            accessors: &[],
        },
    );
    define(g, module, "StringBuilder", builder.ctor, 0);

    let writer = support::class(
        g,
        &ClassSpec {
            name: "StringWriter",
            arity: 0,
            construct: Some(writer_construct),
            parent_proto: 0,
            methods: &[
                ("write", 1, writer_write),
                ("writeLine", 0, writer_write_line),
                ("writeAll", 1, writer_write_all),
                ("toString", 0, buffer_to_string),
            ],
            getters: &[("length", buffer_length)],
            accessors: &[],
        },
    );
    define(g, module, "StringWriter", writer.ctor, 0);

    let reader = support::class(
        g,
        &ClassSpec {
            name: "StringReader",
            arity: 1,
            construct: Some(reader_construct),
            parent_proto: 0,
            methods: &[
                ("read", 0, reader_read),
                ("peek", 0, reader_peek),
                ("readLine", 0, reader_read_line),
                ("readLines", 0, reader_read_lines),
                ("readToEnd", 0, reader_read_to_end),
            ],
            getters: &[("position", reader_position), ("length", reader_length)],
            accessors: &[],
        },
    );
    define(g, module, "StringReader", reader.ctor, 0);

    define(
        g,
        module,
        "TextDiff",
        namespace(
            g,
            "TextDiff",
            &[
                ("lines", 2, diff_lines),
                ("chars", 2, diff_chars),
                ("unified", 2, diff_unified),
            ],
        ),
        0,
    );

    let search = support::class(
        g,
        &ClassSpec {
            name: "TextSearch",
            arity: 1,
            construct: Some(search_construct),
            parent_proto: 0,
            methods: &[("findAll", 1, search_find_all), ("test", 1, search_test)],
            getters: &[],
            accessors: &[],
        },
    );
    define(g, module, "TextSearch", search.ctor, 0);

    let glob = support::class(
        g,
        &ClassSpec {
            name: "Glob",
            arity: 1,
            construct: Some(glob_construct),
            parent_proto: 0,
            methods: &[("match", 1, glob_match), ("filter", 1, glob_filter)],
            getters: &[],
            accessors: &[],
        },
    );
    define(g, module, "Glob", glob.ctor, 0);

    define(g, std, "text", module, 0);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn script(a: &[u32], b: &[u32]) -> Vec<(Edit, usize)> {
        let mut script = Script::default();
        shortest_script(a, b, &mut script);
        script.0
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

    #[test]
    fn a_script_turns_the_old_into_the_new_and_is_the_shortest() {
        let mut seed = 0x2545_f491_4f6c_dd1du64;
        let mut next = |below: u64| {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            seed % below
        };
        for round in 0..4000 {
            let letters = 2 + round % 5;
            let a: Vec<u32> = (0..next(40)).map(|_| next(letters) as u32).collect();
            let b: Vec<u32> = (0..next(40)).map(|_| next(letters) as u32).collect();
            let (mut i, mut j, mut equal) = (0, 0, 0);
            let mut rebuilt = Vec::new();
            for (edit, count) in script(&a, &b) {
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
            assert_eq!((i, rebuilt.as_slice()), (a.len(), b.as_slice()));
            assert_eq!(equal, longest_common(&a, &b), "{a:?} -> {b:?}");
        }
    }

    fn units(text: &str) -> Vec<u16> {
        text.encode_utf16().collect()
    }

    #[test]
    fn a_unified_difference_is_what_diff_prints() {
        let old = units("one\ntwo\nthree\nfour\nfive\nsix\nseven\neight\nnine\nten\n");
        let new = units("one\n2\nthree\nfour\nfive\nsix\nseven\neight\nnine\nten\neleven");
        let text = unified(&Pieces::lines(&old), &Pieces::lines(&new), 1, None);
        assert_eq!(
            String::from_utf16(&text).unwrap(),
            "@@ -1,3 +1,3 @@\n one\n-two\n+2\n three\n@@ -10,1 +10,2 @@\n ten\n+eleven\n\\ No newline at end of file\n"
        );
        let empty = unified(&Pieces::lines(&old), &Pieces::lines(&old), 3, None);
        assert!(empty.is_empty());
        let created = unified(&Pieces::lines(&[]), &Pieces::lines(&units("a\n")), 3, None);
        assert_eq!(
            String::from_utf16(&created).unwrap(),
            "@@ -0,0 +1,1 @@\n+a\n"
        );
    }

    fn glob(pattern: &str) -> Glob {
        Glob::new(pattern, false).unwrap()
    }

    #[test]
    fn a_glob_matches_as_a_shell_does() {
        for (pattern, path, expected) in [
            ("*.ts", "main.ts", true),
            ("*.ts", "src/main.ts", false),
            ("**/*.ts", "main.ts", true),
            ("**/*.ts", "src/deep/main.ts", true),
            ("src/**", "src/a/b.css", true),
            ("src/**", "lib/a.css", false),
            ("src/**/test/*.ts", "src/test/a.ts", true),
            ("src/**/test/*.ts", "src/x/y/test/a.ts", true),
            ("src/**/test/*.ts", "src/x/testing/a.ts", false),
            ("a?c", "abc", true),
            ("a?c", "a/c", false),
            ("[a-c]x", "bx", true),
            ("[!a-c]x", "bx", false),
            ("[]]x", "]x", true),
            ("*.{ts,tsx}", "a.tsx", true),
            ("*.{ts,tsx}", "a.js", false),
            ("{src,lib}/**/*.{js,ts}", "lib/a/b.js", true),
            ("a\\*b", "a*b", true),
            ("a\\*b", "axb", false),
            ("!*.md", "a.md", false),
            ("!*.md", "a.ts", true),
            ("a**b", "axxb", true),
            ("a**b", "ax/xb", false),
            ("*", "", true),
            ("{a", "{a", true),
        ] {
            assert_eq!(glob(pattern).matches(path), expected, "{pattern} on {path}");
        }
        assert!(Glob::new("*.TS", true).unwrap().matches("Main.ts"));
        let many = "{a,b}".repeat(11);
        assert!(Glob::new(&many, false).is_err());
        let stars = glob(&("*a".repeat(30) + "c"));
        assert!(!stars.matches(&"a".repeat(200)));
    }

    fn found(
        patterns: &[&str],
        text: &str,
        ignore_case: bool,
        whole_word: bool,
    ) -> Vec<(usize, usize, usize)> {
        let patterns: Vec<String> = patterns
            .iter()
            .map(|p| {
                if ignore_case {
                    p.chars().map(fold).collect()
                } else {
                    p.to_string()
                }
            })
            .collect();
        let kind = if whole_word {
            MatchKind::Standard
        } else {
            MatchKind::LeftmostLongest
        };
        let search = Search {
            automaton: AhoCorasick::builder()
                .match_kind(kind)
                .build(&patterns)
                .unwrap(),
            ignore_case,
            whole_word,
        };
        let units = units(text);
        let haystack = Haystack::new(&units, ignore_case);
        let mut offsets = Offsets {
            widths: haystack.widths.iter(),
            byte: 0,
            unit: 0,
        };
        search
            .matches(&haystack)
            .into_iter()
            .map(|(start, end, pattern)| {
                let index = offsets.unit_at(start);
                (index, offsets.unit_at(end) - index, pattern)
            })
            .collect()
    }

    #[test]
    fn a_search_answers_in_utf16_units() {
        assert_eq!(
            found(&["он", "она"], "😀 она и он", false, false),
            [(3, 3, 1), (9, 2, 0)]
        );
        assert_eq!(
            found(&["straße", "İ"], "STRASSE Straße İ", true, false),
            [(8, 6, 0), (15, 1, 1)]
        );
        assert_eq!(
            found(&["cat", "category"], "cat category concat", false, true),
            [(0, 3, 0), (4, 8, 1)]
        );
        assert!(found(&["a"], "", false, false).is_empty());
    }
}
