//! Unified diffs as git writes and reads them: parsed, printed, applied, cut down and turned around.

use std::collections::{BTreeMap, BTreeSet};

use super::diff::{self, Edit, Options, Pieces, NEWLINE};

#[derive(Clone, Debug, PartialEq)]
pub(super) struct PatchLine {
    pub(super) edit: Edit,
    /// Without its `\n`; a `\r` before it stays.
    pub(super) text: Vec<u16>,
    /// `\ No newline at end of file` follows it.
    pub(super) no_newline: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub(super) struct PatchHunk {
    /// As the header has them: from 1, or the line before for an empty range.
    pub(super) old_start: usize,
    pub(super) old_count: usize,
    pub(super) new_start: usize,
    pub(super) new_count: usize,
    /// What follows the closing `@@`, usually the enclosing function.
    pub(super) section: Vec<u16>,
    pub(super) lines: Vec<PatchLine>,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum Status {
    Modified,
    Added,
    Deleted,
    Renamed,
    Copied,
}

impl Status {
    pub(super) fn name(self) -> &'static str {
        match self {
            Status::Modified => "modified",
            Status::Added => "added",
            Status::Deleted => "deleted",
            Status::Renamed => "renamed",
            Status::Copied => "copied",
        }
    }
}

/// What a `diff --git` header carries besides the paths and modes.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct Git {
    pub(super) similarity: Option<u32>,
    pub(super) dissimilarity: Option<u32>,
    pub(super) index: Option<(String, String)>,
    /// `a/` and `b/`, or what the diff used instead.
    pub(super) prefixes: (String, String),
}

#[derive(Clone, Debug, PartialEq)]
pub(super) struct PatchFile {
    pub(super) old_path: Option<String>,
    pub(super) new_path: Option<String>,
    pub(super) status: Status,
    pub(super) old_mode: Option<u32>,
    pub(super) new_mode: Option<u32>,
    pub(super) git: Option<Git>,
    /// The `---` and `+++` names of a patch that is not git's, as written.
    pub(super) labels: Option<(Vec<u16>, Vec<u16>)>,
    /// The lines that stand for a binary change, as written.
    pub(super) binary: Option<Vec<Vec<u16>>>,
    pub(super) hunks: Vec<PatchHunk>,
}

impl PatchFile {
    fn plain(labels: Option<(Vec<u16>, Vec<u16>)>) -> PatchFile {
        let path = |label: &[u16]| {
            let name = String::from_utf16_lossy(label);
            let name = name.split('\t').next().unwrap_or("").to_string();
            (name != "/dev/null").then_some(name)
        };
        let (old_path, new_path) = match &labels {
            Some((old, new)) => (path(old), path(new)),
            None => (None, None),
        };
        let status = match (&labels, &old_path, &new_path) {
            (Some(_), None, Some(_)) => Status::Added,
            (Some(_), Some(_), None) => Status::Deleted,
            _ => Status::Modified,
        };
        PatchFile {
            old_path,
            new_path,
            status,
            old_mode: None,
            new_mode: None,
            git: None,
            labels,
            binary: None,
            hunks: Vec::new(),
        }
    }
}

fn units(text: &str) -> impl Iterator<Item = u16> + '_ {
    text.encode_utf16()
}

fn starts(line: &[u16], prefix: &str) -> bool {
    let mut at = 0;
    for unit in units(prefix) {
        if line.get(at) != Some(&unit) {
            return false;
        }
        at += 1;
    }
    true
}

fn strip_cr(line: &[u16]) -> &[u16] {
    line.strip_suffix(&[b'\r' as u16]).unwrap_or(line)
}

fn header_text(line: &[u16]) -> String {
    String::from_utf16_lossy(strip_cr(line))
}

/// A C-quoted path as git writes one with unusual characters: `"a/t\303\251st"`.
fn unquote(text: &str) -> Option<(String, &str)> {
    let rest = text.strip_prefix('"')?;
    let mut bytes = Vec::new();
    let mut chars = rest.char_indices();
    while let Some((i, c)) = chars.next() {
        match c {
            '"' => return Some((String::from_utf8_lossy(&bytes).into_owned(), &rest[i + 1..])),
            '\\' => {
                let (_, e) = chars.next()?;
                let simple = match e {
                    'a' => Some(7),
                    'b' => Some(8),
                    't' => Some(9),
                    'n' => Some(10),
                    'v' => Some(11),
                    'f' => Some(12),
                    'r' => Some(13),
                    '"' => Some(b'"'),
                    '\\' => Some(b'\\'),
                    _ => None,
                };
                match simple {
                    Some(b) => bytes.push(b),
                    None => {
                        let mut value = e.to_digit(8)?;
                        for _ in 0..2 {
                            let (_, d) = chars.next()?;
                            value = value * 8 + d.to_digit(8)?;
                        }
                        bytes.push(value as u8);
                    }
                }
            }
            c => {
                let mut buf = [0u8; 4];
                bytes.extend_from_slice(c.encode_utf8(&mut buf).as_bytes());
            }
        }
    }
    None
}

fn quote(path: &str) -> String {
    let plain = path
        .bytes()
        .all(|b| (0x20..0x7F).contains(&b) && b != b'"' && b != b'\\');
    if plain {
        return path.to_string();
    }
    let mut out = String::from("\"");
    for b in path.bytes() {
        match b {
            7 => out.push_str("\\a"),
            8 => out.push_str("\\b"),
            9 => out.push_str("\\t"),
            10 => out.push_str("\\n"),
            11 => out.push_str("\\v"),
            12 => out.push_str("\\f"),
            13 => out.push_str("\\r"),
            b'"' => out.push_str("\\\""),
            b'\\' => out.push_str("\\\\"),
            0x20..=0x7E => out.push(b as char),
            _ => out.push_str(&format!("\\{b:03o}")),
        }
    }
    out.push('"');
    out
}

/// A path in a header line: quoted, or up to a tab.
fn header_path(text: &str) -> String {
    if let Some((path, _)) = unquote(text) {
        return path;
    }
    text.split('\t').next().unwrap_or("").to_string()
}

/// `a/` of `a/src/main.rs`: one letter and a slash.
fn prefix_of(name: &str) -> Option<&str> {
    let b = name.as_bytes();
    (b.len() > 2 && b[0].is_ascii_alphabetic() && b[1] == b'/').then(|| &name[..2])
}

/// The two names of `diff --git a/x b/x`. Unquoted names with spaces are found by being equal
/// after their prefixes, as git does.
fn git_names(rest: &str) -> Option<(String, String)> {
    if let Some((old, after)) = unquote(rest) {
        let after = after.strip_prefix(' ')?;
        let new = unquote(after).map_or_else(|| after.to_string(), |(n, _)| n);
        return Some((old, new));
    }
    if let Some(at) = rest.find(" \"") {
        let (new, _) = unquote(&rest[at + 1..])?;
        return Some((rest[..at].to_string(), new));
    }
    let strip =
        |name: &str| prefix_of(name).map_or(name.to_string(), |p| name[p.len()..].to_string());
    let spaces: Vec<usize> = rest.match_indices(' ').map(|(at, _)| at).collect();
    let split = |at: usize| Some((rest[..at].to_string(), rest[at + 1..].to_string()));
    if let Some(&at) = spaces
        .iter()
        .find(|at| strip(&rest[..**at]) == strip(&rest[**at + 1..]))
    {
        return split(at);
    }
    // Two different names, as a rename has: the second starts where a prefix like the first's does.
    prefix_of(rest)?;
    let at = *spaces
        .iter()
        .find(|at| prefix_of(&rest[**at + 1..]).is_some())?;
    split(at)
}

fn octal(text: &str) -> Option<u32> {
    u32::from_str_radix(text.trim(), 8).ok()
}

fn percent(text: &str) -> Option<u32> {
    text.trim().trim_end_matches('%').parse().ok()
}

struct Parser<'a> {
    lines: Vec<&'a [u16]>,
    at: usize,
}

fn error<T>(line: usize, message: &str) -> Result<T, String> {
    Err(format!("line {}: {message}", line + 1))
}

impl<'a> Parser<'a> {
    fn new(text: &'a [u16]) -> Parser<'a> {
        let mut lines: Vec<&[u16]> = text.split(|u| *u == NEWLINE).collect();
        if lines.last().is_some_and(|l| l.is_empty()) {
            lines.pop();
        }
        Parser { lines, at: 0 }
    }

    fn line(&self) -> Option<&'a [u16]> {
        self.lines.get(self.at).copied()
    }

    fn files(&mut self) -> Result<Vec<PatchFile>, String> {
        let mut files = Vec::new();
        while let Some(line) = self.line() {
            if starts(line, "diff --git ") {
                files.push(self.git_file()?);
            } else if starts(line, "diff --cc ") || starts(line, "diff --combined ") {
                self.at += 1;
                while self.line().is_some_and(|l| !starts(l, "diff --git ")) {
                    self.at += 1;
                }
            } else if starts(line, "--- ")
                && self
                    .lines
                    .get(self.at + 1)
                    .is_some_and(|l| starts(l, "+++ "))
            {
                let old = strip_cr(&line[4..]).to_vec();
                let new = strip_cr(&self.lines[self.at + 1][4..]).to_vec();
                self.at += 2;
                let mut file = PatchFile::plain(Some((old, new)));
                file.hunks = self.hunks()?;
                files.push(file);
            } else if starts(line, "@@ -") {
                let mut file = PatchFile::plain(None);
                file.hunks = self.hunks()?;
                files.push(file);
            } else {
                self.at += 1;
            }
        }
        Ok(files)
    }

    fn git_file(&mut self) -> Result<PatchFile, String> {
        let first = header_text(self.lines[self.at]);
        let names = git_names(&first["diff --git ".len()..]);
        self.at += 1;
        let mut file = PatchFile {
            old_path: None,
            new_path: None,
            status: Status::Modified,
            old_mode: None,
            new_mode: None,
            git: None,
            labels: None,
            binary: None,
            hunks: Vec::new(),
        };
        let mut git = Git {
            similarity: None,
            dissimilarity: None,
            index: None,
            prefixes: (String::new(), String::new()),
        };
        if let Some((old, new)) = &names {
            if let (Some(a), Some(b)) = (prefix_of(old), prefix_of(new)) {
                git.prefixes = (a.to_string(), b.to_string());
            }
        }
        let strip =
            |name: &str, prefix: &str| name.strip_prefix(prefix).unwrap_or(name).to_string();
        let (mut old_path, mut new_path) = names
            .as_ref()
            .map(|(o, n)| {
                (
                    Some(strip(o, &git.prefixes.0)),
                    Some(strip(n, &git.prefixes.1)),
                )
            })
            .unwrap_or((None, None));
        let mut index_mode = None;
        while let Some(line) = self.line() {
            let text = header_text(line);
            let after = |p: &str| text.strip_prefix(p).map(str::to_string);
            if let Some(m) = after("old mode ") {
                file.old_mode = octal(&m);
            } else if let Some(m) = after("new mode ") {
                file.new_mode = octal(&m);
            } else if let Some(m) = after("deleted file mode ") {
                file.old_mode = octal(&m);
                file.status = Status::Deleted;
            } else if let Some(m) = after("new file mode ") {
                file.new_mode = octal(&m);
                file.status = Status::Added;
            } else if let Some(p) = after("copy from ") {
                old_path = Some(header_path(&p));
                file.status = Status::Copied;
            } else if let Some(p) = after("copy to ") {
                new_path = Some(header_path(&p));
                file.status = Status::Copied;
            } else if let Some(p) = after("rename from ").or_else(|| after("rename old ")) {
                old_path = Some(header_path(&p));
                file.status = Status::Renamed;
            } else if let Some(p) = after("rename to ").or_else(|| after("rename new ")) {
                new_path = Some(header_path(&p));
                file.status = Status::Renamed;
            } else if let Some(n) = after("similarity index ") {
                git.similarity = percent(&n);
            } else if let Some(n) = after("dissimilarity index ") {
                git.dissimilarity = percent(&n);
            } else if let Some(rest) = after("index ") {
                let mut parts = rest.split(' ');
                let hashes = parts.next().unwrap_or("");
                if let Some((a, b)) = hashes.split_once("..") {
                    git.index = Some((a.to_string(), b.to_string()));
                }
                index_mode = parts.next().and_then(octal);
            } else if starts(line, "--- ")
                && self
                    .lines
                    .get(self.at + 1)
                    .is_some_and(|l| starts(l, "+++ "))
            {
                let new_text = header_text(self.lines[self.at + 1]);
                let label = |t: &str, prefix: &str| {
                    let path = header_path(t);
                    (path != "/dev/null").then(|| strip(&path, prefix))
                };
                let old = label(&text[4..], &git.prefixes.0);
                let new = label(&new_text[4..], &git.prefixes.1);
                if old.is_some() || file.status != Status::Added {
                    old_path = old;
                }
                if new.is_some() || file.status != Status::Deleted {
                    new_path = new;
                }
                self.at += 2;
                break;
            } else if text.starts_with("Binary files ") {
                file.binary = Some(vec![strip_cr(line).to_vec()]);
                self.at += 1;
                break;
            } else if text == "GIT binary patch" {
                file.binary = Some(self.binary());
                break;
            } else {
                break;
            }
            self.at += 1;
        }
        if file.old_mode.is_none() && file.new_mode.is_none() {
            file.old_mode = index_mode;
            file.new_mode = index_mode;
        }
        match file.status {
            Status::Added => old_path = None,
            Status::Deleted => new_path = None,
            _ => {}
        }
        file.old_path = old_path;
        file.new_path = new_path;
        file.git = Some(git);
        if file.binary.is_none() {
            file.hunks = self.hunks()?;
        }
        Ok(file)
    }

    /// `GIT binary patch` and its one or two blocks, each ended by an empty line.
    fn binary(&mut self) -> Vec<Vec<u16>> {
        let mut raw = vec![strip_cr(self.lines[self.at]).to_vec()];
        self.at += 1;
        for _ in 0..2 {
            if !self
                .line()
                .is_some_and(|l| starts(l, "literal ") || starts(l, "delta "))
            {
                break;
            }
            while let Some(line) = self.line() {
                self.at += 1;
                let line = strip_cr(line);
                raw.push(line.to_vec());
                if line.is_empty() {
                    break;
                }
            }
        }
        raw
    }

    fn hunks(&mut self) -> Result<Vec<PatchHunk>, String> {
        let mut hunks = Vec::new();
        while self.line().is_some_and(|l| starts(l, "@@ -")) {
            hunks.push(self.hunk()?);
        }
        Ok(hunks)
    }

    fn hunk(&mut self) -> Result<PatchHunk, String> {
        let start = self.at;
        let line = self.lines[start];
        let close = (4..line.len().saturating_sub(1))
            .find(|i| line[*i] == b'@' as u16 && line[*i + 1] == b'@' as u16);
        let Some(close) = close else {
            return error(start, "a hunk header without its closing @@");
        };
        let header = String::from_utf16_lossy(&line[4..close]);
        let range = |text: &str| -> Option<(usize, usize)> {
            match text.split_once(',') {
                Some((s, c)) => Some((s.parse().ok()?, c.parse().ok()?)),
                None => Some((text.parse().ok()?, 1)),
            }
        };
        let parsed = header
            .trim()
            .split_once(" +")
            .and_then(|(old, new)| Some((range(old)?, range(new)?)));
        let Some(((old_start, old_count), (new_start, new_count))) = parsed else {
            return error(
                start,
                "a hunk header that is not @@ -start,count +start,count @@",
            );
        };
        let mut section = strip_cr(&line[close + 2..]);
        if section.first() == Some(&(b' ' as u16)) {
            section = &section[1..];
        }
        let mut hunk = PatchHunk {
            old_start,
            old_count,
            new_start,
            new_count,
            section: section.to_vec(),
            lines: Vec::new(),
        };
        self.at += 1;
        let (mut old_left, mut new_left) = (old_count, new_count);
        while old_left > 0 || new_left > 0 {
            let Some(line) = self.line() else {
                return error(
                    start,
                    &format!("the hunk ends before its {old_count} old and {new_count} new lines"),
                );
            };
            let (edit, text) = match line.first().copied() {
                None => (Edit::Equal, line),
                Some(0x20) => (Edit::Equal, &line[1..]),
                Some(0x2D) => (Edit::Delete, &line[1..]),
                Some(0x2B) => (Edit::Insert, &line[1..]),
                Some(0x5C) if !hunk.lines.is_empty() => {
                    hunk.lines.last_mut().unwrap().no_newline = true;
                    self.at += 1;
                    continue;
                }
                _ => {
                    return error(
                        self.at,
                        &format!(
                            "the hunk from line {} has fewer lines than its header says",
                            start + 1
                        ),
                    );
                }
            };
            let fits = match edit {
                Edit::Equal => old_left > 0 && new_left > 0,
                Edit::Delete => old_left > 0,
                Edit::Insert => new_left > 0,
            };
            if !fits {
                return error(
                    self.at,
                    &format!(
                        "the hunk from line {} has more lines than its header says",
                        start + 1
                    ),
                );
            }
            if edit != Edit::Insert {
                old_left -= 1;
            }
            if edit != Edit::Delete {
                new_left -= 1;
            }
            hunk.lines.push(PatchLine {
                edit,
                text: text.to_vec(),
                no_newline: false,
            });
            self.at += 1;
        }
        if self
            .line()
            .is_some_and(|l| l.first() == Some(&(b'\\' as u16)))
        {
            if let Some(last) = hunk.lines.last_mut() {
                last.no_newline = true;
            }
            self.at += 1;
        }
        Ok(hunk)
    }
}

pub(super) fn parse(text: &[u16]) -> Result<Vec<PatchFile>, String> {
    Parser::new(text).files()
}

/// One file's change from `old` to `new`, with `---` and `+++` lines when there are names.
pub(super) fn from_texts(
    old: &[u16],
    new: &[u16],
    o: &Options,
    names: Option<(Vec<u16>, Vec<u16>)>,
) -> PatchFile {
    let (a, b) = (Pieces::lines(old), Pieces::lines(new));
    let script = diff::diff_pieces(&a, &b, o);
    let mut file = PatchFile::plain(names);
    file.hunks = diff::hunks(&script, o.context)
        .into_iter()
        .map(|h| PatchHunk {
            old_start: if h.old_count == 0 {
                h.old_start
            } else {
                h.old_start + 1
            },
            old_count: h.old_count,
            new_start: if h.new_count == 0 {
                h.new_start
            } else {
                h.new_start + 1
            },
            new_count: h.new_count,
            section: Vec::new(),
            lines: h
                .lines
                .iter()
                .map(|l| {
                    let side = if l.edit == Edit::Insert { &b } else { &a };
                    PatchLine {
                        edit: l.edit,
                        text: side.piece(l.index).to_vec(),
                        no_newline: side.unterminated(l.index),
                    }
                })
                .collect(),
        })
        .collect();
    file
}

struct Out(Vec<u16>);

impl Out {
    fn str(&mut self, text: &str) -> &mut Out {
        self.0.extend(units(text));
        self
    }

    fn units(&mut self, text: &[u16]) -> &mut Out {
        self.0.extend_from_slice(text);
        self
    }
}

fn range(start: usize, count: usize) -> String {
    if count == 1 {
        start.to_string()
    } else {
        format!("{start},{count}")
    }
}

fn print_git_header(out: &mut Out, f: &PatchFile, git: &Git) {
    let (pa, pb) = &git.prefixes;
    let old = f
        .old_path
        .as_deref()
        .or(f.new_path.as_deref())
        .unwrap_or("");
    let new = f
        .new_path
        .as_deref()
        .or(f.old_path.as_deref())
        .unwrap_or("");
    out.str(&format!(
        "diff --git {} {}\n",
        quote(&format!("{pa}{old}")),
        quote(&format!("{pb}{new}"))
    ));
    match (f.status, f.old_mode, f.new_mode) {
        (Status::Added, _, Some(m)) => {
            out.str(&format!("new file mode {m:o}\n"));
        }
        (Status::Deleted, Some(m), _) => {
            out.str(&format!("deleted file mode {m:o}\n"));
        }
        (_, Some(a), Some(b)) if a != b => {
            out.str(&format!("old mode {a:o}\nnew mode {b:o}\n"));
        }
        _ => {}
    }
    if matches!(f.status, Status::Renamed | Status::Copied) {
        if let Some(n) = git.similarity {
            out.str(&format!("similarity index {n}%\n"));
        }
        let word = if f.status == Status::Renamed {
            "rename"
        } else {
            "copy"
        };
        out.str(&format!(
            "{word} from {}\n{word} to {}\n",
            quote(old),
            quote(new)
        ));
    }
    if let Some(n) = git.dissimilarity {
        out.str(&format!("dissimilarity index {n}%\n"));
    }
    if let Some((a, b)) = &git.index {
        out.str(&format!("index {a}..{b}"));
        match (f.status, f.old_mode, f.new_mode) {
            (Status::Added | Status::Deleted, _, _) => {}
            (_, Some(x), Some(y)) if x == y => {
                out.str(&format!(" {x:o}"));
            }
            _ => {}
        }
        out.str("\n");
    }
    if f.binary.is_none() && !f.hunks.is_empty() {
        let label = |prefix: &str, path: Option<&str>| match path {
            None => "/dev/null".to_string(),
            Some(p) => {
                let tab = if p.contains(' ') { "\t" } else { "" };
                format!("{}{tab}", quote(&format!("{prefix}{p}")))
            }
        };
        out.str(&format!(
            "--- {}\n+++ {}\n",
            label(pa, f.old_path.as_deref()),
            label(pb, f.new_path.as_deref())
        ));
    }
}

pub(super) fn print(files: &[PatchFile]) -> Vec<u16> {
    let mut out = Out(Vec::new());
    for f in files {
        if let Some(git) = &f.git {
            print_git_header(&mut out, f, git);
        } else if let (Some((old, new)), false) = (&f.labels, f.hunks.is_empty()) {
            out.str("--- ")
                .units(old)
                .str("\n+++ ")
                .units(new)
                .str("\n");
        }
        if let Some(raw) = &f.binary {
            for line in raw {
                out.units(line).str("\n");
            }
            continue;
        }
        for h in &f.hunks {
            out.str(&format!(
                "@@ -{} +{} @@",
                range(h.old_start, h.old_count),
                range(h.new_start, h.new_count)
            ));
            if !h.section.is_empty() {
                out.str(" ").units(&h.section);
            }
            out.str("\n");
            for l in &h.lines {
                out.str(match l.edit {
                    Edit::Equal => " ",
                    Edit::Delete => "-",
                    Edit::Insert => "+",
                });
                out.units(&l.text).str("\n");
                if l.no_newline {
                    out.str("\\ No newline at end of file\n");
                }
            }
        }
    }
    out.0
}

pub(super) struct Applied {
    pub(super) text: Vec<u16>,
    pub(super) rejected: Vec<usize>,
}

enum Placed<'p> {
    File(usize),
    Patch(&'p PatchLine),
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Try {
    Anchored,
    Exact,
    Near,
}

/// Applies one file's hunks in order. A hunk is looked for at its line, then further away in
/// both directions; with `fuzz` it may also drop up to that many context lines from each end.
/// A hunk that is not found is skipped and named in `rejected`.
pub(super) fn apply(file: &PatchFile, text: &[u16], fuzz: usize) -> Result<Applied, String> {
    if file.binary.is_some() {
        return Err("the change is to a binary file, which only git can apply".into());
    }
    let lines = Pieces::lines(text);
    let n = lines.len();
    let same = |at: usize, line: &PatchLine| {
        lines.piece(at) == line.text.as_slice() && lines.unterminated(at) == line.no_newline
    };
    let mut out: Vec<Placed> = Vec::with_capacity(n);
    let mut done = 0;
    let mut offset = 0isize;
    let mut rejected = Vec::new();
    for (h, hunk) in file.hunks.iter().enumerate() {
        let leading = hunk
            .lines
            .iter()
            .take_while(|l| l.edit == Edit::Equal)
            .count();
        let trailing = if leading == hunk.lines.len() {
            0
        } else {
            hunk.lines
                .iter()
                .rev()
                .take_while(|l| l.edit == Edit::Equal)
                .count()
        };
        let unanchored = leading == 0 && trailing == 0;
        let base = if hunk.old_count == 0 {
            hunk.old_start
        } else {
            hunk.old_start.saturating_sub(1)
        } as isize;
        // Where its line numbers say first. Elsewhere, like git, a hunk whose context stops short
        // of one side is held to the start or the end of the file; a hunk cut from a patch without
        // context looks like that anywhere, hence the exact place first. Fuzz lets go of the hold,
        // then of context lines.
        let mut tries = vec![(0, Try::Exact), (0, Try::Anchored)];
        if fuzz > 0 {
            tries.extend((0..=fuzz.min(leading.max(trailing))).map(|f| (f, Try::Near)));
        }
        let mut placed = None;
        for (f, how) in tries {
            let (drop_lead, drop_trail) = (f.min(leading), f.min(trailing));
            let body = &hunk.lines[drop_lead..hunk.lines.len() - drop_trail];
            let pre: Vec<&PatchLine> = body.iter().filter(|l| l.edit != Edit::Insert).collect();
            if done + pre.len() > n {
                continue;
            }
            let (lo, hi) = (done, n - pre.len());
            let anchored = how == Try::Anchored && !unanchored;
            let begin = anchored && hunk.old_start <= 1;
            let end = anchored && trailing == 0;
            let fits = |p: usize| {
                (!begin || p == 0)
                    && (!end || p == hi)
                    && pre.iter().enumerate().all(|(k, l)| same(p + k, l))
            };
            let at = base + drop_lead as isize + offset;
            let hint = at.clamp(lo as isize, hi as isize) as usize;
            let found = if how == Try::Exact {
                (at == hint as isize && fits(hint)).then_some(hint)
            } else if begin || end {
                let p = if begin { 0 } else { hi };
                (p >= lo && fits(p)).then_some(p)
            } else {
                (0..=(hi - lo)).find_map(|d| {
                    let up = hint + d;
                    if up <= hi && fits(up) {
                        return Some(up);
                    }
                    let down = hint.checked_sub(d).filter(|p| *p >= lo)?;
                    (d > 0 && fits(down)).then_some(down)
                })
            };
            if let Some(p) = found {
                placed = Some((p, drop_lead, drop_trail, pre.len()));
                break;
            }
        }
        let Some((p, drop_lead, drop_trail, taken)) = placed else {
            rejected.push(h);
            continue;
        };
        out.extend((done..p).map(Placed::File));
        out.extend(
            hunk.lines[drop_lead..hunk.lines.len() - drop_trail]
                .iter()
                .filter(|l| l.edit != Edit::Delete)
                .map(Placed::Patch),
        );
        done = p + taken;
        offset = p as isize - (base + drop_lead as isize);
    }
    out.extend((done..n).map(Placed::File));
    let mut result = Vec::with_capacity(text.len());
    let last = out.len();
    for (k, placed) in out.iter().enumerate() {
        let (content, open) = match placed {
            Placed::File(i) => (lines.piece(*i), lines.unterminated(*i)),
            Placed::Patch(l) => (l.text.as_slice(), l.no_newline),
        };
        result.extend_from_slice(content);
        if !open || k + 1 < last {
            result.push(NEWLINE);
        }
    }
    Ok(Applied {
        text: result,
        rejected,
    })
}

/// Deletes after inserts are put back in front of them, as a diff writes them.
fn reorder(lines: &mut [PatchLine]) {
    let mut i = 0;
    while i < lines.len() {
        if lines[i].edit == Edit::Equal {
            i += 1;
            continue;
        }
        let start = i;
        while i < lines.len() && lines[i].edit != Edit::Equal {
            i += 1;
        }
        lines[start..i].sort_by_key(|l| l.edit == Edit::Insert);
    }
}

/// The other way round, a run's deletes first as git writes them.
pub(super) fn reverse(f: &PatchFile) -> PatchFile {
    let mut r = flip(f);
    for h in &mut r.hunks {
        reorder(&mut h.lines);
    }
    r
}

/// Sides swapped with every line where it was, so forward line indices still address it.
fn flip(f: &PatchFile) -> PatchFile {
    let mut r = f.clone();
    std::mem::swap(&mut r.old_path, &mut r.new_path);
    std::mem::swap(&mut r.old_mode, &mut r.new_mode);
    r.status = match f.status {
        Status::Added => Status::Deleted,
        Status::Deleted => Status::Added,
        s => s,
    };
    if let Some((old, new)) = &f.labels {
        r.labels = Some((new.clone(), old.clone()));
    }
    if let Some(git) = &mut r.git {
        if let Some((a, b)) = &git.index {
            git.index = Some((b.clone(), a.clone()));
        }
    }
    for h in &mut r.hunks {
        std::mem::swap(&mut h.old_start, &mut h.new_start);
        std::mem::swap(&mut h.old_count, &mut h.new_count);
        for l in &mut h.lines {
            l.edit = match l.edit {
                Edit::Delete => Edit::Insert,
                Edit::Insert => Edit::Delete,
                Edit::Equal => Edit::Equal,
            };
        }
    }
    r
}

#[derive(Clone, Default)]
pub(super) struct Pick {
    pub(super) whole: bool,
    pub(super) hunks: BTreeSet<usize>,
    pub(super) lines: BTreeSet<(usize, usize)>,
}

/// A hunk cut down to some of its lines: an unpicked delete stays as context, an unpicked insert
/// goes. `None` when nothing changes then.
fn part(hunk: &PatchHunk, h: usize, picked: &BTreeSet<(usize, usize)>) -> Option<Vec<PatchLine>> {
    let mut lines = Vec::with_capacity(hunk.lines.len());
    for (k, l) in hunk.lines.iter().enumerate() {
        let chosen = picked.contains(&(h, k));
        match l.edit {
            Edit::Equal => lines.push(l.clone()),
            Edit::Delete if chosen => lines.push(l.clone()),
            Edit::Delete => lines.push(PatchLine {
                edit: Edit::Equal,
                ..l.clone()
            }),
            Edit::Insert if chosen => lines.push(l.clone()),
            Edit::Insert => {}
        }
    }
    if lines.iter().all(|l| l.edit == Edit::Equal) {
        return None;
    }
    // The old last line, now kept, has no newline but lines are added after it: it gets one.
    if let Some(k) = lines
        .iter()
        .position(|l| l.edit == Edit::Equal && l.no_newline)
    {
        if lines[k + 1..].iter().any(|l| l.edit == Edit::Insert) {
            let kept = PatchLine {
                edit: Edit::Insert,
                text: lines[k].text.clone(),
                no_newline: false,
            };
            lines[k].edit = Edit::Delete;
            lines.insert(k + 1, kept);
        }
    }
    Some(lines)
}

/// The files, hunks and lines picked, with the new line numbers counted again. With `reverse`, the
/// patch that takes the picked lines back out of the new side, picked by their forward indices.
pub(super) fn select(
    files: &[PatchFile],
    picks: &BTreeMap<usize, Pick>,
    reverse: bool,
) -> Vec<PatchFile> {
    let mut out = Vec::new();
    for (&index, pick) in picks {
        let flipped;
        let file = if reverse {
            flipped = flip(&files[index]);
            &flipped
        } else {
            &files[index]
        };
        let mut f = file.clone();
        f.hunks.clear();
        let mut delta = 0isize;
        for (h, hunk) in file.hunks.iter().enumerate() {
            let mut lines = if pick.whole || pick.hunks.contains(&h) {
                hunk.lines.clone()
            } else {
                match part(hunk, h, &pick.lines) {
                    Some(lines) => lines,
                    None => continue,
                }
            };
            if reverse {
                reorder(&mut lines);
            }
            let old_count = lines.iter().filter(|l| l.edit != Edit::Insert).count();
            let new_count = lines.iter().filter(|l| l.edit != Edit::Delete).count();
            let old_at = if hunk.old_count == 0 {
                hunk.old_start
            } else {
                hunk.old_start.saturating_sub(1)
            };
            let new_at = (old_at as isize + delta).max(0) as usize;
            delta += new_count as isize - old_count as isize;
            f.hunks.push(PatchHunk {
                old_start: hunk.old_start,
                old_count,
                new_start: if new_count == 0 { new_at } else { new_at + 1 },
                new_count,
                section: hunk.section.clone(),
                lines,
            });
        }
        if f.binary.is_none()
            && f.hunks.is_empty()
            && f.status == Status::Modified
            && f.old_mode == f.new_mode
        {
            continue;
        }
        out.push(f);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn u(text: &str) -> Vec<u16> {
        text.encode_utf16().collect()
    }

    fn s(units: &[u16]) -> String {
        String::from_utf16(units).unwrap()
    }

    fn patch(old: &str, new: &str) -> PatchFile {
        from_texts(&u(old), &u(new), &Options::default(), None)
    }

    fn applied(file: &PatchFile, text: &str, fuzz: usize) -> (String, Vec<usize>) {
        let a = apply(file, &u(text), fuzz).unwrap();
        (s(&a.text), a.rejected)
    }

    fn rng(mut seed: u64) -> impl FnMut(u64) -> u64 {
        move |below| {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            seed % below
        }
    }

    fn random_text(next: &mut impl FnMut(u64) -> u64, lines: u64) -> String {
        const WORDS: [&str; 8] = [
            "alpha",
            "}",
            "",
            "  return x;\r",
            "Привет",
            "𝄞 clef",
            "{",
            "beta",
        ];
        let parts: Vec<&str> = (0..next(lines + 1))
            .map(|_| WORDS[next(8) as usize])
            .collect();
        let mut text = parts.join("\n");
        if next(3) > 0 && !parts.is_empty() {
            text.push('\n');
        }
        text
    }

    #[test]
    fn a_patch_from_two_texts_applies_and_reverses_on_varied_inputs() {
        let mut next = rng(0x9e37_79b9_7f4a_7c15);
        let fixed = [
            ("", ""),
            ("", "a\n"),
            ("a\n", ""),
            ("a", "a\n"),
            ("a\n", "a"),
            ("a", "b"),
            ("x\r\ny\r\n", "x\r\nz\r\n"),
            ("x\r\ny\r\n", "x\ny\n"),
            ("😀\n𝄞\n", "😀\n𝄢\n"),
            ("one\ntwo\nthree", "one\n2\nthree"),
        ];
        let mut cases: Vec<(String, String)> = fixed
            .iter()
            .map(|(a, b)| (a.to_string(), b.to_string()))
            .collect();
        for _ in 0..1500 {
            let a = random_text(&mut next, 30);
            let b = random_text(&mut next, 30);
            cases.push((a, b));
        }
        for (a, b) in &cases {
            for context in [0, 1, 3] {
                for algorithm in [
                    diff::Algorithm::Histogram,
                    diff::Algorithm::Myers,
                    diff::Algorithm::Patience,
                ] {
                    let o = Options {
                        context,
                        algorithm,
                        ..Options::default()
                    };
                    let file = from_texts(&u(a), &u(b), &o, None);
                    assert_eq!(
                        applied(&file, a, 0),
                        (b.clone(), vec![]),
                        "{a:?} -> {b:?} {o:?}"
                    );
                    let back = reverse(&file);
                    assert_eq!(
                        applied(&back, b, 0),
                        (a.clone(), vec![]),
                        "reverse of {a:?} -> {b:?}"
                    );
                    let printed = print(std::slice::from_ref(&file));
                    let parsed = parse(&printed).unwrap();
                    if file.hunks.is_empty() {
                        assert!(parsed.is_empty());
                    } else {
                        assert_eq!(parsed[0].hunks, file.hunks, "{}", s(&printed));
                    }
                }
            }
        }
    }

    #[test]
    fn a_large_file_round_trips() {
        let mut next = rng(42);
        let old: Vec<String> = (0..20_000)
            .map(|i| format!("line {i} {}", next(50)))
            .collect();
        let mut new = old.clone();
        for _ in 0..300 {
            let at = next(new.len() as u64) as usize;
            if next(2) == 0 {
                new[at] = format!("changed {}", next(1000));
            } else {
                new.insert(at, "inserted".into());
            }
        }
        let (a, b) = (old.join("\n") + "\n", new.join("\n"));
        let file = patch(&a, &b);
        assert_eq!(applied(&file, &a, 0), (b.clone(), vec![]));
        let reparsed = parse(&print(&[file])).unwrap();
        assert_eq!(applied(&reparsed[0], &a, 0).0, b);
    }

    #[test]
    fn no_newline_markers_are_printed_and_read() {
        let file = patch("a\nb", "a\nc");
        let text = s(&print(std::slice::from_ref(&file)));
        assert_eq!(
            text,
            "@@ -1,2 +1,2 @@\n a\n-b\n\\ No newline at end of file\n+c\n\\ No newline at end of file\n"
        );
        let file = patch("a\n", "a");
        assert_eq!(
            s(&print(&[file])),
            "@@ -1 +1 @@\n-a\n+a\n\\ No newline at end of file\n"
        );
    }

    #[test]
    fn a_hunk_is_found_away_from_its_line_and_rejected_when_it_is_not_there() {
        let base = "a\nb\nc\nd\ne\nf\ng\nh\ni\nj\n";
        let file = patch(base, "a\nb\nc\nd\nE\nf\ng\nh\ni\nj\n");
        let moved = format!("new 1\nnew 2\n{base}");
        assert_eq!(
            applied(&file, &moved, 0),
            (
                format!("new 1\nnew 2\na\nb\nc\nd\nE\nf\ng\nh\ni\nj\n"),
                vec![]
            )
        );
        let lost = "a\nb\nc\nX\ne\nf\ng\nh\ni\nj\n";
        assert_eq!(applied(&file, lost, 0), (lost.to_string(), vec![0]));
        // One changed context line is too much without fuzz and fine with it.
        let fuzzy = "a\nb\nc\nd\ne\nf\nG\nh\ni\nj\n";
        assert_eq!(applied(&file, fuzzy, 0).1, vec![0]);
        assert_eq!(
            applied(&file, fuzzy, 2),
            ("a\nb\nc\nd\nE\nf\nG\nh\ni\nj\n".to_string(), vec![])
        );
    }

    #[test]
    fn later_hunks_still_apply_after_a_rejected_one() {
        let old: String = (0..30).map(|i| format!("line {i}\n")).collect();
        let new = old
            .replace("line 3\n", "three\n")
            .replace("line 25\n", "twenty-five\n");
        let file = patch(&old, &new);
        assert_eq!(file.hunks.len(), 2);
        let changed = old.replace("line 2\n", "two\n");
        let (text, rejected) = applied(&file, &changed, 0);
        assert_eq!(rejected, vec![0]);
        assert!(text.contains("twenty-five\n") && text.contains("two\nline 3\n"));
    }

    #[test]
    fn a_hunk_at_the_start_or_end_must_stay_there() {
        let file = patch("a\nb\nc\nd\n", "a\nb\nc\nd\ne\n");
        assert_eq!(applied(&file, "z\na\nb\nc\nd\nx\n", 0).1, vec![0]);
        assert_eq!(
            applied(&file, "z\na\nb\nc\nd\nx\n", 1).0,
            "z\na\nb\nc\nd\ne\nx\n"
        );
        assert_eq!(applied(&file, "a\nb\nc\nd\nx\n", 0).0, "a\nb\nc\nd\ne\nx\n");
        let file = patch("a\nb\nc\nd\n", "new\na\nb\nc\nd\n");
        assert_eq!(applied(&file, "x\na\nb\nc\nd\n", 0).1, vec![0]);
    }

    fn picked(files: &[PatchFile], pick: Pick) -> Vec<PatchFile> {
        select(files, &BTreeMap::from([(0, pick)]), false)
    }

    #[test]
    fn a_reversed_selection_takes_exactly_the_picked_lines_out_of_the_new_text() {
        let mut next = rng(0x5eed_f00d);
        let mut cases: Vec<(String, String)> = [
            ("a\nb\n", "c\nd\n"),
            ("a", "a\nb"),
            ("a\nb", "a"),
            ("x\n", "y"),
        ]
        .iter()
        .map(|(a, b)| (a.to_string(), b.to_string()))
        .collect();
        for _ in 0..1500 {
            cases.push((random_text(&mut next, 20), random_text(&mut next, 20)));
        }
        for (a, b) in &cases {
            let o = Options {
                context: next(4) as usize,
                ..Options::default()
            };
            let files = vec![from_texts(&u(a), &u(b), &o, None)];
            let (mut some, mut rest) = (Pick::default(), Pick::default());
            for (h, hunk) in files[0].hunks.iter().enumerate() {
                for k in 0..hunk.lines.len() {
                    if next(2) == 0 {
                        some.lines.insert((h, k));
                    } else {
                        rest.lines.insert((h, k));
                    }
                }
            }
            let target = picked(&files, rest)
                .first()
                .map_or(a.clone(), |p| applied(p, a, 0).0);
            if let Some(forward) =
                select(&files, &BTreeMap::from([(0, some.clone())]), false).first()
            {
                let staged = applied(forward, a, 0).0;
                assert_eq!(applied(&reverse(forward), &staged, 0), (a.clone(), vec![]));
            }
            let undo = select(&files, &BTreeMap::from([(0, some)]), true);
            let back = undo.first().map_or(b.clone(), |p| {
                let (text, rejected) = applied(p, b, 0);
                assert!(rejected.is_empty(), "{a:?} -> {b:?}: {}", s(&print(&undo)));
                let reparsed = parse(&print(std::slice::from_ref(p))).unwrap();
                assert_eq!(applied(&reparsed[0], b, 0).0, text);
                text
            });
            assert_eq!(back, target, "{a:?} -> {b:?}: {}", s(&print(&undo)));
            if !files[0].hunks.is_empty() {
                let whole = Pick {
                    whole: true,
                    ..Pick::default()
                };
                let all = select(&files, &BTreeMap::from([(0, whole)]), true);
                assert_eq!(all, vec![reverse(&files[0])]);
            }
        }
    }

    #[test]
    fn selecting_hunks_and_lines_makes_patches_that_apply() {
        let old: String = (0..40).map(|i| format!("line {i}\n")).collect();
        let new = old
            .replace("line 5\n", "five\n")
            .replace("line 20\n", "twenty\nextra\n")
            .replace("line 35\n", "");
        let files = vec![patch(&old, &new)];
        assert_eq!(files[0].hunks.len(), 3);
        let second = picked(
            &files,
            Pick {
                hunks: BTreeSet::from([1]),
                ..Pick::default()
            },
        );
        assert_eq!(second[0].hunks[0].new_start, second[0].hunks[0].old_start);
        let expected = old.replace("line 20\n", "twenty\nextra\n");
        assert_eq!(applied(&second[0], &old, 0), (expected, vec![]));
        let third = picked(
            &files,
            Pick {
                hunks: BTreeSet::from([2]),
                ..Pick::default()
            },
        );
        assert_eq!(applied(&third[0], &old, 0).0, old.replace("line 35\n", ""));
        let hunk = &files[0].hunks[1];
        let extra = hunk
            .lines
            .iter()
            .position(|l| s(&l.text) == "extra")
            .unwrap();
        let line = picked(
            &files,
            Pick {
                lines: BTreeSet::from([(1, extra)]),
                ..Pick::default()
            },
        );
        let with_extra = old.replace("line 20\n", "line 20\nextra\n");
        assert_eq!(applied(&line[0], &old, 0), (with_extra.clone(), vec![]));
        let minus = hunk
            .lines
            .iter()
            .position(|l| l.edit == Edit::Delete)
            .unwrap();
        let only_delete = picked(
            &files,
            Pick {
                lines: BTreeSet::from([(1, minus)]),
                ..Pick::default()
            },
        );
        assert_eq!(
            applied(&only_delete[0], &old, 0).0,
            old.replace("line 20\n", "")
        );
        let printed = print(&line);
        assert_eq!(applied(&parse(&printed).unwrap()[0], &old, 0).0, with_extra);
        let both = picked(
            &files,
            Pick {
                hunks: BTreeSet::from([0]),
                lines: BTreeSet::from([(2, 3)]),
                ..Pick::default()
            },
        );
        assert_eq!(both[0].hunks.len(), 2);
        assert_eq!(
            applied(&both[0], &old, 0).0,
            old.replace("line 5\n", "five\n").replace("line 35\n", "")
        );
        assert!(picked(&files, Pick::default()).is_empty());
    }

    #[test]
    fn any_selection_of_lines_applies_cleanly_and_all_of_them_is_the_whole_patch() {
        let mut next = rng(0x1234_5678);
        for _ in 0..1500 {
            let a = random_text(&mut next, 20);
            let b = random_text(&mut next, 20);
            let o = Options {
                context: next(4) as usize,
                ..Options::default()
            };
            let files = vec![from_texts(&u(&a), &u(&b), &o, None)];
            let mut some = Pick::default();
            let mut all = Pick::default();
            for (h, hunk) in files[0].hunks.iter().enumerate() {
                for k in 0..hunk.lines.len() {
                    all.lines.insert((h, k));
                    if next(2) == 0 {
                        some.lines.insert((h, k));
                    }
                }
            }
            if let Some(part) = picked(&files, some).first() {
                let (text, rejected) = applied(part, &a, 0);
                assert!(
                    rejected.is_empty(),
                    "{a:?} -> {b:?}: {}\nfrom\n{}",
                    s(&print(&[part.clone()])),
                    s(&print(&files))
                );
                let reparsed = parse(&print(std::slice::from_ref(part))).unwrap();
                assert_eq!(applied(&reparsed[0], &a, 0).0, text);
            }
            if !files[0].hunks.is_empty() {
                let whole = picked(&files, all);
                assert_eq!(applied(&whole[0], &a, 0), (b.clone(), vec![]));
            }
        }
    }

    #[test]
    fn selecting_lines_around_a_missing_final_newline() {
        let files = vec![patch("a", "a\nb")];
        assert_eq!(files[0].hunks[0].lines.len(), 3);
        let plus_b = picked(
            &files,
            Pick {
                lines: BTreeSet::from([(0, 2)]),
                ..Pick::default()
            },
        );
        assert_eq!(applied(&plus_b[0], "a", 0), ("a\nb".to_string(), vec![]));
        let minus_a = picked(
            &files,
            Pick {
                lines: BTreeSet::from([(0, 0)]),
                ..Pick::default()
            },
        );
        assert_eq!(applied(&minus_a[0], "a", 0), ("".to_string(), vec![]));
        let again = print(&plus_b);
        assert_eq!(applied(&parse(&again).unwrap()[0], "a", 0).0, "a\nb");
    }

    const GIT_SHOW: &str = r#"commit 1f0c
Author: A <a@b.c>

    message
    --- not a diff

diff --git a/src/lib.rs b/src/lib.rs
index 83db48f..bf269f4 100644
--- a/src/lib.rs
+++ b/src/lib.rs
@@ -1,3 +1,3 @@ fn main() {
 one
-two
+2
 three
@@ -10 +10,2 @@
 ten
+eleven
diff --git a/old name.txt b/new name.txt
similarity index 90%
rename from old name.txt
rename to new name.txt
index 1111111..2222222 100644
--- a/old name.txt<TAB>
+++ b/new name.txt<TAB>
@@ -1 +1 @@
-x
+y
diff --git a/run.sh b/run.sh
old mode 100644
new mode 100755
diff --git a/logo.png b/logo.png
new file mode 100644
index 0000000..3b18e51
Binary files /dev/null and b/logo.png differ
diff --git a/gone.txt b/gone.txt
deleted file mode 100644
index e69de29..0000000
diff --git "a/caf\303\251.txt" "b/caf\303\251.txt"
new file mode 100644
index 0000000..257cc56
--- /dev/null
+++ "b/caf\303\251.txt"
@@ -0,0 +1 @@
+foo
diff --git a/bin.dat b/bin.dat
index 1234567..89abcde 100644
GIT binary patch
literal 3
KcmZ?wW&i>K

literal 0
HcmV?d00001

-- <END>
2.47.0
"#;

    /// `git show` output: a commit, then a modified file, a rename, a mode change, binary files,
    /// a deletion and a quoted name.
    fn git_show() -> String {
        GIT_SHOW.replace("<TAB>", "\t").replace("<END>", "")
    }

    #[test]
    fn real_git_output_is_parsed_and_printed_back() {
        let git = git_show();
        let files = parse(&u(&git)).unwrap();
        let summary: Vec<_> = files
            .iter()
            .map(|f| {
                (
                    f.old_path.clone(),
                    f.new_path.clone(),
                    f.status,
                    f.old_mode,
                    f.new_mode,
                    f.binary.is_some(),
                    f.hunks.len(),
                )
            })
            .collect();
        let p = |s: &str| Some(s.to_string());
        assert_eq!(
            summary,
            [
                (
                    p("src/lib.rs"),
                    p("src/lib.rs"),
                    Status::Modified,
                    Some(0o100644),
                    Some(0o100644),
                    false,
                    2
                ),
                (
                    p("old name.txt"),
                    p("new name.txt"),
                    Status::Renamed,
                    Some(0o100644),
                    Some(0o100644),
                    false,
                    1
                ),
                (
                    p("run.sh"),
                    p("run.sh"),
                    Status::Modified,
                    Some(0o100644),
                    Some(0o100755),
                    false,
                    0
                ),
                (
                    None,
                    p("logo.png"),
                    Status::Added,
                    None,
                    Some(0o100644),
                    true,
                    0
                ),
                (
                    p("gone.txt"),
                    None,
                    Status::Deleted,
                    Some(0o100644),
                    None,
                    false,
                    0
                ),
                (
                    None,
                    p("café.txt"),
                    Status::Added,
                    None,
                    Some(0o100644),
                    false,
                    1
                ),
                (
                    p("bin.dat"),
                    p("bin.dat"),
                    Status::Modified,
                    Some(0o100644),
                    Some(0o100644),
                    true,
                    0
                ),
            ]
        );
        assert_eq!(s(&files[0].hunks[0].section), "fn main() {");
        assert_eq!(
            (files[0].hunks[1].old_count, files[0].hunks[1].new_count),
            (1, 2)
        );
        assert_eq!(files[1].git.as_ref().unwrap().similarity, Some(90));
        let start = git.find("diff --git").unwrap();
        let end = git.find("-- \n2.47").unwrap();
        assert_eq!(s(&print(&files)), git[start..end]);
        let modified = apply(&files[0], &u("one\ntwo\nthree\n4\n5\n6\n7\n8\n9\nten\n"), 0).unwrap();
        assert_eq!(
            s(&modified.text),
            "one\n2\nthree\n4\n5\n6\n7\n8\n9\nten\neleven\n"
        );
        assert!(apply(&files[3], &[], 0).is_err());
    }

    #[test]
    fn a_reversed_git_patch_swaps_sides() {
        let files = parse(&u(&git_show())).unwrap();
        let back: Vec<PatchFile> = files.iter().map(reverse).collect();
        assert_eq!(back[3].status, Status::Deleted);
        assert_eq!(back[3].old_path.as_deref(), Some("logo.png"));
        assert_eq!(back[1].old_path.as_deref(), Some("new name.txt"));
        let text = s(&print(&back[..3]));
        assert!(text.contains("@@ -1,3 +1,3 @@ fn main() {\n one\n-2\n+two\n three\n"));
        assert!(text.contains(
            "rename from new name.txt\nrename to old name.txt\nindex 2222222..1111111 100644\n"
        ));
        assert!(text.contains("old mode 100755\nnew mode 100644\n"));
        let twice: Vec<PatchFile> = back.iter().map(reverse).collect();
        assert_eq!(twice, files);
    }

    #[test]
    fn malformed_hunks_are_errors_with_a_line_number() {
        assert!(parse(&u("@@ -1,2 +1,2 @@\n a\n"))
            .unwrap_err()
            .starts_with("line 1:"));
        assert!(parse(&u("@@ -1 +1 @@\n a\n+b\n")).is_ok());
        assert!(parse(&u("@@ -1,2 +1,2 @@\n a\n*b\n"))
            .unwrap_err()
            .starts_with("line 3:"));
        assert!(parse(&u("@@ -x +1 @@\n")).is_err());
        assert!(parse(&u("no patch here\n")).unwrap().is_empty());
        let blank = parse(&u("@@ -1,3 +1,3 @@\n a\n\n-b\n+c\n")).unwrap();
        assert_eq!(blank[0].hunks[0].lines[1].text, Vec::<u16>::new());
    }
}
