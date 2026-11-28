use std::borrow::Cow;

use sk_payload::{Payload, SkBank};

// Written by the .NET DeHive v3 writer at f394f2a, then checked with `hive verify`.
const STORE: &[u8] = include_bytes!("fixtures/dehive-store.skpk");
const BROTLI: &[u8] = include_bytes!("fixtures/dehive-brotli.skpk");
const ZSTD: &[u8] = include_bytes!("fixtures/dehive-zstd.skpk");
const TYPED: &[u8] = include_bytes!("fixtures/dehive-typed.skpk");
const LOGO: &[u8] = &[0, 1, 2, 127, 128, 255];

#[test]
fn dotnet_files_can_address_private_kinds_and_case_sensitive_skathi_assets() {
    let p = Payload::parse(TYPED).unwrap();
    assert_eq!(p.meta(), Some(r#"{"name":"interop"}"#));
    assert_eq!(p.script(), Some("private script"));
    assert_eq!(p.asset("sk/app.js"), Some(b"public asset".as_slice()));
    assert_eq!(p.asset("ui/Logo.png"), Some(b"upper".as_slice()));
    assert_eq!(
        p.asset("ui/copy.png").unwrap().as_ptr(),
        p.asset("ui/Logo.png").unwrap().as_ptr()
    );
    assert_eq!(p.asset("ui/logo.png"), Some(b"lower".as_slice()));
    assert_eq!(p.asset("ui/café.txt"), Some(b"SkBank".as_slice()));
    assert!(p.asset("sk/app.json").is_none());
}

#[test]
fn stored_aliases_borrow_the_same_range_from_the_dotnet_bank() {
    let bank = SkBank::parse(STORE).unwrap();
    let logo = bank.entry(r"UI\LOGO.PNG").unwrap();
    let alias = bank.entry("ui/copy.png").unwrap();
    let bytes = logo.stored_bytes().unwrap();
    assert_eq!(bytes, LOGO);
    assert_eq!(logo.crc64, 0x8486_6280_1ba8_92f1);
    assert_eq!(alias.stored_bytes().unwrap().as_ptr(), bytes.as_ptr());
    assert!(STORE.as_ptr_range().contains(&bytes.as_ptr()));
    assert!(matches!(alias.read(LOGO.len()).unwrap(), Cow::Borrowed(_)));
    assert_eq!(bank.entry_by_id(&alias.id).unwrap().path, "ui/copy.png");
    assert_eq!(
        bank.entry("café.txt").unwrap().read(6).unwrap().as_ref(),
        b"SkBank"
    );
    assert!(bank.entry("empty").unwrap().read(0).unwrap().is_empty());
}

#[test]
fn stored_integrity_is_lazy_and_verifies_the_original_bytes() {
    let mut bytes = STORE.to_vec();
    let offset = {
        let bank = SkBank::parse(&bytes).unwrap();
        let payload = bank.entry("ui/Logo.png").unwrap().stored_bytes().unwrap();
        payload.as_ptr() as usize - bytes.as_ptr() as usize
    };
    bytes[offset] ^= 0xff;
    let bank = SkBank::parse(&bytes).unwrap();
    assert!(bank
        .entry("ui/Logo.png")
        .unwrap()
        .verify(LOGO.len())
        .is_err());
    assert!(bank
        .entry("ui/copy.png")
        .unwrap()
        .verify(LOGO.len())
        .is_err());
    bank.entry("café.txt").unwrap().verify(6).unwrap();
}

#[test]
fn dotnet_scrambled_store_whole_entity_and_chunked_codecs_decode_with_a_limit() {
    let chunks = "0123456789abcdef".repeat(193) + "end";
    for fixture in [BROTLI, ZSTD] {
        let bank = SkBank::parse(fixture).unwrap();
        for (path, expected) in [
            ("raw.png", LOGO),
            ("small.txt", b"SkBank".as_slice()),
            ("chunks.bin", chunks.as_bytes()),
        ] {
            let entry = bank.entry(path).unwrap();
            assert!(entry.stored_bytes().is_none());
            assert!(entry.read(expected.len() - 1).is_err());
            assert_eq!(entry.read(expected.len()).unwrap().as_ref(), expected);
            entry.verify(expected.len()).unwrap();
        }
        let entry = bank.entry("chunks.bin").unwrap();
        assert!(entry.is_chunked());
        assert_eq!(
            entry.read_range(1019..1031).unwrap().as_ref(),
            &chunks.as_bytes()[1019..1031]
        );
        assert_eq!(
            entry
                .read_range(chunks.len() - 3..chunks.len())
                .unwrap()
                .as_ref(),
            b"end"
        );
        assert_eq!(
            bank.entry("raw.png")
                .unwrap()
                .read_range(2..5)
                .unwrap()
                .as_ref(),
            &LOGO[2..5]
        );
    }
}
