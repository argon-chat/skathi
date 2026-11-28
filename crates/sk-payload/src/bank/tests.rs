use super::*;

const FIXTURE: &[u8] = include_bytes!("../../tests/fixtures/dehive-store.skpk");

fn toc_start(bank: &[u8]) -> usize {
    let at = bank.len() - 16;
    u64::from_le_bytes(bank[at..at + 8].try_into().unwrap()) as usize
}

#[test]
fn malformed_lengths_offsets_flags_and_dedup_targets_are_rejected() {
    let toc = toc_start(FIXTURE);
    let name = toc + 16;
    let name_len = u32::from_le_bytes(FIXTURE[name..name + 4].try_into().unwrap()) as usize;
    let data_offset = name + 4 + name_len;
    let mut cases = vec![
        (52, u64::MAX.to_le_bytes().to_vec()),
        (toc + 8, u64::MAX.to_le_bytes().to_vec()),
        (name, u32::MAX.to_le_bytes().to_vec()),
        (data_offset, 0_u64.to_le_bytes().to_vec()),
        (data_offset + 8, u64::MAX.to_le_bytes().to_vec()),
        (data_offset + 40, vec![0xff]),
        (data_offset + 41, vec![6]),
        (FIXTURE.len() - 16, u64::MAX.to_le_bytes().to_vec()),
    ];
    let target = b"ui/Logo.png";
    let reference_name = FIXTURE[toc..]
        .windows(target.len())
        .rposition(|p| p == target)
        .unwrap();
    cases.push((toc + reference_name, b"ui/copy.png".to_vec()));
    for (offset, replacement) in cases {
        let mut bytes = FIXTURE.to_owned();
        bytes[offset..offset + replacement.len()].copy_from_slice(&replacement);
        assert!(
            SkBank::parse(&bytes).is_err(),
            "bad table value at {offset}"
        );
    }
    for end in 0..FIXTURE.len() {
        assert!(SkBank::parse(&FIXTURE[..end]).is_err());
    }
}

#[test]
fn paths_use_single_character_ordinal_casing_and_normalized_separators() {
    assert_eq!(path_key("/UI\\café"), path_key("ui/CAFÉ"));
    assert_ne!(path_key("straße"), path_key("STRASSE"));
    assert_ne!(path_key("ſ"), path_key("S"));
    assert_ne!(path_key("ı"), path_key("I"));
    assert_eq!(path_key("ᾀ"), path_key("ᾈ"));
}

#[test]
fn signature_digest_zeroes_only_the_signature_slot() {
    use sha2::{Digest, Sha256};
    let mut bytes = Vec::from(MAGIC.as_slice());
    bytes.extend(FORMAT.to_le_bytes());
    bytes.push(0xaa);
    bytes.extend(4u64.to_le_bytes());
    bytes.extend([0; 56]);
    bytes.extend([0; 256]);
    bytes.extend(0u64.to_le_bytes());
    bytes.push(0xdd);
    let toc = bytes.len() as u64;
    bytes.extend(TOC_MAGIC.to_le_bytes());
    bytes.extend(0u64.to_le_bytes());
    bytes.extend(toc.to_le_bytes());
    bytes.extend(END_MAGIC.to_le_bytes());
    let expected: [u8; 32] = Sha256::digest(&bytes).into();
    bytes[76..332].fill(0xae);
    let bank = SkBank::parse(&bytes).unwrap();
    assert_eq!(bank.signature().unwrap(), &[0xae; 256]);
    assert_eq!(bank.signature_digest(), Some(expected));
    assert_eq!(SkBank::parse(FIXTURE).unwrap().signature_digest(), None);
}

#[test]
fn invalid_chunk_tables_fail_before_decompression() {
    let mut bytes = Vec::from([0x42]);
    bytes.extend(8u64.to_le_bytes());
    bytes.extend(8u32.to_le_bytes());
    bytes.extend(1u32.to_le_bytes());
    bytes.extend(2u32.to_le_bytes());
    bytes.extend(20u32.to_le_bytes());
    let encoded = Encoded {
        bytes: &bytes,
        codec: BankCodec::Brotli,
        chunked: true,
        scramble: None,
    };
    assert!(encoded.read(8).is_err());
    assert!(encoded.read_range(9..9).is_err());
    assert!(encoded.read_range(5..5).is_err());
}
