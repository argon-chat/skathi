//! [`Payload`] reads the `.skpk` shipped with a Skathi app. It borrows the script,
//! configuration and assets from the input, which can be a mapped file or executable resource.
//!
//! ```
//! use sk_payload::{kind, Entry, Payload};
//!
//! let asset = Entry { kind: kind::ASSET, name: "ui/title.txt", data: b"Notes" };
//! let bytes = Payload::write_bank(&[asset], 0)?;
//! let app = Payload::parse(&bytes)?;
//! assert_eq!(app.asset("ui/title.txt"), Some(b"Notes".as_slice()));
//! # Ok::<(), sk_payload::Error>(())
//! ```
//!
//! For a general-purpose DeHive v3 data bank, use [`SkBank`]. Its entries support
//! bounded decompression, range reads and explicit CRC64 verification.

mod app_bank;
pub mod bank;
mod legacy;
pub use bank::{BankCodec, BankEntry, BankError, BankFile, SkBank};
pub use legacy::{write_unaligned, FORMAT, HEADER_SIZE, MAGIC, TOC_ENTRY_SIZE};

use std::fmt;
use std::ops::Range;

pub const TRAILER_MAGIC: &[u8; 8] = b"SKPKTAIL";
pub const TRAILER_SIZE: usize = 24;

/// Locates the embedded payload from the offset and length in a Linux executable's `SKPKTAIL` trailer.
pub fn appended(file: &[u8]) -> Option<Range<usize>> {
    let tail = file.len().checked_sub(TRAILER_SIZE)?;
    if &file[tail + 16..] != TRAILER_MAGIC {
        return None;
    }
    let offset = usize::try_from(u64::from_le_bytes(file[tail..tail + 8].try_into().ok()?)).ok()?;
    let len = usize::try_from(u64::from_le_bytes(
        file[tail + 8..tail + 16].try_into().ok()?,
    ))
    .ok()?;
    (offset.checked_add(len)? == tail).then_some(offset..tail)
}

/// Only entries of kind [`kind::ASSET`] are served by the app's asset resolver.
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
    Bank(BankError),
    InvalidBankEntry(String),
    RequiresStore(String),
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
            Error::Bank(error) => error.fmt(f),
            Error::InvalidBankEntry(path) => write!(f, "invalid Skathi payload entry {path:?}"),
            Error::RequiresStore(path) => write!(f, "application payload entry {path:?} requires unscrambled Store data"),
            Error::TooShort(n) => write!(f, "the payload is {n} bytes, shorter than its 64-byte header"),
            Error::BadMagic => write!(f, "unrecognized payload header (expected HIVE.BANK or SKPK)"),
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

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Bank(error) => Some(error),
            _ => None,
        }
    }
}

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
    layout: PayloadLayout<'a>,
}

#[derive(Debug)]
struct PayloadLayout<'a> {
    index: &'a [u8],
    flags: u16,
    skew: u32,
    data_offset: u64,
}

impl<'a> Payload<'a> {
    /// Opens a Skathi application payload, using its magic bytes to select the reader.
    pub fn parse(bytes: &'a [u8]) -> Result<Self, Error> {
        if bytes.starts_with(bank::MAGIC) {
            Self::from_bank(bytes)
        } else {
            legacy::parse(bytes)
        }
    }

    fn from_entries(
        bytes: &'a [u8],
        entries: Vec<Entry<'a>>,
        layout: PayloadLayout<'a>,
    ) -> Result<Self, Error> {
        for entry in &entries {
            entry.validate_text()?;
        }
        for (i, pair) in entries.windows(2).enumerate() {
            if pair[0].key() >= pair[1].key() {
                return Err(Error::Unsorted {
                    index: i + 1,
                    name: pair[1].name.to_string(),
                });
            }
        }
        let first = entries.partition_point(|e| e.kind < kind::ASSET);
        let last = entries.partition_point(|e| e.kind <= kind::ASSET);
        Ok(Payload {
            bytes,
            entries,
            assets: first..last,
            layout,
        })
    }

    fn data(&self, kind: u16, name: &str) -> Option<&'a [u8]> {
        let index = self
            .entries
            .binary_search_by(|e| e.key().cmp(&(kind, name)))
            .ok()?;
        Some(self.entries[index].data)
    }

    fn text(&self, kind: u16, name: &str) -> Option<&'a str> {
        std::str::from_utf8(self.data(kind, name)?).ok()
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
        self.data(kind::NATIVE_IMAGE, name::NATIVE_IMAGE)
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
        self.data(kind::ASSET, key)
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
        self.layout.flags
    }

    pub fn skew(&self) -> u32 {
        self.layout.skew
    }

    pub fn data_offset(&self) -> u64 {
        self.layout.data_offset
    }

    /// The bytes hashed for the pack ID. SkBank's index includes each entry's content CRC.
    pub fn index(&self) -> &'a [u8] {
        self.layout.index
    }

    pub fn as_ptr(&self) -> *const u8 {
        self.bytes.as_ptr()
    }
}

impl<'a> Entry<'a> {
    fn key(&self) -> (u16, &'a str) {
        (self.kind, self.name)
    }

    fn validate_text(&self) -> Result<(), Error> {
        if matches!(
            self.kind,
            kind::META | kind::WINDOW | kind::SCRIPT | kind::NATIVE_MANIFEST
        ) && std::str::from_utf8(self.data).is_err()
        {
            return Err(Error::BadUtf8(format!("{:?}", self.name)));
        }
        Ok(())
    }
}

fn valid_bank_name(name: &str) -> bool {
    !name.contains(['\\', '\0'])
        && !matches!(name.as_bytes(), [drive, b':', ..] if drive.is_ascii_alphabetic())
        && name.split('/').all(|part| !matches!(part, "" | "." | ".."))
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
mod tests;
