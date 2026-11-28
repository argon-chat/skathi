//! Indexed DeHive v3 data-bank layout. All file offsets are relative to the start of the header.

use super::{BankCodec, BankEntry, BankError, Encoded};
use std::ops::Range;

pub const MAGIC: &[u8; 9] = b"HIVE.BANK";
pub const FORMAT: u16 = 3;
pub const TOC_MAGIC: u64 = 0x4849_5645_544f_4333;
pub const END_MAGIC: u64 = 0x544f_435f_454e_4421;
pub(super) const MAX_STRING: usize = 1_048_576;
pub(super) const MAX_ENTRIES: usize = 10_000_000;
pub(super) const SIGNATURE_LEN: usize = 256;
pub(super) const SIGNATURE_OFFSET: usize = MAGIC.len() + 2 + 1 + 8 + 2 * 16 + 3 * 8;
const SIGNED: u64 = 1 << 2;
const SCRAMBLED: u64 = 1 << 4;
const MASTER: u64 = 1 << 5;
const DATA_FLAGS: u64 = (1 << 1) | SIGNED | (1 << 3) | SCRAMBLED; // compression, signature, root world, scramble
const HEADER_START: u8 = 0xaa;
const HEADER_END: u8 = 0xdd;
const FOOTER_LEN: usize = 16;

pub(super) struct Header<'a> {
    pub entry_count: usize,
    pub signature: Option<&'a [u8; SIGNATURE_LEN]>,
    length: usize,
    alignment: u64,
    scrambled: bool,
}

impl<'a> Header<'a> {
    pub fn read(bytes: &'a [u8]) -> Result<Self, BankError> {
        let mut input = Cursor(bytes);
        if input.take(MAGIC.len())? != MAGIC {
            return Err(BankError::invalid("missing HIVE.BANK header"));
        }
        if u16::from_le_bytes(input.array()?) != FORMAT {
            return Err(BankError::invalid("unsupported bank version (expected 3)"));
        }
        input.marker(HEADER_START)?;
        let flags = input.u64()?;
        if flags & MASTER != 0 {
            return Err(BankError::invalid("a master bank is not a data bank"));
        }
        if flags & !DATA_FLAGS != 0 {
            return Err(BankError::invalid("unsupported data bank flags"));
        }
        input.take(32)?; // shard and set IDs
        let entry_count = input.size()?;
        if input.u64()? != 0 || input.u64()? != 0 {
            return Err(BankError::invalid(
                "component and archetype banks are unsupported",
            ));
        }
        let signature = if flags & SIGNED != 0 {
            Some(input.take(SIGNATURE_LEN)?.try_into().unwrap())
        } else {
            None
        };
        let alignment = input.u64()?;
        input.marker(HEADER_END)?;
        Ok(Self {
            entry_count,
            signature,
            alignment,
            length: bytes.len() - input.0.len(),
            scrambled: flags & SCRAMBLED != 0,
        })
    }

    /// The writer fills the returned shard ID slot after hashing the ordered paths.
    pub fn write_store(out: &mut Vec<u8>, entry_count: usize, set_id: &[u8; 16]) -> Range<usize> {
        out.extend_from_slice(MAGIC);
        out.extend(FORMAT.to_le_bytes());
        out.push(HEADER_START);
        out.extend(0u64.to_le_bytes()); // flags
        let shard_id = out.len()..out.len() + 16;
        out.extend([0; 16]);
        out.extend_from_slice(set_id);
        out.extend((entry_count as u64).to_le_bytes());
        out.extend([0; 24]); // component/archetype counts and bank-wide alignment
        out.push(HEADER_END);
        shard_id
    }
}

pub(super) struct TableOfContents<'a> {
    pub serialized: &'a [u8],
    records: Cursor<'a>,
    // Offsets in TOC rows use this slice's origin and stop before the TOC.
    before_index: &'a [u8],
}

impl<'a> TableOfContents<'a> {
    pub fn read(bytes: &'a [u8], header: &Header<'_>) -> Result<Self, BankError> {
        let footer_start = bytes.len().checked_sub(FOOTER_LEN).ok_or_else(truncated)?;
        let mut footer = Cursor(&bytes[footer_start..]);
        let start = footer.size()?;
        if footer.u64()? != END_MAGIC || !(header.length..=footer_start).contains(&start) {
            return Err(BankError::invalid("invalid TOC footer"));
        }
        let mut records = Cursor(&bytes[start..footer_start]);
        if records.u64()? != TOC_MAGIC {
            return Err(BankError::invalid("expected v3 TOC"));
        }
        let count = records.size()?;
        if count != header.entry_count
            || count > MAX_ENTRIES
            || count > records.0.len() / TocRecord::MIN_LEN
        {
            return Err(BankError::invalid("invalid TOC entry count"));
        }
        let (before_index, serialized) = bytes.split_at(start);
        Ok(Self {
            serialized,
            records,
            before_index,
        })
    }

    pub fn read_entry(&mut self, header: &Header<'a>) -> Result<BankEntry<'a>, BankError> {
        TocRecord::read(&mut self.records)?.into_entry(self.before_index, header)
    }

    pub fn finish(&self) -> Result<(), BankError> {
        if !self.records.0.is_empty() {
            return Err(BankError::invalid("trailing TOC bytes"));
        }
        Ok(())
    }

    pub fn write_header(out: &mut Vec<u8>, entry_count: usize) {
        out.extend(TOC_MAGIC.to_le_bytes());
        out.extend((entry_count as u64).to_le_bytes());
    }

    pub fn write_footer(out: &mut Vec<u8>, toc_start: usize) {
        out.extend((toc_start as u64).to_le_bytes());
        out.extend(END_MAGIC.to_le_bytes());
    }
}

const COMPRESSED: u8 = 1;
const DEDUP: u8 = 1 << 1;
const CHUNKED: u8 = 1 << 7;

pub(super) struct TocRecord<'a> {
    path: &'a str,
    offset: usize,
    length: usize,
    crc64: u64,
    id: [u8; 16],
    flags: u8,
    codec: u8,
    canonical_path: &'a str,
}

impl<'a> TocRecord<'a> {
    const MIN_LEN: usize = 4 + 8 + 8 + 8 + 16 + 1 + 1 + 4;

    fn read(input: &mut Cursor<'a>) -> Result<Self, BankError> {
        Ok(Self {
            path: input.string()?,
            offset: input.size()?,
            length: input.size()?,
            crc64: input.u64()?,
            id: input.array()?,
            flags: input.byte()?,
            codec: input.byte()?,
            canonical_path: input.string()?,
        })
    }

    pub fn stored(path: &'a str, offset: usize, length: usize, crc64: u64, id: [u8; 16]) -> Self {
        Self {
            path,
            offset,
            length,
            crc64,
            id,
            flags: 0,
            codec: 0,
            canonical_path: "",
        }
    }

    pub fn write(&self, out: &mut Vec<u8>) {
        write_string(out, self.path);
        out.extend((self.offset as u64).to_le_bytes());
        out.extend((self.length as u64).to_le_bytes());
        out.extend(self.crc64.to_le_bytes());
        out.extend(self.id);
        out.extend([self.flags, self.codec]);
        write_string(out, self.canonical_path);
    }

    fn into_entry(
        self,
        before_index: &'a [u8],
        header: &Header<'_>,
    ) -> Result<BankEntry<'a>, BankError> {
        let codec = match self.codec & !CHUNKED {
            0 => BankCodec::Store,
            1 => BankCodec::Brotli,
            2 => BankCodec::Zstd,
            _ => return Err(BankError::invalid("unsupported codec")),
        };
        let chunked = self.codec & CHUNKED != 0;
        let canonical_path = match (self.flags, codec, chunked, self.canonical_path) {
            (0, BankCodec::Store, false, "") => None,
            (COMPRESSED, BankCodec::Brotli | BankCodec::Zstd, _, "") => None,
            (DEDUP, BankCodec::Store, false, target) if self.length == 0 && !target.is_empty() => {
                Some(target)
            }
            _ => return Err(BankError::invalid("invalid entry flags")),
        };
        let bytes = self
            .offset
            .checked_add(self.length)
            .filter(|_| self.offset >= header.length)
            .and_then(|end| before_index.get(self.offset..end))
            .ok_or_else(|| BankError::invalid("entry range outside bank data"))?;
        if header.alignment > 1 && !(self.offset as u64).is_multiple_of(header.alignment) {
            return Err(BankError::invalid("entry does not meet block alignment"));
        }
        Ok(BankEntry {
            path: self.path,
            id: self.id,
            crc64: self.crc64,
            canonical_path,
            encoded: Encoded {
                bytes,
                codec,
                chunked,
                scramble: header.scrambled.then_some((self.id, self.crc64)),
            },
        })
    }
}

pub(super) fn write_string(out: &mut Vec<u8>, value: &str) {
    out.extend((value.len() as u32).to_le_bytes());
    out.extend_from_slice(value.as_bytes());
}

pub(super) fn truncated() -> BankError {
    BankError::invalid("truncated bank")
}

pub(super) struct Cursor<'a>(pub &'a [u8]);
impl<'a> Cursor<'a> {
    fn take(&mut self, n: usize) -> Result<&'a [u8], BankError> {
        let (value, rest) = self.0.split_at_checked(n).ok_or_else(truncated)?;
        self.0 = rest;
        Ok(value)
    }
    fn array<const N: usize>(&mut self) -> Result<[u8; N], BankError> {
        Ok(self.take(N)?.try_into().unwrap())
    }
    fn byte(&mut self) -> Result<u8, BankError> {
        Ok(self.take(1)?[0])
    }
    fn marker(&mut self, byte: u8) -> Result<(), BankError> {
        if self.byte()? != byte {
            return Err(BankError::invalid("invalid header marker"));
        }
        Ok(())
    }
    fn u64(&mut self) -> Result<u64, BankError> {
        Ok(u64::from_le_bytes(self.array()?))
    }
    fn size(&mut self) -> Result<usize, BankError> {
        let value = i64::from_le_bytes(self.array()?);
        usize::try_from(value).map_err(|_| BankError::invalid("invalid offset, length or count"))
    }
    pub fn string(&mut self) -> Result<&'a str, BankError> {
        let len = u32::from_le_bytes(self.array()?);
        if len as usize > MAX_STRING {
            return Err(BankError::invalid("string exceeds the bank format limit"));
        }
        std::str::from_utf8(self.take(len as usize)?)
            .map_err(|_| BankError::invalid("invalid UTF-8 path"))
    }
}
