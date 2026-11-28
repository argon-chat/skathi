use super::*;

const GOLDEN: &[u8] = include_bytes!("../../../packages/cli/test/fixtures/golden.skpk");
const BANK_GOLDEN: &[u8] = include_bytes!("../../../packages/cli/test/fixtures/golden-skbank.skpk");

#[test]
fn rust_and_cli_write_the_same_typed_bank() {
    let legacy = Payload::parse(GOLDEN).unwrap();
    let bytes = Payload::write_bank(legacy.entries(), 0).unwrap();
    assert_eq!(bytes, BANK_GOLDEN);
    let bank = Payload::parse(BANK_GOLDEN).unwrap();
    assert_eq!(bank.data_offset() % 4096, 0);
    for (new, old) in bank.entries().iter().zip(legacy.entries()) {
        assert_eq!(
            (new.kind, new.name, new.data),
            (old.kind, old.name, old.data)
        );
    }
    assert_eq!(bank.entries().len(), legacy.entries().len());
    assert!(bank.asset(name::SCRIPT).is_none());
}

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
    let names_offset = u32::from_le_bytes(GOLDEN[16..20].try_into().unwrap());
    let names_len = u32::from_le_bytes(GOLDEN[20..24].try_into().unwrap());
    let names_end = u64::from(names_offset) + u64::from(names_len);
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
