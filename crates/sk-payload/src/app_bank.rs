//! The Skathi application profile of a DeHive data bank.

use crate::{kind, valid_bank_name, BankFile, Entry, Error, Payload, PayloadLayout, SkBank};
use sha2::{Digest, Sha256};

const KEY_PREFIX: &str = "skathi/";
const PAGE_ALIGNMENT: usize = 4096;
const ENTRY_ALIGNMENT: usize = 16;
const HAS_BYTECODE: u16 = 1;

/// The digest keeps case-distinct logical names separate in case-insensitive DeHive readers.
struct PayloadKey<'a> {
    kind: u16,
    name: &'a str,
}

impl<'a> PayloadKey<'a> {
    fn new(kind: u16, name: &'a str) -> Option<Self> {
        (kind != 0 && valid_bank_name(name)).then_some(Self { kind, name })
    }

    fn from_path(path: &'a str) -> Option<Self> {
        let (kind, rest) = path.strip_prefix(KEY_PREFIX)?.split_once('/')?;
        let (digest, name) = rest.split_once('/')?;
        if kind.len() != 4
            || !kind
                .bytes()
                .all(|b| b.is_ascii_digit() || matches!(b, b'a'..=b'f'))
        {
            return None;
        }
        let key = Self::new(u16::from_str_radix(kind, 16).ok()?, name)?;
        (digest.len() == 64 && digest == key.digest()).then_some(key)
    }

    fn path(&self) -> String {
        format!(
            "{KEY_PREFIX}{:04x}/{}/{}",
            self.kind,
            self.digest(),
            self.name
        )
    }

    fn digest(&self) -> String {
        format!("{:x}", Sha256::digest(self.name.as_bytes()))
    }
}

impl<'a> Payload<'a> {
    pub(super) fn from_bank(bytes: &'a [u8]) -> Result<Self, Error> {
        let bank = SkBank::parse(bytes).map_err(Error::Bank)?;
        let mut layout = PayloadLayout {
            index: bank.index(),
            data_offset: (bytes.len() - bank.index().len()) as u64,
            skew: 0,
            flags: 0,
        };
        let mut entries = Vec::with_capacity(bank.entries().len());
        for record in bank.entries() {
            let key = PayloadKey::from_path(record.path)
                .ok_or_else(|| Error::InvalidBankEntry(record.path.into()))?;
            let data = record
                .stored_bytes()
                .ok_or_else(|| Error::RequiresStore(key.name.into()))?;
            let offset = data.as_ptr() as usize - bytes.as_ptr() as usize;
            layout.data_offset = layout.data_offset.min(offset as u64);
            if key.kind == kind::BYTECODE {
                layout.flags |= HAS_BYTECODE;
            }
            entries.push(Entry {
                kind: key.kind,
                name: key.name,
                data,
            });
        }
        entries.sort_unstable_by_key(Entry::key);
        Self::from_entries(bytes, entries, layout)
    }

    /// Writes a typed application bank. Use base offset zero for a standalone file or the
    /// payload's offset in the enclosing mapping for an embedded bank.
    pub fn write_bank(entries: &[Entry<'_>], base_offset: u64) -> Result<Vec<u8>, Error> {
        let mut named = Vec::with_capacity(entries.len());
        for entry in entries {
            let key = PayloadKey::new(entry.kind, entry.name)
                .ok_or_else(|| Error::InvalidBankEntry(entry.name.into()))?;
            entry.validate_text()?;
            named.push((entry, key.path()));
        }
        named.sort_unstable_by_key(|(entry, _)| entry.key());
        let files: Vec<_> = named
            .iter()
            .enumerate()
            .map(|(i, (entry, path))| BankFile {
                path,
                bytes: entry.data,
                alignment: if i == 0 || matches!(entry.kind, kind::ICU_DATA | kind::BYTECODE) {
                    PAGE_ALIGNMENT
                } else {
                    ENTRY_ALIGNMENT
                },
            })
            .collect();
        SkBank::write(&files, base_offset).map_err(Error::Bank)
    }
}
