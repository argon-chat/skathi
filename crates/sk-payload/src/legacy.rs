//! SKPK v1 support for workloads that predate SkBank.

use crate::{Entry, Error, Payload, PayloadLayout};

pub const MAGIC: &[u8; 4] = b"SKPK";
pub const FORMAT: u16 = 1;
pub const HEADER_SIZE: usize = 64;
pub const TOC_ENTRY_SIZE: usize = 32;

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

pub(super) fn parse(bytes: &[u8]) -> Result<Payload<'_>, Error> {
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
        entries.push(Entry { name, kind, data });
    }
    let end = data_off
        .saturating_sub(u64::from(skew))
        .max(HEADER_SIZE as u64);
    Payload::from_entries(
        bytes,
        entries,
        PayloadLayout {
            index: &bytes[..end as usize],
            flags,
            skew,
            data_offset: data_off,
        },
    )
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
