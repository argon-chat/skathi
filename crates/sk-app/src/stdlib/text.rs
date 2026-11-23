//! `skathi:text`: building and reading strings, differences and patches, many-word search and
//! glob patterns. Strings stay UTF-16 here, as the engine has them, so an index means what it
//! means in JS.

mod diff;
mod merge;
mod patch;

use std::collections::BTreeMap;
use std::ffi::c_void;
use std::panic::AssertUnwindSafe;
use std::sync::{Mutex, OnceLock};

use aho_corasick::{AhoCorasick, MatchKind};
use sk_runtime::host;
use sk_runtime::redjs::api::HostArgs;
use sk_runtime::redjs::capi::JSClassRef;
use sk_runtime::redjs::glue;
use sk_runtime::redjs::webapi::util::*;

use super::support::{self, *};
use diff::{Algorithm, Edit, Inline, Options, Pieces, Whitespace};
use patch::{PatchFile, Pick};

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

/// A text to compare as it came: bytes are read as UTF-8 where the work is done.
enum Source {
    Units(Vec<u16>),
    Bytes(Vec<u8>),
}

impl Source {
    fn units(self) -> Vec<u16> {
        match self {
            Source::Units(units) => units,
            Source::Bytes(bytes) => String::from_utf8_lossy(&bytes).encode_utf16().collect(),
        }
    }
}

unsafe fn raw_source(g: *mut c_void, v: u64, what: &str) -> Js<Source> {
    if kind(g, v) == KIND_STRING {
        return read_string16(g, v).map(Source::Units).ok_or(Thrown);
    }
    if let Some(view) = buffer_source(g, v) {
        return Ok(Source::Bytes(view.bytes().to_vec()));
    }
    match super::memory::region_of(g, v, &what)? {
        Some(region) => Ok(Source::Bytes(region.bytes().to_vec())),
        None => type_error(
            g,
            &format!("{what} must be a string, a Uint8Array or a NativeBuffer"),
        ),
    }
}

unsafe fn two_sources(g: *mut c_void, old: u64, new: u64, what: &str) -> Js<(Source, Source)> {
    Ok((
        raw_source(g, old, &format!("{what}: the old text"))?,
        raw_source(g, new, &format!("{what}: the new text"))?,
    ))
}

unsafe fn two_texts(g: *mut c_void, old: u64, new: u64, what: &str) -> Js<(Vec<u16>, Vec<u16>)> {
    Ok((
        units(g, old, &format!("{what}: the old text"))?,
        units(g, new, &format!("{what}: the new text"))?,
    ))
}

unsafe fn context_of(g: *mut c_void, bag: u64, what: &str) -> Js<usize> {
    let given = option(g, bag, "context");
    if given == UNDEF {
        return Ok(3);
    }
    count(g, given, &format!("{what}: context"))
}

unsafe fn diff_options(g: *mut c_void, bag: u64, what: &str, algorithm: Algorithm) -> Js<Options> {
    let pick = |name: &str, names: &[&str], default: usize| {
        choice(
            g,
            option(g, bag, name),
            names,
            default,
            &format!("{what}: {name}"),
        )
    };
    let default = match algorithm {
        Algorithm::Histogram => 0,
        Algorithm::Myers => 1,
        Algorithm::Patience => 2,
    };
    let algorithm = match pick("algorithm", &["histogram", "myers", "patience"], default)? {
        1 => Algorithm::Myers,
        2 => Algorithm::Patience,
        _ => Algorithm::Histogram,
    };
    let whitespace = match pick(
        "ignoreWhitespace",
        &["none", "trailing", "amount", "all"],
        0,
    )? {
        1 => Whitespace::Trailing,
        2 => Whitespace::Amount,
        3 => Whitespace::All,
        _ => Whitespace::None,
    };
    let inline = match pick("inline", &["none", "words", "chars"], 0)? {
        1 => Inline::Words,
        2 => Inline::Chars,
        _ => Inline::None,
    };
    Ok(Options {
        algorithm,
        whitespace,
        ignore_case: flag(g, bag, "ignoreCase"),
        ignore_line_endings: flag(g, bag, "ignoreLineEndings"),
        context: context_of(g, bag, what)?,
        inline,
    })
}

/// `oldName` and `newName`: both or neither.
unsafe fn names_of(g: *mut c_void, bag: u64, what: &str) -> Js<Option<(Vec<u16>, Vec<u16>)>> {
    let (old_name, new_name) = (option(g, bag, "oldName"), option(g, bag, "newName"));
    if old_name == UNDEF && new_name == UNDEF {
        return Ok(None);
    }
    Ok(Some((
        units(g, old_name, &format!("{what}: oldName"))?,
        units(g, new_name, &format!("{what}: newName"))?,
    )))
}

unsafe fn number_at(g: *mut c_void, object: u64, name: &str, n: usize) {
    define(g, object, name, enc_f64(n as f64), 0);
}

unsafe fn changes_out(g: *mut c_void, changes: &[diff::LineChange]) -> u64 {
    let result = new_array(g);
    for (i, c) in changes.iter().enumerate() {
        let entry = new_object(g, UNDEF);
        number_at(g, entry, "oldStart", c.change.old.0);
        number_at(g, entry, "oldEnd", c.change.old.1);
        number_at(g, entry, "newStart", c.change.new.0);
        number_at(g, entry, "newEnd", c.change.new.1);
        if let Some(inline) = &c.inline {
            let parts = new_array(g);
            for (k, [old_from, old_to, new_from, new_to]) in inline.iter().enumerate() {
                let part = new_object(g, UNDEF);
                number_at(g, part, "oldFrom", *old_from);
                number_at(g, part, "oldTo", *old_to);
                number_at(g, part, "newFrom", *new_from);
                number_at(g, part, "newTo", *new_to);
                set_i(g, parts, k as u32, part);
            }
            define(g, entry, "inline", parts, 0);
        }
        set_i(g, result, i as u32, entry);
    }
    result
}

std_fn!(fn diff_changes(g, args) {
    let what = "TextDiff.changes";
    let (old, new) = two_sources(g, args.arg(0).0, args.arg(1).0, what)?;
    let o = diff_options(g, args.arg(2).0, what, Algorithm::Histogram)?;
    let changes = diff::line_changes(&old.units(), &new.units(), &o);
    Ok(changes_out(g, &changes))
});

type Finish = Box<dyn FnOnce() + Send>;

static FINISHED: Mutex<Vec<Finish>> = Mutex::new(Vec::new());

/// Runs `work` on the native-module task pool; `done` runs on the JS thread at the next pump,
/// with `None` if the work panicked.
fn on_worker<R: Send + 'static>(
    g: *mut c_void,
    work: impl FnOnce() -> R + Send + 'static,
    done: impl FnOnce(*mut c_void, Option<R>) + Send + 'static,
) {
    let g = g as usize;
    crate::embed::pool_run(Box::new(move || {
        let result = std::panic::catch_unwind(AssertUnwindSafe(work)).ok();
        FINISHED
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .push(Box::new(move || done(g as *mut c_void, result)));
        sk_runtime::frames::wake();
        crate::background::prompt();
    }));
}

/// Settles the promises of the differences that finished since the last frame.
pub(crate) fn pump() {
    let finished = std::mem::take(&mut *FINISHED.lock().unwrap_or_else(|e| e.into_inner()));
    for finish in finished {
        finish();
    }
}

/// The promise `run` returns, or one rejected with what it threw.
unsafe fn promise_of(g: *mut c_void, run: impl FnOnce() -> Js<u64>) -> u64 {
    match run() {
        Ok(promise) => promise,
        Err(Thrown) => {
            let mut error = UNDEF;
            glue::rjs_exception_pending(g, &mut error);
            host::rejected_with(g, error)
        }
    }
}

std_fn!(fn diff_changes_async(g, args) {
    let (old, new, bag) = (args.arg(0).0, args.arg(1).0, args.arg(2).0);
    Ok(promise_of(g, || {
        let what = "TextDiff.changesAsync";
        let (old, new) = two_sources(g, old, new, what)?;
        let o = diff_options(g, bag, what, Algorithm::Histogram)?;
        let (promise, settler) = host::deferred(g);
        on_worker(
            g,
            move || diff::line_changes(&old.units(), &new.units(), &o),
            move |g, result| unsafe {
                match result {
                    Some(changes) => settler.resolve(g, changes_out(g, &changes)),
                    None => settler.reject(g, host::error(g, "Error", &format!("{what}: the difference could not be computed"))),
                }
            },
        );
        Ok(promise)
    }))
});

std_fn!(fn diff_lines(g, args) {
    let what = "TextDiff.lines";
    let (old, new) = two_sources(g, args.arg(0).0, args.arg(1).0, what)?;
    let o = diff_options(g, args.arg(2).0, what, Algorithm::Histogram)?;
    let (old, new) = (old.units(), new.units());
    let (a, b) = (Pieces::lines(&old), Pieces::lines(&new));
    let result = new_array(g);
    for (i, hunk) in diff::hunks(&diff::diff_pieces(&a, &b, &o), o.context).iter().enumerate() {
        let lines = new_array(g);
        let object = new_object(g, UNDEF);
        number_at(g, object, "oldStart", hunk.old_start + 1);
        number_at(g, object, "oldLines", hunk.old_count);
        number_at(g, object, "newStart", hunk.new_start + 1);
        number_at(g, object, "newLines", hunk.new_count);
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

/// The runs of `words()` and `chars()`: joining the `equal` and `insert` ones gives the new text.
unsafe fn runs(g: *mut c_void, args: &HostArgs, what: &str, words: bool) -> Js<u64> {
    let (old, new) = two_texts(g, args.arg(0).0, args.arg(1).0, what)?;
    let default = if words {
        Algorithm::Histogram
    } else {
        Algorithm::Myers
    };
    let o = Options {
        ignore_line_endings: false,
        ..diff_options(g, args.arg(2).0, what, default)?
    };
    let (a, b) = if words {
        (Pieces::words(&old), Pieces::words(&new))
    } else {
        (Pieces::points(&old), Pieces::points(&new))
    };
    let (ids, tokens) = diff::intern(&[&a, &b], &o);
    let script = diff::diff(&ids[0], &ids[1], tokens, o.algorithm);
    let result = new_array(g);
    let (mut at_old, mut at_new) = (0, 0);
    for (i, (edit, count)) in script.0.iter().enumerate() {
        let text = if *edit == Edit::Insert {
            b.span(at_new, *count)
        } else {
            a.span(at_old, *count)
        };
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
}

std_fn!(fn diff_words(g, args) { runs(g, &args, "TextDiff.words", true) });
std_fn!(fn diff_chars(g, args) { runs(g, &args, "TextDiff.chars", false) });

std_fn!(fn diff_unified(g, args) {
    let what = "TextDiff.unified";
    let (old, new) = two_sources(g, args.arg(0).0, args.arg(1).0, what)?;
    let bag = args.arg(2).0;
    let o = diff_options(g, bag, what, Algorithm::Histogram)?;
    let names = names_of(g, bag, what)?;
    let file = patch::from_texts(&old.units(), &new.units(), &o, names);
    Ok(js_string16(g, &patch::print(&[file])))
});

unsafe fn pair(g: *mut c_void, (from, to): (usize, usize)) -> u64 {
    let out = new_array(g);
    set_i(g, out, 0, enc_f64(from as f64));
    set_i(g, out, 1, enc_f64(to as f64));
    out
}

std_fn!(fn diff_merge(g, args) {
    let what = "TextDiff.merge";
    let base = units(g, args.arg(0).0, &format!("{what}: the base text"))?;
    let ours = units(g, args.arg(1).0, &format!("{what}: our text"))?;
    let theirs = units(g, args.arg(2).0, &format!("{what}: their text"))?;
    let bag = args.arg(3).0;
    let style = match choice(g, option(g, bag, "style"), &["merge", "diff3", "zdiff3"], 0, &format!("{what}: style"))? {
        1 => merge::Style::Diff3,
        2 => merge::Style::Zdiff3,
        _ => merge::Style::Merge,
    };
    let given = option(g, bag, "labels");
    let labels: Vec<Vec<u16>> = if given == UNDEF {
        ["base", "ours", "theirs"].iter().map(|l| l.encode_utf16().collect()).collect()
    } else {
        let what = format!("{what}: labels");
        if list(g, given, &what)? != 3 {
            return type_error(g, &format!("{what} must be three strings: base, ours and theirs"));
        }
        let mut labels = Vec::with_capacity(3);
        for i in 0..3 {
            let label = units(g, get_i(g, given, i), &format!("{what}: label {i}"))?;
            if label.iter().any(|u| *u == NEWLINE || *u == RETURN) {
                return type_error(g, &format!("{what}: label {i} has a line break, which would break the markers"));
            }
            labels.push(label);
        }
        labels
    };
    let merged = merge::merge(&base, &ours, &theirs, style, [&labels[0], &labels[1], &labels[2]]);
    let result = new_object(g, UNDEF);
    define(g, result, "text", js_string16(g, &merged.text), 0);
    let conflicts = new_array(g);
    for (i, c) in merged.conflicts.iter().enumerate() {
        let entry = new_object(g, UNDEF);
        define(g, entry, "base", pair(g, c.base), 0);
        define(g, entry, "ours", pair(g, c.ours), 0);
        define(g, entry, "theirs", pair(g, c.theirs), 0);
        number_at(g, entry, "at", c.at);
        set_i(g, conflicts, i as u32, entry);
    }
    define(g, result, "conflicts", conflicts, 0);
    Ok(result)
});

struct PatchState {
    files: Vec<PatchFile>,
}

host_state!(patch_state, patch_new, PatchState, PATCH, "Patch");

const FILES: &str = "files";

std_fn!(fn patch_parse(g, args) {
    let text = units(g, args.arg(0).0, "Patch.parse: the text")?;
    match patch::parse(&text) {
        Ok(files) => Ok(patch_new(g, PatchState { files })),
        Err(error) => syntax_error(g, &format!("Patch.parse: {error}")),
    }
});

std_fn!(fn patch_from(g, args) {
    let what = "Patch.from";
    let (old, new) = two_sources(g, args.arg(0).0, args.arg(1).0, what)?;
    let bag = args.arg(2).0;
    let o = diff_options(g, bag, what, Algorithm::Histogram)?;
    let names = names_of(g, bag, what)?;
    let file = patch::from_texts(&old.units(), &new.units(), &o, names);
    Ok(patch_new(g, PatchState { files: vec![file] }))
});

unsafe fn path_out(g: *mut c_void, path: &Option<String>) -> u64 {
    path.as_deref().map_or(NULL, |p| js_string(g, p))
}

unsafe fn files_out(g: *mut c_void, files: &[PatchFile]) -> u64 {
    let result = new_array(g);
    for (i, f) in files.iter().enumerate() {
        let file = new_object(g, UNDEF);
        define(g, file, "oldPath", path_out(g, &f.old_path), 0);
        define(g, file, "newPath", path_out(g, &f.new_path), 0);
        define(g, file, "status", js_string(g, f.status.name()), 0);
        if let Some(mode) = f.old_mode {
            number_at(g, file, "oldMode", mode as usize);
        }
        if let Some(mode) = f.new_mode {
            number_at(g, file, "newMode", mode as usize);
        }
        define(g, file, "binary", enc_bool(f.binary.is_some()), 0);
        let hunks = new_array(g);
        for (h, hunk) in f.hunks.iter().enumerate() {
            let object = new_object(g, UNDEF);
            number_at(g, object, "oldStart", hunk.old_start);
            number_at(g, object, "oldLines", hunk.old_count);
            number_at(g, object, "newStart", hunk.new_start);
            number_at(g, object, "newLines", hunk.new_count);
            if !hunk.section.is_empty() {
                define(g, object, "section", js_string16(g, &hunk.section), 0);
            }
            let lines = new_array(g);
            for (k, line) in hunk.lines.iter().enumerate() {
                let entry = new_object(g, UNDEF);
                define(g, entry, "op", js_string(g, line.edit.name()), 0);
                define(g, entry, "text", js_string16(g, &line.text), 0);
                if line.no_newline {
                    define(g, entry, "noNewline", enc_bool(true), 0);
                }
                set_i(g, lines, k as u32, entry);
            }
            define(g, object, "lines", lines, 0);
            set_i(g, hunks, h as u32, object);
        }
        define(g, file, "hunks", hunks, 0);
        set_i(g, result, i as u32, file);
    }
    result
}

std_fn!(fn patch_files(g, args) {
    let this = args.this().0;
    let files = &patch_state(g, this)?.files;
    let cached = slot_get(g, this, FILES);
    if cached != UNDEF {
        return Ok(cached);
    }
    let out = files_out(g, files);
    slot_set(g, this, FILES, out);
    Ok(out)
});

std_fn!(fn patch_apply(g, args) {
    let what = "Patch.apply";
    let text = units(g, args.arg(0).0, &format!("{what}: the text"))?;
    let bag = args.arg(1).0;
    let index = match option(g, bag, "file") {
        UNDEF => 0,
        v => count(g, v, &format!("{what}: file"))?,
    };
    let fuzz = match option(g, bag, "fuzz") {
        UNDEF => 0,
        v => count(g, v, &format!("{what}: fuzz"))?,
    };
    let reverse = flag(g, bag, "reverse");
    let files = &patch_state(g, args.this().0)?.files;
    let Some(file) = files.get(index) else {
        return range_error(g, &format!("{what}: there is no file {index}; the patch has {}", files.len()));
    };
    let reversed;
    let file = if reverse {
        reversed = patch::reverse(file);
        &reversed
    } else {
        file
    };
    let applied = match patch::apply(file, &text, fuzz) {
        Ok(applied) => applied,
        Err(error) => return type_error(g, &format!("{what}: {error}")),
    };
    let result = new_object(g, UNDEF);
    define(g, result, "text", js_string16(g, &applied.text), 0);
    let rejected = new_array(g);
    for (i, h) in applied.rejected.iter().enumerate() {
        set_i(g, rejected, i as u32, enc_f64(*h as f64));
    }
    define(g, result, "rejected", rejected, 0);
    Ok(result)
});

std_fn!(fn patch_select(g, args) {
    let what = "Patch.select";
    let given = args.arg(0).0;
    let n = list(g, given, &format!("{what}: the picks"))?;
    let mut picks: BTreeMap<usize, Pick> = BTreeMap::new();
    let mut wanted = Vec::new();
    for i in 0..n {
        let pick = get_i(g, given, i as u32);
        if !is_cell(pick) {
            return type_error(g, &format!("{what}: pick {i} must be an object"));
        }
        let file = count(g, get(g, pick, "file"), &format!("{what}: pick {i}: file"))?;
        let (hunks, lines) = (get(g, pick, "hunks"), get(g, pick, "lines"));
        let mut entry = Pick { whole: hunks == UNDEF && lines == UNDEF, ..Pick::default() };
        if hunks != UNDEF {
            for k in 0..list(g, hunks, &format!("{what}: pick {i}: hunks"))? {
                entry.hunks.insert(count(g, get_i(g, hunks, k as u32), &format!("{what}: pick {i}: hunk {k}"))?);
            }
        }
        if lines != UNDEF {
            for k in 0..list(g, lines, &format!("{what}: pick {i}: lines"))? {
                let line = get_i(g, lines, k as u32);
                let name = format!("{what}: pick {i}: line {k}");
                if list(g, line, &name)? != 2 {
                    return type_error(g, &format!("{name} must be [hunk, line]"));
                }
                entry.lines.insert((count(g, get_i(g, line, 0), &name)?, count(g, get_i(g, line, 1), &name)?));
            }
        }
        wanted.push((i, file, entry));
    }
    let reverse = flag(g, args.arg(1).0, "reverse");
    let files = &patch_state(g, args.this().0)?.files;
    for (i, file, entry) in wanted {
        let Some(f) = files.get(file) else {
            return range_error(g, &format!("{what}: pick {i}: there is no file {file}; the patch has {}", files.len()));
        };
        if let Some(h) = entry.hunks.iter().find(|h| **h >= f.hunks.len()) {
            return range_error(g, &format!("{what}: pick {i}: file {file} has no hunk {h}"));
        }
        if let Some((h, l)) = entry.lines.iter().find(|(h, l)| f.hunks.get(*h).is_none_or(|hunk| *l >= hunk.lines.len())) {
            return range_error(g, &format!("{what}: pick {i}: file {file} has no line {l} in hunk {h}"));
        }
        let merged = picks.entry(file).or_default();
        merged.whole |= entry.whole;
        merged.hunks.extend(entry.hunks);
        merged.lines.extend(entry.lines);
    }
    let selected = patch::select(files, &picks, reverse);
    Ok(patch_new(g, PatchState { files: selected }))
});

std_fn!(fn patch_reverse(g, args) {
    let files = patch_state(g, args.this().0)?.files.iter().map(patch::reverse).collect();
    Ok(patch_new(g, PatchState { files }))
});

std_fn!(fn patch_to_string(g, args) {
    Ok(js_string16(g, &patch::print(&patch_state(g, args.this().0)?.files)))
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
                ("changes", 2, diff_changes),
                ("changesAsync", 2, diff_changes_async),
                ("lines", 2, diff_lines),
                ("words", 2, diff_words),
                ("chars", 2, diff_chars),
                ("unified", 2, diff_unified),
                ("merge", 3, diff_merge),
            ],
        ),
        0,
    );

    let patch = support::class(
        g,
        &ClassSpec {
            name: "Patch",
            arity: 0,
            construct: None,
            parent_proto: 0,
            methods: &[
                ("apply", 1, patch_apply),
                ("select", 1, patch_select),
                ("reverse", 0, patch_reverse),
                ("toString", 0, patch_to_string),
            ],
            getters: &[("files", patch_files)],
            accessors: &[],
        },
    );
    static_fn(g, patch.ctor, "parse", 1, patch_parse);
    static_fn(g, patch.ctor, "from", 2, patch_from);
    define(g, module, "Patch", patch.ctor, 0);

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
    use std::time::{Duration, Instant};

    use super::*;

    fn units(text: &str) -> Vec<u16> {
        text.encode_utf16().collect()
    }

    #[test]
    fn a_unified_difference_is_what_diff_prints() {
        let old = units("one\ntwo\nthree\nfour\nfive\nsix\nseven\neight\nnine\nten\n");
        let new = units("one\n2\nthree\nfour\nfive\nsix\nseven\neight\nnine\nten\neleven");
        let unified = |old: &[u16], new: &[u16], context: usize| {
            let o = Options {
                context,
                ..Options::default()
            };
            String::from_utf16(&patch::print(&[patch::from_texts(old, new, &o, None)])).unwrap()
        };
        assert_eq!(
            unified(&old, &new, 1),
            "@@ -1,3 +1,3 @@\n one\n-two\n+2\n three\n@@ -10 +10,2 @@\n ten\n+eleven\n\\ No newline at end of file\n"
        );
        assert!(unified(&old, &old, 3).is_empty());
        assert_eq!(unified(&[], &units("a\n"), 3), "@@ -0,0 +1 @@\n+a\n");
    }

    #[test]
    fn changes_async_runs_off_the_js_thread_and_patches_work_from_js() {
        sk_runtime::testing::on_js_thread(move || {
            sk_runtime::JsRuntime::with(|rt| {
                super::super::install(rt);
                let context = rt.ctx();
                let eval = |src: &str| -> String {
                    let v = context.eval(src).unwrap();
                    context.read_string(v).unwrap()
                };
                eval(
                    "(() => {
                       const { TextDiff } = __sk_std.text;
                       globalThis.__old = Array.from({ length: 20000 }, (_, i) => 'line ' + i).join('\\n');
                       globalThis.__new = __old.replace('line 500\\n', 'five hundred\\n');
                       globalThis.__out = {};
                       const bytes = Uint8Array.from(__new, (c) => c.charCodeAt(0));
                       TextDiff.changesAsync(__old, bytes, { inline: 'words' })
                         .then((c) => { __out.changes = c; }, (e) => { __out.failed = String(e); });
                       TextDiff.changesAsync(__old, 1).then(() => {}, (e) => { __out.rejected = e.name; });
                       return 'started';
                     })()",
                );
                // Nothing runs on this thread while it waits, so the work finishing proves a worker did it.
                let deadline = Instant::now() + Duration::from_secs(20);
                while FINISHED.lock().unwrap().is_empty() {
                    assert!(
                        Instant::now() < deadline,
                        "the difference did not finish on a worker"
                    );
                    std::thread::sleep(Duration::from_millis(5));
                }
                assert_eq!(eval("String('changes' in __out)"), "false");
                while eval("String('changes' in __out || 'failed' in __out)") != "true" {
                    assert!(Instant::now() < deadline, "the promise did not settle");
                    pump();
                    context.drain_microtasks();
                    std::thread::sleep(Duration::from_millis(5));
                }
                assert_eq!(
                    eval(
                        "JSON.stringify([__out.rejected, __out.changes.map((c) => [c.oldStart, c.oldEnd, c.newStart, c.newEnd,
                           c.inline.map((p) => __old.slice(p.oldFrom, p.oldTo) + '>' + __new.slice(p.newFrom, p.newTo))])])"
                    ),
                    r#"["TypeError",[[500,501,500,501,["line>five","500>hundred"]]]]"#
                );
                assert_eq!(
                    eval(
                        "(() => {
                           const { Patch, TextDiff } = __sk_std.text;
                           const p = Patch.from('a\\nb\\nc\\n', 'a\\nB\\nc\\nd\\n', { oldName: 'a/x', newName: 'b/x', context: 0 });
                           const parsed = Patch.parse(p.toString());
                           const merged = TextDiff.merge('1\\n2\\n', '1\\nours\\n', '1\\ntheirs\\n', { labels: ['B', 'O', 'T'] });
                           let illegal = 'ok';
                           try { new Patch(); } catch (e) { illegal = e.name; }
                           return JSON.stringify([
                             p.toString(),
                             parsed.files[0].status,
                             parsed.files === parsed.files,
                             parsed.apply('x\\na\\nb\\nc\\n').text,
                             parsed.reverse().apply('a\\nB\\nc\\nd\\n').text,
                             parsed.select([{ file: 0, hunks: [1] }]).toString(),
                             parsed.select([{ file: 0, lines: [[0, 1]] }], { reverse: true }).apply('a\\nB\\nc\\nd\\n').text,
                             parsed.apply('a\\nq\\nc\\n').rejected,
                             merged,
                             TextDiff.words('one two', 'one three'),
                             illegal,
                           ]);
                         })()"
                    ),
                    r#"["--- a/x\n+++ b/x\n@@ -2 +2 @@\n-b\n+B\n@@ -3,0 +4 @@\n+d\n","modified",true,"x\na\nB\nc\nd\n","a\nb\nc\n","--- a/x\n+++ b/x\n@@ -3,0 +4 @@\n+d\n","a\nc\nd\n",[0],{"text":"1\n<<<<<<< O\nours\n=======\ntheirs\n>>>>>>> T\n","conflicts":[{"base":[1,2],"ours":[1,2],"theirs":[1,2],"at":1}]},[{"op":"equal","text":"one "},{"op":"delete","text":"two"},{"op":"insert","text":"three"}],"TypeError"]"#
                );
            })
        });
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
