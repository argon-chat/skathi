use super::format::{write_string, Header, TableOfContents, TocRecord, MAX_ENTRIES, MAX_STRING};
use super::{crc64, path_key, BankError, SkBank};
use sha2::{Digest, Sha256};
use std::collections::HashSet;

/// One ordinary Store entry. Alignment is in bytes (0 or 1 for none), relative to the enclosing mapping.
#[derive(Clone, Copy, Debug)]
pub struct BankFile<'a> {
    pub path: &'a str,
    pub bytes: &'a [u8],
    pub alignment: usize,
}

impl SkBank<'_> {
    /// Writes a deterministic v3 Store bank in input order. Use a base offset of zero for a standalone file.
    pub fn write(files: &[BankFile<'_>], base_offset: u64) -> Result<Vec<u8>, BankError> {
        if files.len() > MAX_ENTRIES {
            return Err(BankError::invalid("too many entries"));
        }
        let mut seen = HashSet::new();
        let mut bank_id = Sha256::new().chain_update(b"SkBank/v3\0");
        let mut body = Vec::new();
        let set_id = Sha256::digest(b"SkBank/v3");
        let shard_id =
            Header::write_store(&mut body, files.len(), set_id[..16].try_into().unwrap());
        let mut toc = Vec::new();
        TableOfContents::write_header(&mut toc, files.len());

        for file in files {
            let path = file.path.replace('\\', "/");
            if path.len() > MAX_STRING || !crate::valid_bank_name(&path) {
                return Err(BankError::invalid(
                    "expected a bank-relative file path up to 1 MiB",
                ));
            }
            if !seen.insert(path_key(&path)) {
                return Err(BankError::invalid("duplicate bank path"));
            }
            if file.alignment > MAX_STRING {
                return Err(BankError::invalid("entry alignment exceeds 1 MiB"));
            }

            bank_id.update(path.as_bytes());
            bank_id.update([0]);
            let record = write_entity(&mut body, &path, file, base_offset)?;
            record.write(&mut toc);
        }
        body[shard_id].copy_from_slice(&bank_id.finalize()[..16]);
        TableOfContents::write_footer(&mut toc, body.len());
        body.try_reserve(toc.len())
            .map_err(|_| BankError::invalid("bank too large"))?;
        body.extend_from_slice(&toc);
        Ok(body)
    }
}

const FILE_TYPE: &str = ".e_file";
const FILE_SENTINEL: u64 = 0x00aa_bbcc_ddee;

fn write_entity<'a>(
    out: &mut Vec<u8>,
    path: &'a str,
    file: &BankFile<'_>,
    base: u64,
) -> Result<TocRecord<'a>, BankError> {
    let crc = crc64(file.bytes);
    let digest = Sha256::new()
        .chain_update(path)
        .chain_update(crc.to_le_bytes())
        .finalize();
    let id: [u8; 16] = digest[..16].try_into().unwrap();
    let name = path.rsplit('/').next().unwrap();
    let (stem, extension) = match name.rfind('.') {
        Some(dot) if dot > 0 => name.split_at(dot),
        _ => (name, ""),
    };
    out.extend((file.bytes.len() as u64).to_le_bytes());
    out.extend(id);
    write_string(out, stem);
    write_string(out, name);
    write_string(out, path);
    write_string(out, extension);
    write_tags(out, base, file.alignment)?;
    out.extend(crc.to_le_bytes());
    out.extend([0, 0x0a]); // Store codec, end of entity metadata
    out.extend(FILE_SENTINEL.to_le_bytes());
    write_string(out, FILE_TYPE);

    let record = TocRecord::stored(path, out.len(), file.bytes.len(), crc, id);
    out.try_reserve(file.bytes.len())
        .map_err(|_| BankError::invalid("bank too large"))?;
    out.extend_from_slice(file.bytes);
    Ok(record)
}

// A normal string tag carries the padding, keeping sequential DeHive readers in sync when the bank
// starts at an arbitrary address in an executable. The header's global BlockAlign remains zero.
fn write_tags(out: &mut Vec<u8>, base: u64, alignment: usize) -> Result<(), BankError> {
    const SUFFIX: u64 = 8 + 2 + 8 + 4 + FILE_TYPE.len() as u64; // CRC, codec and marker, sentinel, ".e_file"
    let unpadded = base
        .checked_add(out.len() as u64)
        .and_then(|v| v.checked_add(4 + SUFFIX))
        .ok_or_else(|| BankError::invalid("alignment address overflow"))?;
    let alignment = alignment.max(1) as u64;
    if unpadded.is_multiple_of(alignment) {
        out.extend(0u32.to_le_bytes());
    } else {
        const PREFIX: &[u8] = b"sk.pad";
        let tag_start = unpadded
            .checked_add((4 + PREFIX.len()) as u64)
            .ok_or_else(|| BankError::invalid("alignment address overflow"))?;
        let fill = (alignment - tag_start % alignment) % alignment;
        let mut len = fill as usize + PREFIX.len();
        if len > MAX_STRING {
            len -= alignment as usize;
        }
        unpadded
            .checked_add(4 + len as u64)
            .ok_or_else(|| BankError::invalid("alignment address overflow"))?;
        out.extend(1u32.to_le_bytes());
        out.extend((len as u32).to_le_bytes());
        let prefix_len = PREFIX.len().min(len);
        out.extend_from_slice(&PREFIX[..prefix_len]);
        out.resize(out.len() + len - prefix_len, b'.');
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn aligns_generic_files_to_their_enclosing_mapping() {
        let files = [
            BankFile {
                path: "UI\\café",
                bytes: b"first",
                alignment: 63,
            },
            BankFile {
                path: "app.bin",
                bytes: b"second",
                alignment: 4096,
            },
        ];
        let bytes = SkBank::write(&files, 0x1234).unwrap();
        let bank = SkBank::parse(&bytes).unwrap();
        for file in files {
            let entry = bank.entry(file.path).unwrap();
            let data = entry.stored_bytes().unwrap();
            assert_eq!(data, file.bytes);
            let offset = data.as_ptr() as usize - bytes.as_ptr() as usize;
            assert!((offset + 0x1234).is_multiple_of(file.alignment));
            entry.verify(file.bytes.len()).unwrap();
        }
        let duplicate = BankFile {
            path: "ui/CAFÉ",
            ..files[0]
        };
        assert!(SkBank::write(&[files[0], duplicate], 0).is_err());
        assert!(SkBank::write(
            &[BankFile {
                alignment: MAX_STRING + 1,
                ..files[0]
            }],
            0
        )
        .is_err());
        assert!(SkBank::write(&files, u64::MAX).is_err());
        assert!(SkBank::parse(&SkBank::write(&[], 0).unwrap())
            .unwrap()
            .entries()
            .is_empty());
    }

    #[test]
    fn writes_only_bank_relative_file_names() {
        for path in [
            "",
            "/a",
            "\\a",
            "C:a",
            "D:/a",
            "../a",
            "ui\\..\\a",
            "a/./b",
            "a\0b",
        ] {
            let file = BankFile {
                path,
                bytes: &[],
                alignment: 0,
            };
            assert!(SkBank::write(&[file], 0).is_err(), "{path:?}");
        }
    }

    #[test]
    fn dot_prefixed_entity_names_match_the_cli() {
        use crate::bank::format::Cursor;
        for (name, stem, ext) in [
            (".env", ".env", ""),
            ("..foo", ".", ".foo"),
            ("...", "..", "."),
        ] {
            let file = BankFile {
                path: name,
                bytes: &[],
                alignment: 1,
            };
            let bytes = SkBank::write(&[file], 0).unwrap();
            let mut fields = Cursor(&bytes[85 + 8 + 16..]);
            assert_eq!(fields.string().unwrap(), stem);
            assert_eq!(fields.string().unwrap(), name);
            assert_eq!(fields.string().unwrap(), name);
            assert_eq!(fields.string().unwrap(), ext);
        }
    }
}
