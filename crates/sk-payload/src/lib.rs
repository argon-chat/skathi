//! SKPK v1, the app payload a packaged exe carries, and the one function that turns a URL into an asset key.
//!
//! `sk build` writes one blob, stamps it into the bare host as the `RT_RCDATA` resource `SKPAYLOAD`, and the
//! host maps it zero-copy at boot. This crate is the reader; a golden fixture
//! (`packages/cli/test/fixtures/golden.skpk`) ties it to the writer (`packages/cli/cli/skpk.ts`).
//!
//! ```text
//! header 64 B | toc (32 B × n, sorted by (kind, name bytes)) | names | skew | data
//! ```
//!
//! Every field is read byte-wise: the blob is only 4-byte aligned in the image. [`Payload::parse`] validates
//! once, in O(entries); asset bytes are never read at boot, since that would fault in every page.
//! [`content_key`] is the normaliser both asset paths use, so they agree on what `assets/a.png?v=2` names.

use std::fmt;
use std::ops::Range;

pub const MAGIC: &[u8; 4] = b"SKPK";
pub const FORMAT: u16 = 1;
pub const HEADER_SIZE: usize = 64;
pub const TOC_ENTRY_SIZE: usize = 32;

pub const TRAILER_MAGIC: &[u8; 8] = b"SKPKTAIL";
pub const TRAILER_SIZE: usize = 24;

/// The payload `sk build` appended to a Linux executable: `exe | zero pad | SKPK | u64 offset | u64 len | "SKPKTAIL"`,
/// with the offset page-aligned so the data region lands on a page boundary in a mapping of the whole file.
pub fn appended(file: &[u8]) -> Option<Range<usize>> {
    let tail = file.len().checked_sub(TRAILER_SIZE)?;
    if &file[tail + 16..] != TRAILER_MAGIC {
        return None;
    }
    let offset = usize::try_from(u64_at(file, tail)).ok()?;
    let len = usize::try_from(u64_at(file, tail + 8)).ok()?;
    (offset.checked_add(len)? == tail).then_some(offset..tail)
}

/// Entry kinds. Only [`kind::ASSET`] is reachable from app code, so `fetch("sk/app.js")` is a miss.
pub mod kind {
    pub const META: u16 = 1;
    pub const WINDOW: u16 = 2;
    pub const SCRIPT: u16 = 3;
    pub const BYTECODE: u16 = 4;
    /// The manifest of the app's native AOT image (`sk build --aot`).
    pub const NATIVE_MANIFEST: u16 = 5;
    /// That image, raw-DEFLATE compressed.
    pub const NATIVE_IMAGE: u16 = 6;
    /// The app's own `--shading` effects: `sk/effects/effects.json` and the shaders it names.
    pub const EFFECT: u16 = 7;
    /// ICU common data (`sk/icu/<package>.dat`), alone in the `icu.skpk` pack.
    pub const ICU_DATA: u16 = 8;
    pub const ASSET: u16 = 16;
}

pub mod name {
    pub const META: &str = "sk/app.json";
    pub const WINDOW: &str = "sk/window.json";
    pub const SCRIPT: &str = "sk/app.js";
    pub const BYTECODE: &str = "sk/app.bc";
    pub const NATIVE_MANIFEST: &str = "sk/app.native.json";
    pub const NATIVE_IMAGE: &str = "sk/app.image";
    /// The prefix of every [`super::kind::EFFECT`] entry.
    pub const EFFECTS_DIR: &str = "sk/effects/";
    /// The prefix of the [`super::kind::ICU_DATA`] entry.
    pub const ICU_DIR: &str = "sk/icu/";
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    TooShort(usize),
    BadMagic,
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
            Error::BadMagic => write!(f, "the payload does not start with \"SKPK\""),
            Error::NewerFormat(v) => write!(
                f,
                "the payload is format {v}, and this host reads format {FORMAT} — it was built by a newer sk; \
                 rebuild the app with the sk that matches this host"
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

#[derive(Debug, Clone, Copy)]
pub struct Entry<'a> {
    pub name: &'a str,
    pub kind: u16,
    pub data: &'a [u8],
}

#[derive(Debug)]
pub struct Payload<'a> {
    bytes: &'a [u8],
    entries: Vec<Entry<'a>>,
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
            if matches!(
                kind,
                kind::META | kind::WINDOW | kind::SCRIPT | kind::NATIVE_MANIFEST
            ) && std::str::from_utf8(data).is_err()
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
        std::str::from_utf8(e.data).ok()
    }

    pub fn meta(&self) -> Option<&'a str> {
        self.text(kind::META, name::META)
    }

    pub fn window_json(&self) -> Option<&'a str> {
        self.text(kind::WINDOW, name::WINDOW)
    }

    pub fn script(&self) -> Option<&'a str> {
        self.text(kind::SCRIPT, name::SCRIPT)
    }

    pub fn native_manifest(&self) -> Option<&'a str> {
        self.text(kind::NATIVE_MANIFEST, name::NATIVE_MANIFEST)
    }

    /// The compressed native image; only meaningful beside [`Self::native_manifest`].
    pub fn native_image(&self) -> Option<&'a [u8]> {
        self.entries
            .iter()
            .find(|e| e.kind == kind::NATIVE_IMAGE && e.name == name::NATIVE_IMAGE)
            .map(|e| e.data)
    }

    /// The ICU common-data package of an `icu.skpk` pack.
    pub fn icu_data(&self) -> Option<&'a [u8]> {
        self.entries
            .iter()
            .find(|e| e.kind == kind::ICU_DATA)
            .map(|e| e.data)
    }

    /// The app's effect files, named relative to [`name::EFFECTS_DIR`]; empty for an app without any.
    pub fn effects(&self) -> impl Iterator<Item = (&'a str, &'a [u8])> + '_ {
        self.entries.iter().filter_map(|e| {
            (e.kind == kind::EFFECT)
                .then(|| e.name.strip_prefix(name::EFFECTS_DIR))
                .flatten()
                .map(|n| (n, e.data))
        })
    }

    pub fn assets(&self) -> &[Entry<'a>] {
        &self.entries[self.assets.clone()]
    }

    pub fn asset_count(&self) -> usize {
        self.assets.len()
    }

    pub fn asset(&self, key: &str) -> Option<&'a [u8]> {
        let run = self.assets();
        run.binary_search_by(|e| e.name.as_bytes().cmp(key.as_bytes()))
            .ok()
            .map(|i| run[i].data)
    }

    /// The asset whose name equals `key` ignoring ASCII case: NTFS is case-insensitive in dev, so a packaged exe must not be stricter.
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

    pub fn data_offset(&self) -> u64 {
        self.data_off
    }

    /// The header, TOC and names: an asset pack's identity is a hash of this.
    pub fn index(&self) -> &'a [u8] {
        let end = self
            .data_off
            .saturating_sub(u64::from(self.skew))
            .min(self.bytes.len() as u64);
        &self.bytes[..(end as usize).max(HEADER_SIZE.min(self.bytes.len()))]
    }

    pub fn as_ptr(&self) -> *const u8 {
        self.bytes.as_ptr()
    }
}

/// A payload with no skew and no data alignment, for tests: `sk build` writes the real ones.
/// `entries` must already be sorted by (kind, name bytes).
pub fn write_unaligned(entries: &[(u16, &str, &[u8])]) -> Vec<u8> {
    let names: Vec<u8> = entries.iter().flat_map(|e| e.1.bytes()).collect();
    let names_off = HEADER_SIZE + entries.len() * TOC_ENTRY_SIZE;
    let data_off = names_off + names.len();
    let mut toc = Vec::new();
    let (mut name_at, mut data) = (0usize, Vec::new());
    for (kind, name, bytes) in entries {
        toc.extend_from_slice(&(name_at as u32).to_le_bytes());
        toc.extend_from_slice(&(name.len() as u32).to_le_bytes());
        toc.extend_from_slice(&(data.len() as u64).to_le_bytes());
        toc.extend_from_slice(&(bytes.len() as u64).to_le_bytes());
        toc.extend_from_slice(&kind.to_le_bytes());
        toc.extend_from_slice(&[0; 6]);
        name_at += name.len();
        data.extend_from_slice(bytes);
    }
    let mut blob = vec![0u8; HEADER_SIZE];
    blob[..4].copy_from_slice(MAGIC);
    blob[4..6].copy_from_slice(&FORMAT.to_le_bytes());
    blob[8..12].copy_from_slice(&(entries.len() as u32).to_le_bytes());
    blob[12..16].copy_from_slice(&(HEADER_SIZE as u32).to_le_bytes());
    blob[16..20].copy_from_slice(&(names_off as u32).to_le_bytes());
    blob[20..24].copy_from_slice(&(names.len() as u32).to_le_bytes());
    blob[32..40].copy_from_slice(&(data_off as u64).to_le_bytes());
    blob[40..48].copy_from_slice(&((data_off + data.len()) as u64).to_le_bytes());
    blob.extend(toc);
    blob.extend(names);
    blob.extend(data);
    blob
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyError {
    Empty,
    EscapesRoot,
    Invalid,
}

/// The app-root-relative key a relative or root-relative URL names, applied textually: `?query` and
/// `#hash` are cut, `%XX` decoded, `\` is `/`, empty and `.` segments vanish, `..` folds lexically (one
/// climbing above the root is [`KeyError::EscapesRoot`]). The result is `/`-separated and case-preserved.
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

pub fn content_key(url: &str) -> Option<String> {
    content_key_checked(url).ok()
}

/// `%XX` → byte; a malformed escape is kept literally, as a browser does. `None` when the bytes are not UTF-8.
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

/// The media type a packaged `<video>`/`<audio>` source is handed to the platform with; `""` = unknown.
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

    const GOLDEN: &[u8] = include_bytes!("../../../packages/cli/test/fixtures/golden.skpk");

    #[test]
    fn the_golden_fixture_parses_to_what_the_writer_put_in() {
        let p = Payload::parse(GOLDEN).expect("golden.skpk parses");
        assert_eq!(p.skew(), 100);
        assert_eq!(p.entries().len(), 7);
        assert_eq!(p.asset_count(), 4);
        assert_eq!(p.script(), Some("globalThis.__golden = 42;\n"));
        assert_eq!(p.window_json(), Some(r#"{"title":"Golden","width":640}"#));
        let meta = p.meta().unwrap();
        assert!(meta.contains(r#""name":"golden""#), "{meta}");
        assert!(meta.contains(r#""variant":"standard""#), "{meta}");
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
        let names_end = u32_at(GOLDEN, 16) as u64 + u32_at(GOLDEN, 20) as u64;
        assert_eq!(p.data_offset(), names_end + 100);
        assert_eq!(p.index(), &GOLDEN[..names_end as usize]);
    }

    #[test]
    fn internal_entries_are_not_assets() {
        let p = Payload::parse(GOLDEN).unwrap();
        // `fetch("sk/app.js")` must be a 404, not the bundle's source.
        assert_eq!(p.asset(name::SCRIPT), None);
        assert_eq!(p.asset(name::META), None);
        assert!(p.asset_ignoring_case("SK/APP.JS").is_none());
    }

    #[test]
    fn a_native_image_rides_beside_the_script_and_is_not_an_asset() {
        let blob = write_unaligned(&[
            (kind::SCRIPT, name::SCRIPT, b"app()"),
            (kind::NATIVE_MANIFEST, name::NATIVE_MANIFEST, b"{}"),
            (kind::NATIVE_IMAGE, name::NATIVE_IMAGE, &[0xff, 0, 1]),
            (kind::ASSET, "assets/a.txt", b"a"),
        ]);
        let p = Payload::parse(&blob).unwrap();
        assert_eq!(p.script(), Some("app()"));
        assert_eq!(p.native_manifest(), Some("{}"));
        assert_eq!(p.native_image(), Some(&[0xff, 0, 1][..]));
        assert_eq!(p.asset_count(), 1);
        assert_eq!(p.asset(name::NATIVE_IMAGE), None);
        assert_eq!(Payload::parse(GOLDEN).unwrap().native_manifest(), None);
        let bad = write_unaligned(&[(kind::NATIVE_MANIFEST, name::NATIVE_MANIFEST, &[0xff])]);
        assert_eq!(
            Payload::parse(&bad).unwrap_err(),
            Error::BadUtf8("\"sk/app.native.json\"".into())
        );
    }

    #[test]
    fn effects_are_their_own_kind_and_not_assets() {
        let blob = write_unaligned(&[
            (kind::SCRIPT, name::SCRIPT, b"app()"),
            (kind::EFFECT, "sk/effects/effects.json", b"{}"),
            (kind::EFFECT, "sk/effects/ripple.wgsl", b"fn"),
            (kind::ASSET, "assets/a.txt", b"a"),
        ]);
        let p = Payload::parse(&blob).unwrap();
        let effects: Vec<_> = p.effects().collect();
        assert_eq!(
            effects,
            [("effects.json", &b"{}"[..]), ("ripple.wgsl", &b"fn"[..])]
        );
        assert_eq!(p.asset("sk/effects/ripple.wgsl"), None);
        assert_eq!(p.asset_count(), 1);
        assert_eq!(Payload::parse(GOLDEN).unwrap().effects().count(), 0);
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
        let oob = mutated(|v| {
            v[HEADER_SIZE + 16..HEADER_SIZE + 24].copy_from_slice(&(1u64 << 40).to_le_bytes())
        });
        assert!(matches!(
            Payload::parse(&oob).unwrap_err(),
            Error::OutOfBounds { .. }
        ));
        let many = mutated(|v| v[8..12].copy_from_slice(&1000u32.to_le_bytes()));
        assert!(matches!(
            Payload::parse(&many).unwrap_err(),
            Error::OutOfBounds { .. }
        ));
        assert!(matches!(
            Payload::parse(&GOLDEN[..GOLDEN.len() - 1]).unwrap_err(),
            Error::OutOfBounds { .. }
        ));
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
            Error::BadUtf8("\"sk/app.js\"".into())
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
        assert_eq!(k("assets/a.png?v=2").as_deref(), Some("assets/a.png"));
        assert_eq!(k("assets/a.png#frag").as_deref(), Some("assets/a.png"));
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
        assert_eq!(k("Assets/A.PNG").as_deref(), Some("Assets/A.PNG"));
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
    fn a_payload_appended_to_an_executable_is_found_by_its_trailer() {
        let mut file = b"\x7fELF".to_vec();
        file.resize(4096, 0);
        file.extend_from_slice(GOLDEN);
        file.extend_from_slice(&4096u64.to_le_bytes());
        file.extend_from_slice(&(GOLDEN.len() as u64).to_le_bytes());
        file.extend_from_slice(TRAILER_MAGIC);
        let range = appended(&file).expect("the trailer names the payload");
        assert_eq!(range, 4096..4096 + GOLDEN.len());
        assert_eq!(Payload::parse(&file[range]).unwrap().asset_count(), 4);
        // The same trailer the TS writer's test pins (packages/cli/test/elf.test.ts).
        assert_eq!(
            file[file.len() - TRAILER_SIZE..],
            *b"\x00\x10\x00\x00\x00\x00\x00\x00\x04\x04\x00\x00\x00\x00\x00\x00SKPKTAIL"
        );

        assert_eq!(appended(b"\x7fELF bare host"), None);
        assert_eq!(appended(&file[..file.len() - 1]), None);
        let mut short = file.clone();
        short.insert(0, 0);
        assert_eq!(appended(&short), None);
        assert_eq!(appended(TRAILER_MAGIC), None);
    }
}
