//! A mapped DeHive v3 data bank. Opening a bank reads its index; entry data is borrowed or decoded on demand.

use std::borrow::Cow;
use std::collections::HashMap;
use std::fmt;

mod crc;
mod decode;
mod format;
mod write;

pub use crc::crc64;
use decode::Encoded;
use format::{Header, TableOfContents, SIGNATURE_LEN, SIGNATURE_OFFSET};
pub use format::{END_MAGIC, FORMAT, MAGIC, TOC_MAGIC};
pub use write::BankFile;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BankError(String);

impl BankError {
    fn invalid(reason: &str) -> Self {
        Self(format!("SkBank: {reason}"))
    }
}

impl fmt::Display for BankError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for BankError {}

impl From<std::io::Error> for BankError {
    fn from(error: std::io::Error) -> Self {
        Self(format!("SkBank decode: {error}"))
    }
}

#[derive(Debug, Copy, Clone, Eq, PartialEq)]
pub enum BankCodec {
    Store,
    Brotli,
    Zstd,
}

#[derive(Debug)]
pub struct BankEntry<'a> {
    pub path: &'a str,
    pub id: [u8; 16],
    pub crc64: u64,
    pub canonical_path: Option<&'a str>,
    encoded: Encoded<'a>,
}

impl<'a> BankEntry<'a> {
    pub fn codec(&self) -> BankCodec {
        self.encoded.codec
    }

    pub fn is_chunked(&self) -> bool {
        self.encoded.chunked
    }

    /// Borrows the canonical file bytes when no decoding is needed.
    pub fn stored_bytes(&self) -> Option<&'a [u8]> {
        (self.encoded.codec == BankCodec::Store && self.encoded.scramble.is_none())
            .then_some(self.encoded.bytes)
    }

    /// Returns at most `max_bytes` of uncompressed data. Codecs allocate their working memory separately.
    pub fn read(&self, max_bytes: usize) -> Result<Cow<'a, [u8]>, BankError> {
        self.encoded.read(max_bytes)
    }

    /// Reads only the covered chunks of a chunked file. Whole-stream codecs must decode from the beginning.
    pub fn read_range(&self, range: std::ops::Range<usize>) -> Result<Cow<'a, [u8]>, BankError> {
        self.encoded.read_range(range)
    }

    /// Checks CRC64 over the full entry using the uncompressed output limit from [`Self::read`].
    pub fn verify(&self, max_bytes: usize) -> Result<(), BankError> {
        if crc64(&self.read(max_bytes)?) != self.crc64 {
            return Err(BankError::invalid("CRC64 mismatch"));
        }
        Ok(())
    }
}

/// A borrowed, indexed v3 data bank. Opening it reads the index and leaves asset pages untouched.
///
/// ```
/// use sk_payload::{BankFile, SkBank};
///
/// let file = BankFile { path: "ui/title.txt", bytes: b"Notes", alignment: 1 };
/// let bytes = SkBank::write(&[file], 0)?;
/// let bank = SkBank::parse(&bytes)?;
/// let title = bank.entry("ui/title.txt").unwrap();
/// title.verify(64)?;
/// assert_eq!(title.stored_bytes(), Some(b"Notes".as_slice()));
/// # Ok::<(), sk_payload::BankError>(())
/// ```
#[derive(Debug)]
pub struct SkBank<'a> {
    entries: Vec<BankEntry<'a>>,
    paths: HashMap<String, usize>,
    ids: HashMap<[u8; 16], usize>,
    index: &'a [u8],
    bytes: &'a [u8],
    signature: Option<&'a [u8; 256]>,
}

impl<'a> SkBank<'a> {
    /// Opens the exact bank slice, including its footer. Use a trusted key to authenticate a signed bank.
    pub fn parse(bytes: &'a [u8]) -> Result<Self, BankError> {
        let header = Header::read(bytes)?;
        let mut toc = TableOfContents::read(bytes, &header)?;
        let mut bank = Self {
            entries: Vec::new(),
            paths: HashMap::new(),
            ids: HashMap::new(),
            index: toc.serialized,
            bytes,
            signature: header.signature,
        };
        for row in 0..header.entry_count {
            let entry = toc.read_entry(&header)?;
            let path = path_key(entry.path);
            if path.is_empty() || bank.paths.insert(path, row).is_some() {
                return Err(BankError::invalid("empty or duplicate bank path"));
            }
            bank.ids.entry(entry.id).or_insert(row);
            bank.entries.push(entry);
        }
        toc.finish()?;
        bank.resolve_aliases()?;
        Ok(bank)
    }

    fn resolve_aliases(&mut self) -> Result<(), BankError> {
        const MAX_ALIAS_DEPTH: usize = 16;
        for row in 0..self.entries.len() {
            if self.entries[row].canonical_path.is_none() {
                continue;
            }
            let mut origin = row;
            for _ in 0..MAX_ALIAS_DEPTH {
                let Some(target) = self.entries[origin].canonical_path else {
                    break;
                };
                origin = *self
                    .paths
                    .get(&path_key(target))
                    .ok_or_else(|| BankError::invalid("missing deduplication target"))?;
            }
            let canonical = &self.entries[origin];
            if canonical.canonical_path.is_some() {
                return Err(BankError::invalid(
                    "deduplication chain is cyclic or too long",
                ));
            }
            if canonical.crc64 != self.entries[row].crc64 {
                return Err(BankError::invalid("deduplication target CRC64 differs"));
            }
            self.entries[row].encoded = canonical.encoded;
        }
        Ok(())
    }

    pub fn entries(&self) -> &[BankEntry<'a>] {
        &self.entries
    }
    pub fn entry(&self, path: &str) -> Option<&BankEntry<'a>> {
        self.paths
            .get(&path_key(path))
            .map(|&row| &self.entries[row])
    }
    pub fn entry_by_id(&self, id: &[u8; 16]) -> Option<&BankEntry<'a>> {
        self.ids.get(id).map(|&row| &self.entries[row])
    }
    /// The serialized TOC and its trailing locator, suitable for identifying an external pack.
    pub fn index(&self) -> &'a [u8] {
        self.index
    }
    /// The raw RSA-2048 signature slot, when present.
    pub fn signature(&self) -> Option<&'a [u8; 256]> {
        self.signature
    }

    /// The SHA-256 prehash for RSA PKCS#1 v1.5 verification with the caller's trusted public key.
    pub fn signature_digest(&self) -> Option<[u8; 32]> {
        use sha2::{Digest, Sha256};
        self.signature?;
        Some(
            Sha256::new()
                .chain_update(&self.bytes[..SIGNATURE_OFFSET])
                .chain_update([0; SIGNATURE_LEN])
                .chain_update(&self.bytes[SIGNATURE_OFFSET + SIGNATURE_LEN..])
                .finalize()
                .into(),
        )
    }
}

fn path_key(path: &str) -> String {
    let path = path.trim_start_matches(['/', '\\']);
    if path.is_ascii() && !path.contains('\\') {
        return path.to_ascii_uppercase();
    }
    path.chars()
        .map(|c| match c {
            '\\' => '/',
            c if c.is_ascii() => c.to_ascii_uppercase(),
            // OrdinalIgnoreCase uses the single-codepoint invariant uppercase mapping.
            // Non-ASCII characters cannot alias ASCII; full Unicode case expansions do not apply.
            '\u{1f80}'..='\u{1f87}' | '\u{1f90}'..='\u{1f97}' | '\u{1fa0}'..='\u{1fa7}' => {
                char::from_u32(c as u32 + 8).unwrap()
            }
            '\u{1fb3}' | '\u{1fc3}' | '\u{1ff3}' => char::from_u32(c as u32 + 9).unwrap(),
            _ => {
                let mut upper = c.to_uppercase();
                let first = upper.next().unwrap();
                if upper.next().is_none() && !first.is_ascii() {
                    first
                } else {
                    c
                }
            }
        })
        .collect()
}

#[cfg(test)]
mod tests;
