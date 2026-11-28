//! Strict packaging policy for the embedded native inspector. No tool is run
//! here: the release orchestrator supplies one bundle and its matching image.
use sha2::{Digest, Sha256};
use std::{
    fs::File,
    io::{Read, Write},
    path::{Path, PathBuf},
};

const MAX_IMAGE_BYTES: u64 = 256 * 1024 * 1024;

fn digest(reader: impl Read) -> String {
    let mut hash = Sha256::new();
    let mut reader = reader;
    let mut buffer = [0u8; 64 * 1024];
    loop {
        let count = reader
            .read(&mut buffer)
            .expect("cannot hash native inspector input");
        if count == 0 {
            break;
        }
        hash.update(&buffer[..count]);
    }
    hex::encode(hash.finalize())
}
fn text<'a>(value: &'a serde_json::Value, name: &str) -> &'a str {
    value
        .get(name)
        .and_then(serde_json::Value::as_str)
        .unwrap_or_else(|| panic!("native inspector manifest lacks {name}"))
}
fn require(value: &serde_json::Value, name: &str, expected: &str) {
    assert_eq!(
        text(value, name),
        expected,
        "native inspector {name} differs from this build"
    );
}
fn watched_json(path: &Path) -> (String, serde_json::Value) {
    println!("cargo:rerun-if-changed={}", path.display());
    let bytes = std::fs::read(path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    assert!(
        bytes.len() <= 65_536,
        "native inspector manifest exceeds 64 KiB"
    );
    let string = String::from_utf8(bytes).expect("native inspector manifest is not UTF-8");
    let value = serde_json::from_str(&string).expect("invalid native inspector JSON");
    (string, value)
}

pub fn prepare(repo: &Path, out: &Path) {
    println!("cargo:rerun-if-env-changed=SK_DEVTOOLS_NATIVE_MANIFEST");
    let generated = out.join("native_inspector.rs");
    if std::env::var_os("CARGO_FEATURE_NATIVE_INSPECTOR").is_none() {
        std::fs::write(generated, "const EMBEDDED_MANIFEST: Option<&str> = None;\nconst EMBEDDED_IMAGE_DEFLATE: &[u8] = &[];\nconst EMBEDDED_IMAGE_DLL: Option<&str> = None;\nconst EMBEDDED_IMAGE_CONTAINER: Option<&str> = None;\nconst EMBEDDED_IMAGE_BYTES: u64 = 0;\n")
            .expect("cannot write native inspector policy");
        return;
    }
    let (target, options) = match std::env::var("TARGET").as_deref() {
        Ok("x86_64-pc-windows-msvc") => ("x86_64-windows", "x86_64;references=narrow"),
        Ok("aarch64-apple-darwin") => ("aarch64-macos", "aarch64;os=macos;references=wide"),
        _ => panic!("native-inspector requires Windows x64 or macOS arm64"),
    };
    assert!(std::env::var_os("SK_DEVTOOLS_BUNDLE").is_some(),
        "native-inspector requires SK_DEVTOOLS_BUNDLE from scripts/aot/prepare-inspector.ts; rebuilding JS here would invalidate the image");
    let selected = std::env::var_os("SK_DEVTOOLS_NATIVE_MANIFEST")
        .map(PathBuf::from).expect("native-inspector requires SK_DEVTOOLS_NATIVE_MANIFEST; run scripts/aot/prepare-inspector.ts first");
    let (manifest_text, manifest) = watched_json(&selected);
    require(&manifest, "schema", "skathi.native-aot");
    assert_eq!(
        manifest.get("version").and_then(serde_json::Value::as_u64),
        Some(1)
    );
    for (name, expected) in [
        ("stage", "native-image"),
        ("kind", "inspector"),
        ("target", target),
        ("transform", "opaque-host-v1"),
    ] {
        require(&manifest, name, expected);
    }
    let engine = std::env::var("DEP_REDJS_WEBKIT_VERSION").expect("redjs reports its engine pin");
    require(&manifest, "engine", &engine);
    let lock = repo.join("packages/cli/aot/toolchain.lock.json");
    if lock.is_file() {
        let (_, pin) = watched_json(&lock);
        require(&pin, "schema", "skathi.native-aot-toolchain");
        assert_eq!(
            pin.get("version").and_then(serde_json::Value::as_u64),
            Some(1)
        );
        require(&manifest, "frontend", text(&pin["frontend"], "revision"));
        require(&manifest, "tableId", text(&pin["table"], "id"));
    }
    require(
        &manifest,
        "compileOptions",
        &format!("{options};image=pic;threads=1;default-options@{engine}"),
    );
    require(
        &manifest,
        "sourceBundleSha256",
        &digest(
            File::open(out.join("app.prod.js")).expect("cannot open embedded inspector source"),
        ),
    );
    let mut preludes = include_bytes!("src/prelude.js").to_vec();
    preludes.push(0);
    preludes.extend_from_slice(include_bytes!("src/window.prelude.js"));
    require(&manifest, "preludeSha256", &digest(preludes.as_slice()));
    let entry = manifest.get("entry").and_then(serde_json::Value::as_u64);
    assert!(
        entry.is_some_and(|entry| entry > 0 && entry <= u32::MAX as u64),
        "invalid native inspector entry"
    );
    let identity = text(&manifest, "identity");
    assert!(
        identity.len() == 32 && identity.bytes().all(|byte| byte.is_ascii_hexdigit()),
        "invalid native inspector identity"
    );
    let name = text(&manifest, "image");
    assert!(
        !name.is_empty()
            && name != "."
            && name != ".."
            && !name.chars().any(|c| matches!(c, '/' | '\\' | ':' | '\0')),
        "native image must be a sibling filename"
    );
    let image = selected
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join(name);
    println!("cargo:rerun-if-changed={}", image.display());
    let source = File::open(&image).unwrap_or_else(|e| panic!("{}: {e}", image.display()));
    let size = source
        .metadata()
        .expect("cannot stat native inspector image")
        .len();
    assert!(
        size > 0 && size <= MAX_IMAGE_BYTES,
        "native inspector image must be within 256 MiB"
    );
    let destination = File::create(out.join("inspector.image.deflate"))
        .expect("cannot create compressed native image");
    let mut compressor =
        flate2::write::DeflateEncoder::new(destination, flate2::Compression::best());
    let mut hash = Sha256::new();
    let mut buffer = [0u8; 64 * 1024];
    let mut source = source.take(MAX_IMAGE_BYTES + 1);
    let mut count = 0;
    loop {
        let length = source.read(&mut buffer).expect("cannot read native image");
        if length == 0 {
            break;
        }
        hash.update(&buffer[..length]);
        compressor
            .write_all(&buffer[..length])
            .expect("cannot compress native image");
        count += length as u64;
    }
    require(&manifest, "imageSha256", &hex::encode(hash.finalize()));
    assert_eq!(
        count, size,
        "native inspector input changed while packaging"
    );
    compressor
        .finish()
        .expect("cannot finish native image compression")
        .flush()
        .expect("cannot flush native image");
    std::fs::write(out.join("inspector.native.json"), manifest_text)
        .expect("cannot embed native manifest");
    // `SK_DEVTOOLS_NATIVE_IMAGE_DLL=inspector.dll`: the compressed image is the
    // `SKINSPECTOR` RCDATA of that resource-only DLL beside the executable,
    // not part of it. The manifest, and with it the image's SHA-256, stays in
    // the executable, so the DLL is checked like the embedded copy is.
    //
    // `SK_DEVTOOLS_NATIVE_IMAGE_CONTAINER=inspector.image.dll`: the image, uncompressed, is the
    // `.skins` sections of that container beside the executable (scripts/release-hosts.ts writes
    // it), mapped in place.
    println!("cargo:rerun-if-env-changed=SK_DEVTOOLS_NATIVE_IMAGE_DLL");
    println!("cargo:rerun-if-env-changed=SK_DEVTOOLS_NATIVE_IMAGE_CONTAINER");
    let beside = |variable: &str| {
        let name = std::env::var(variable)
            .ok()
            .filter(|name| !name.is_empty())?;
        assert!(
            name != "."
                && name != ".."
                && !name
                    .chars()
                    .any(|c| matches!(c, '/' | '\\' | ':' | '\0' | '"')),
            "{variable} must be a file name beside the executable"
        );
        Some(name)
    };
    let image = match (beside("SK_DEVTOOLS_NATIVE_IMAGE_DLL"), beside("SK_DEVTOOLS_NATIVE_IMAGE_CONTAINER")) {
        (Some(_), Some(_)) => panic!("set SK_DEVTOOLS_NATIVE_IMAGE_DLL or SK_DEVTOOLS_NATIVE_IMAGE_CONTAINER, not both"),
        (Some(dll), None) => format!("const EMBEDDED_IMAGE_DEFLATE: &[u8] = &[];\nconst EMBEDDED_IMAGE_DLL: Option<&str> = Some({dll:?});\nconst EMBEDDED_IMAGE_CONTAINER: Option<&str> = None;\n"),
        (None, Some(container)) => format!("const EMBEDDED_IMAGE_DEFLATE: &[u8] = &[];\nconst EMBEDDED_IMAGE_DLL: Option<&str> = None;\nconst EMBEDDED_IMAGE_CONTAINER: Option<&str> = Some({container:?});\n"),
        (None, None) => "const EMBEDDED_IMAGE_DEFLATE: &[u8] = include_bytes!(concat!(env!(\"OUT_DIR\"), \"/devtools/inspector.image.deflate\"));\nconst EMBEDDED_IMAGE_DLL: Option<&str> = None;\nconst EMBEDDED_IMAGE_CONTAINER: Option<&str> = None;\n".to_string(),
    };
    std::fs::write(generated, format!("const EMBEDDED_MANIFEST: Option<&str> = Some(include_str!(concat!(env!(\"OUT_DIR\"), \"/devtools/inspector.native.json\")));\n{image}const EMBEDDED_IMAGE_BYTES: u64 = {size};\n"))
        .expect("cannot write embedded inspector declarations");
}
