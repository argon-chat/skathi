//! VNPK v1, the app payload a packaged exe carries, and the one function that turns a URL into an
//! asset key.
//!
//! `vn build` writes one blob, stamps it into the bare host as the `RT_RCDATA` resource `VNPAYLOAD`,
//! and the host maps it zero-copy at boot. This crate is the reader; a golden fixture
//! (`packages/vue-native/test/fixtures/golden.vnpk`) ties it to the writer
//! (`packages/vue-native/cli/vnpk.ts`). Layout:
//!
//! ```text
//! header 64 B | toc (32 B × n, sorted by (kind, name bytes)) | names | skew | data
//! ```
//!
//! Every field is read byte-wise: the blob is only 4-byte aligned in the image.
//!
//! [`Payload::parse`] validates once, in O(entries): magic, format, TOC bounds and order, UTF-8
//! names and text entries. Asset bytes are never read at boot, since that would fault in every page.
//!
//! [`content_key`] is the normaliser both asset paths use (redjs's content provider and
//! `vn_tree::img`), in dev and packaged, so they agree on what `assets/a.png?v=2` names.

use std::fmt;
use std::ops::Range;

pub const MAGIC: &[u8; 4] = b"VNPK";
/// The newest format this host reads; a higher number was built by a newer `vn`.
pub const FORMAT: u16 = 1;
pub const HEADER_SIZE: usize = 64;
pub const TOC_ENTRY_SIZE: usize = 32;

/// Entry kinds. Only [`kind::ASSET`] is reachable from app code: internal entries are looked up by
/// kind and name, so `fetch("vn/app.js")` is a miss.
pub mod kind {
    pub const META: u16 = 1;
    pub const WINDOW: u16 = 2;
    pub const SCRIPT: u16 = 3;
    /// Reserved (a JSC bytecode cache). Page-aligned when present.
    pub const BYTECODE: u16 = 4;
    pub const ASSET: u16 = 16;
}

/// The names the internal entries are written under.
pub mod name {
    pub const META: &str = "vn/app.json";
    pub const WINDOW: &str = "vn/window.json";
    pub const SCRIPT: &str = "vn/app.js";
    pub const BYTECODE: &str = "vn/app.bc";
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    TooShort(usize),
    BadMagic,
    /// Built by a newer `vn` than this host understands.
    NewerFormat(u16),
    OutOfBounds {
        what: String,
        start: u64,
        len: u64,
        blob: usize,
    },
    BadUtf8(String),
    Unsorted {
        index: usize,
        name: String,
    },
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::TooShort(n) => write!(f, "the payload is {n} bytes, shorter than its 64-byte header"),
            Error::BadMagic => write!(f, "the payload does not start with \"VNPK\""),
            Error::NewerFormat(v) => write!(
                f,
                "the payload is format {v}, and this host reads format {FORMAT} — it was built by a newer vn; \
                 rebuild the app with the vn that matches this host"
            ),
            Error::OutOfBounds { what, start, len, blob } => {
                write!(f, "{what} [{start}, +{len}) lies outside the {blob}-byte payload")
            }
            Error::BadUtf8(what) => write!(f, "{what} is not valid UTF-8"),
            Error::Unsorted { index, name } => {
                write!(f, "the table of contents is not sorted at entry {index} ({name:?})")
            }
        }
    }
}

impl std::error::Error for Error {}

/// One table-of-contents entry, resolved to slices of the blob.
#[derive(Debug, Clone, Copy)]
pub struct Entry<'a> {
    pub name: &'a str,
    pub kind: u16,
    pub data: &'a [u8],
}

/// A validated payload, borrowing the blob (`'static` in the host: the mapped image).
#[derive(Debug)]
pub struct Payload<'a> {
    bytes: &'a [u8],
    entries: Vec<Entry<'a>>,
    /// The contiguous run of [`kind::ASSET`] entries, sorted by name bytes, for the binary search.
    assets: Range<usize>,
    flags: u16,
    skew: u32,
    data_off: u64,
}

fn u16_at(b: &[u8], o: usize) -> u16 {
    u16::from_le_bytes([b[o], b[o + 1]])
}
fn u32_at(b: &[u8], o: usize) -> u32 {
    u32::from_le_bytes([b[o], b[o + 1], b[o + 2], b[o + 3]])
}
fn u64_at(b: &[u8], o: usize) -> u64 {
    let mut a = [0u8; 8];
    a.copy_from_slice(&b[o..o + 8]);
    u64::from_le_bytes(a)
}

impl<'a> Payload<'a> {
    /// Validates `bytes` and builds the entry view. O(entries); asset bytes are
    /// never touched.
    pub fn parse(bytes: &'a [u8]) -> Result<Self, Error> {
        if bytes.len() < HEADER_SIZE {
            return Err(Error::TooShort(bytes.len()));
        }
        if &bytes[0..4] != MAGIC {
            return Err(Error::BadMagic);
        }
        let format = u16_at(bytes, 4);
        if format > FORMAT {
            return Err(Error::NewerFormat(format));
        }
        let flags = u16_at(bytes, 6);
        let count = u32_at(bytes, 8) as u64;
        let toc_off = u32_at(bytes, 12) as u64;
        let names_off = u32_at(bytes, 16) as u64;
        let names_len = u32_at(bytes, 20) as u64;
        let skew = u32_at(bytes, 24);
        let data_off = u64_at(bytes, 32);
        let blob_len = u64_at(bytes, 40);

        // A resource can be longer than what was written into it, never shorter.
        let len = bytes.len() as u64;
        let within = |what: &str, start: u64, n: u64| -> Result<(), Error> {
            match start.checked_add(n) {
                Some(end) if end <= len.min(blob_len.max(HEADER_SIZE as u64)) => Ok(()),
                _ => Err(Error::OutOfBounds {
                    what: what.to_string(),
                    start,
                    len: n,
                    blob: bytes.len(),
                }),
            }
        };
        if blob_len > len {
            return Err(Error::OutOfBounds {
                what: "blob_len".into(),
                start: 0,
                len: blob_len,
                blob: bytes.len(),
            });
        }
        within(
            "the table of contents",
            toc_off,
            count.saturating_mul(TOC_ENTRY_SIZE as u64),
        )?;
        within("the names", names_off, names_len)?;
        within("the data region", data_off, 0)?;

        let mut entries = Vec::with_capacity(count as usize);
        for i in 0..count as usize {
            let t = (toc_off as usize) + i * TOC_ENTRY_SIZE;
            let name_off = u32_at(bytes, t) as u64;
            let name_len = u32_at(bytes, t + 4) as u64;
            let off = u64_at(bytes, t + 8);
            let n = u64_at(bytes, t + 16);
            let kind = u16_at(bytes, t + 24);
            if name_off.saturating_add(name_len) > names_len {
                return Err(Error::OutOfBounds {
                    what: format!("entry {i}'s name"),
                    start: names_off + name_off,
                    len: name_len,
                    blob: bytes.len(),
                });
            }
            let ns = (names_off + name_off) as usize;
            let name = std::str::from_utf8(&bytes[ns..ns + name_len as usize])
                .map_err(|_| Error::BadUtf8(format!("entry {i}'s name")))?;
            let start = data_off
                .checked_add(off)
                .ok_or_else(|| Error::OutOfBounds {
                    what: format!("{name:?}"),
                    start: off,
                    len: n,
                    blob: bytes.len(),
                })?;
            within(&format!("the bytes of {name:?}"), start, n)?;
            let data = &bytes[start as usize..(start + n) as usize];
            if matches!(kind, kind::META | kind::WINDOW | kind::SCRIPT)
                && std::str::from_utf8(data).is_err()
            {
                return Err(Error::BadUtf8(format!("{name:?}")));
            }
            entries.push(Entry { name, kind, data });
        }
        for i in 1..entries.len() {
            let (a, b) = (&entries[i - 1], &entries[i]);
            if (a.kind, a.name.as_bytes()) >= (b.kind, b.name.as_bytes()) {
                return Err(Error::Unsorted {
                    index: i,
                    name: b.name.to_string(),
                });
            }
        }
        let first = entries
            .iter()
            .position(|e| e.kind == kind::ASSET)
            .unwrap_or(entries.len());
        let last = entries
            .iter()
            .rposition(|e| e.kind == kind::ASSET)
            .map_or(first, |i| i + 1);
        Ok(Payload {
            bytes,
            entries,
            assets: first..last,
            flags,
            skew,
            data_off,
        })
    }

    fn text(&self, kind: u16, name: &str) -> Option<&'a str> {
        let e = self
            .entries
            .iter()
            .find(|e| e.kind == kind && e.name == name)?;
        // Validated in `parse`.
        std::str::from_utf8(e.data).ok()
    }

    /// `vn/app.json`.
    pub fn meta(&self) -> Option<&'a str> {
        self.text(kind::META, name::META)
    }

    /// `vn/window.json`.
    pub fn window_json(&self) -> Option<&'a str> {
        self.text(kind::WINDOW, name::WINDOW)
    }

    /// `vn/app.js`.
    pub fn script(&self) -> Option<&'a str> {
        self.text(kind::SCRIPT, name::SCRIPT)
    }

    pub fn assets(&self) -> &[Entry<'a>] {
        &self.entries[self.assets.clone()]
    }

    pub fn asset_count(&self) -> usize {
        self.assets.len()
    }

    /// The asset stored under exactly `key`; a binary search over the sorted run.
    pub fn asset(&self, key: &str) -> Option<&'a [u8]> {
        let run = self.assets();
        run.binary_search_by(|e| e.name.as_bytes().cmp(key.as_bytes()))
            .ok()
            .map(|i| run[i].data)
    }

    /// The asset whose name equals `key` ignoring ASCII case, with the stored spelling. A linear
    /// scan for the miss path: NTFS is case-insensitive in dev, so a packaged exe must not be stricter.
    pub fn asset_ignoring_case(&self, key: &str) -> Option<(&'a str, &'a [u8])> {
        self.assets()
            .iter()
            .find(|e| e.name.eq_ignore_ascii_case(key))
            .map(|e| (e.name, e.data))
    }

    pub fn entries(&self) -> &[Entry<'a>] {
        &self.entries
    }

    pub fn len(&self) -> usize {
        self.bytes.len()
    }

    pub fn is_empty(&self) -> bool {
        self.bytes.is_empty()
    }

    pub fn flags(&self) -> u16 {
        self.flags
    }

    pub fn skew(&self) -> u32 {
        self.skew
    }

    /// Where the data region starts, relative to the blob.
    pub fn data_offset(&self) -> u64 {
        self.data_off
    }

    /// The header, TOC and names: names the content (TOC carries lengths and crc32) without reading
    /// it; an asset pack's identity is a hash of this.
    pub fn index(&self) -> &'a [u8] {
        let end = self.data_off.saturating_sub(u64::from(self.skew)).min(self.bytes.len() as u64);
        &self.bytes[..(end as usize).max(HEADER_SIZE.min(self.bytes.len()))]
    }

    /// The blob's own address — for the alignment report in `--vn-host-info`.
    pub fn as_ptr(&self) -> *const u8 {
        self.bytes.as_ptr()
    }
}

/// Why a URL has no key.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyError {
    /// Nothing left after cleaning (`""`, `"/"`, `"./"`, `"?x"`).
    Empty,
    /// More `..` than there are segments: the URL climbs out of the app root.
    EscapesRoot,
    /// A segment no file under the app root can have: `:`, NUL, or bytes that decode to invalid UTF-8.
    Invalid,
}

/// The app-root-relative key a relative or root-relative URL names, applied textually:
///
/// * `?query` and `#hash` are cut;
/// * `%XX` escapes are decoded;
/// * `\` is `/`, empty and `.` segments vanish, a leading `/` is the app root;
/// * `..` folds lexically; one climbing above the app root is [`KeyError::EscapesRoot`].
///
/// The result is `/`-separated and case-preserved, the key `vn build` stores a file under.
pub fn content_key_checked(url: &str) -> Result<String, KeyError> {
    let mut s = url.trim();
    if let Some(cut) = s.find(['?', '#']) {
        s = &s[..cut];
    }
    let decoded = percent_decode(s).ok_or(KeyError::Invalid)?;
    let decoded = decoded.replace('\\', "/");
    let mut segs: Vec<&str> = Vec::new();
    for seg in decoded.split('/') {
        match seg {
            "" | "." => {}
            ".." => {
                if segs.pop().is_none() {
                    return Err(KeyError::EscapesRoot);
                }
            }
            s if s.contains([':', '\0']) => return Err(KeyError::Invalid),
            s => segs.push(s),
        }
    }
    if segs.is_empty() {
        return Err(KeyError::Empty);
    }
    Ok(segs.join("/"))
}

/// [`content_key_checked`] without the reason.
pub fn content_key(url: &str) -> Option<String> {
    content_key_checked(url).ok()
}

/// `%XX` → byte; a malformed escape is kept literally (what a browser's URL
/// parser does with `%zz`). `None` when the decoded bytes are not UTF-8.
fn percent_decode(s: &str) -> Option<String> {
    if !s.contains('%') {
        return Some(s.to_string());
    }
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' && i + 2 < b.len() {
            if let (Some(h), Some(l)) = (hex(b[i + 1]), hex(b[i + 2])) {
                out.push(h * 16 + l);
                i += 3;
                continue;
            }
        }
        out.push(b[i]);
        i += 1;
    }
    String::from_utf8(out).ok()
}

fn hex(c: u8) -> Option<u8> {
    match c {
        b'0'..=b'9' => Some(c - b'0'),
        b'a'..=b'f' => Some(c - b'a' + 10),
        b'A'..=b'F' => Some(c - b'A' + 10),
        _ => None,
    }
}

/// The media type a packaged `<video>`/`<audio>` source is handed to the platform with (an entry
/// has no `Content-Type`); matches vn-video's Media Foundation `extension_for`. `""` = unknown.
pub fn media_type_for(key: &str) -> &'static str {
    let ext = key
        .rsplit_once('.')
        .map(|(_, e)| e.to_ascii_lowercase())
        .unwrap_or_default();
    match ext.as_str() {
        "mp4" | "m4v" => "video/mp4",
        "m4a" => "audio/mp4",
        "mov" => "video/quicktime",
        "webm" => "video/webm",
        "ogv" => "video/ogg",
        "ogg" | "oga" => "audio/ogg",
        "mkv" => "video/x-matroska",
        "wmv" => "video/x-ms-wmv",
        "mpg" | "mpeg" => "video/mpeg",
        "avi" => "video/x-msvideo",
        "mp3" => "audio/mpeg",
        "wav" => "audio/wav",
        "flac" => "audio/flac",
        "aac" => "audio/aac",
        _ => "",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Written by packages/vue-native/cli/vnpk.ts; the TS writer test pins it byte for byte.
    const GOLDEN: &[u8] = include_bytes!("../../../packages/vue-native/test/fixtures/golden.vnpk");

    #[test]
    fn the_golden_fixture_parses_to_what_the_writer_put_in() {
        let p = Payload::parse(GOLDEN).expect("golden.vnpk parses");
        assert_eq!(p.skew(), 100);
        assert_eq!(p.entries().len(), 7);
        assert_eq!(p.asset_count(), 4);
        assert_eq!(p.script(), Some("globalThis.__golden = 42;\n"));
        assert_eq!(p.window_json(), Some(r#"{"title":"Golden","width":640}"#));
        let meta = p.meta().unwrap();
        assert!(meta.contains(r#""name":"golden""#), "{meta}");
        assert!(meta.contains(r#""variant":"standard""#), "{meta}");
        // Sorted by NAME BYTES: "L" (0x4c) < "d" < "h" < "ü" (0xc3 0xbc).
        let names: Vec<&str> = p.assets().iter().map(|e| e.name).collect();
        assert_eq!(
            names,
            [
                "assets/Logo.PNG",
                "assets/dir/data.bin",
                "assets/hello.txt",
                "assets/\u{fc}n\u{ef}code.txt"
            ]
        );
        assert_eq!(p.asset("assets/hello.txt"), Some(&b"hello\n"[..]));
        assert_eq!(
            p.asset("assets/\u{fc}n\u{ef}code.txt"),
            Some(&b"unicode\n"[..])
        );
        let data = p.asset("assets/dir/data.bin").unwrap();
        assert_eq!(data.len(), 256);
        assert!(data.iter().enumerate().all(|(i, b)| *b as usize == i));
        // Read from the header, not recomputed.
        let names_end = u32_at(GOLDEN, 16) as u64 + u32_at(GOLDEN, 20) as u64;
        assert_eq!(p.data_offset(), names_end + 100);
        // The index is everything up to the names' end: no skew, no data.
        assert_eq!(p.index(), &GOLDEN[..names_end as usize]);
    }

    #[test]
    fn internal_entries_are_not_assets() {
        let p = Payload::parse(GOLDEN).unwrap();
        // `fetch("vn/app.js")` must be a 404, not the bundle's source.
        assert_eq!(p.asset(name::SCRIPT), None);
        assert_eq!(p.asset(name::META), None);
        assert!(p.asset_ignoring_case("VN/APP.JS").is_none());
    }

    #[test]
    fn a_case_miss_is_found_by_the_fallback_with_its_stored_spelling() {
        let p = Payload::parse(GOLDEN).unwrap();
        assert_eq!(
            p.asset("assets/logo.png"),
            None,
            "the exact lookup is exact"
        );
        let (stored, bytes) = p.asset_ignoring_case("assets/logo.png").unwrap();
        assert_eq!(stored, "assets/Logo.PNG");
        assert_eq!(&bytes[..4], &[0x89, b'P', b'N', b'G']);
        assert!(p.asset_ignoring_case("assets/nope.png").is_none());
    }

    fn mutated(f: impl FnOnce(&mut Vec<u8>)) -> Vec<u8> {
        let mut v = GOLDEN.to_vec();
        f(&mut v);
        v
    }

    #[test]
    fn what_is_refused_and_why() {
        assert_eq!(
            Payload::parse(&GOLDEN[..10]).unwrap_err(),
            Error::TooShort(10)
        );
        let bad = mutated(|v| v[0] = b'X');
        assert_eq!(Payload::parse(&bad).unwrap_err(), Error::BadMagic);
        let newer = mutated(|v| v[4] = 2);
        assert_eq!(Payload::parse(&newer).unwrap_err(), Error::NewerFormat(2));
        assert!(Error::NewerFormat(2).to_string().contains("newer vn"));
        // An entry whose length runs past the end.
        let oob = mutated(|v| {
            v[HEADER_SIZE + 16..HEADER_SIZE + 24].copy_from_slice(&(1u64 << 40).to_le_bytes())
        });
        assert!(matches!(
            Payload::parse(&oob).unwrap_err(),
            Error::OutOfBounds { .. }
        ));
        // A TOC that claims more entries than fit.
        let many = mutated(|v| v[8..12].copy_from_slice(&1000u32.to_le_bytes()));
        assert!(matches!(
            Payload::parse(&many).unwrap_err(),
            Error::OutOfBounds { .. }
        ));
        // A truncated blob (blob_len says more than there is).
        assert!(matches!(
            Payload::parse(&GOLDEN[..GOLDEN.len() - 1]).unwrap_err(),
            Error::OutOfBounds { .. }
        ));
        // Unsorted: make the last entry's kind 1.
        let unsorted = mutated(|v| {
            let t = HEADER_SIZE + 6 * TOC_ENTRY_SIZE + 24;
            v[t..t + 2].copy_from_slice(&1u16.to_le_bytes());
        });
        assert!(matches!(
            Payload::parse(&unsorted).unwrap_err(),
            Error::Unsorted { .. }
        ));
    }

    #[test]
    fn a_script_that_is_not_utf8_is_refused() {
        let p = Payload::parse(GOLDEN).unwrap();
        let script = p.entries().iter().find(|e| e.kind == kind::SCRIPT).unwrap();
        let at = script.data.as_ptr() as usize - GOLDEN.as_ptr() as usize;
        let broken = mutated(|v| v[at] = 0xff);
        assert_eq!(
            Payload::parse(&broken).unwrap_err(),
            Error::BadUtf8("\"vn/app.js\"".into())
        );
    }

    #[test]
    fn a_trailing_pad_after_blob_len_is_tolerated() {
        let padded = mutated(|v| v.extend_from_slice(&[0; 12]));
        assert!(Payload::parse(&padded).is_ok());
    }

    #[test]
    fn content_keys_are_browser_shaped() {
        let k = |s: &str| content_key(s);
        assert_eq!(k("assets/a.png").as_deref(), Some("assets/a.png"));
        assert_eq!(k("./assets/a.png").as_deref(), Some("assets/a.png"));
        assert_eq!(k("/assets/a.png").as_deref(), Some("assets/a.png"));
        assert_eq!(k("assets\\a.png").as_deref(), Some("assets/a.png"));
        assert_eq!(k("assets//a.png").as_deref(), Some("assets/a.png"));
        // A server never sees the query or the fragment.
        assert_eq!(k("assets/a.png?v=2").as_deref(), Some("assets/a.png"));
        assert_eq!(k("assets/a.png#frag").as_deref(), Some("assets/a.png"));
        // Escapes decode; a malformed one is literal; `+` is not a space in a path.
        assert_eq!(
            k("assets/my%20pic.png").as_deref(),
            Some("assets/my pic.png")
        );
        assert_eq!(
            k("assets/%E2%9C%93.txt").as_deref(),
            Some("assets/\u{2713}.txt")
        );
        assert_eq!(k("assets/100%.txt").as_deref(), Some("assets/100%.txt"));
        assert_eq!(k("assets/a+b.txt").as_deref(), Some("assets/a+b.txt"));
        // Case is preserved — the store's fallback, not the key, forgives case.
        assert_eq!(k("Assets/A.PNG").as_deref(), Some("Assets/A.PNG"));
        // Dot segments fold lexically.
        assert_eq!(k("assets/x/../a.png").as_deref(), Some("assets/a.png"));
        assert_eq!(k("assets/./a.png").as_deref(), Some("assets/a.png"));
    }

    #[test]
    fn what_has_no_key() {
        assert_eq!(content_key_checked(""), Err(KeyError::Empty));
        assert_eq!(content_key_checked("/"), Err(KeyError::Empty));
        assert_eq!(content_key_checked("./"), Err(KeyError::Empty));
        assert_eq!(content_key_checked("?x=1"), Err(KeyError::Empty));
        assert_eq!(content_key_checked("../a.png"), Err(KeyError::EscapesRoot));
        assert_eq!(
            content_key_checked("assets/../../a.png"),
            Err(KeyError::EscapesRoot)
        );
        assert_eq!(
            content_key_checked("%2e%2e/a.png"),
            Err(KeyError::EscapesRoot)
        );
        // A decoded drive letter or stream name must never become a key: in dev the key is joined
        // onto the app root, and `C:/…` would replace it.
        assert_eq!(
            content_key_checked("C%3A/Windows/win.ini"),
            Err(KeyError::Invalid)
        );
        assert_eq!(content_key_checked("a.txt:stream"), Err(KeyError::Invalid));
        assert_eq!(content_key_checked("a%00b"), Err(KeyError::Invalid));
        assert_eq!(content_key_checked("%ff.txt"), Err(KeyError::Invalid));
    }

    #[test]
    fn media_types_follow_the_extension() {
        assert_eq!(media_type_for("assets/clip-1080p.mp4"), "video/mp4");
        assert_eq!(media_type_for("assets/A.MKV"), "video/x-matroska");
        assert_eq!(media_type_for("assets/chirp.wav"), "audio/wav");
        assert_eq!(media_type_for("assets/noext"), "");
    }
}
