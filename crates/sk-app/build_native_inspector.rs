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
    format!("{:x}", hash.finalize())
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
        std::fs::write(generated, "const EMBEDDED_MANIFEST: Option<&str> = None;\nconst EMBEDDED_IMAGE_DEFLATE: &[u8] = &[];\nconst EMBEDDED_IMAGE_DLL: Option<&str> = None;\nconst EMBEDDED_IMAGE_BYTES: u64 = 0;\n")
            .expect("cannot write native inspector policy");
        return;
    }
    assert_eq!(
        std::env::var("TARGET").as_deref(),
        Ok("x86_64-pc-windows-msvc"),
        "native-inspector requires Windows x64"
    );
    assert!(std::env::var_os("SK_DEVTOOLS_BUNDLE").is_some(),
        "native-inspector requires SK_DEVTOOLS_BUNDLE from scripts/aot/prepare-inspector.ts; rebuilding JS here would invalidate the image");
    let selected = std::env::var_os("SK_DEVTOOLS_NATIVE_MANIFEST")
        .map(PathBuf::from).expect("native-inspector requires SK_DEVTOOLS_NATIVE_MANIFEST; run scripts/aot/prepare-inspector.ts first");
    let (manifest_text, manifest) = watched_json(&selected);
    let (_, pin) = watched_json(&repo.join("packages/cli/aot/toolchain.lock.json"));
    require(&pin, "schema", "skathi.native-aot-toolchain");
    assert_eq!(
        pin.get("version").and_then(serde_json::Value::as_u64),
        Some(1)
    );
    require(&manifest, "schema", "skathi.native-aot");
    assert_eq!(
        manifest.get("version").and_then(serde_json::Value::as_u64),
        Some(1)
    );
    for (name, expected) in [
        ("stage", "native-image"),
        ("kind", "inspector"),
        ("target", "x86_64-windows"),
        ("transform", "opaque-host-v1"),
    ] {
        require(&manifest, name, expected);
    }
    let frontend = text(&pin["frontend"], "revision");
    let engine = text(&pin["engine"], "revision");
    require(&manifest, "frontend", frontend);
    require(&manifest, "engine", engine);
    require(&manifest, "tableId", text(&pin["table"], "id"));
    require(
        &manifest,
        "compileOptions",
        &format!("x86_64;references=narrow;image=pic;threads=1;admit=FilesRunByHost;default-options@{engine}"),
    );
    require(
        &manifest,
        "sourceBundleSha256",
        &digest(
            File::open(out.join("app.prod.js")).expect("cannot open embedded inspector source"),
        ),
    );
    let mut preludes = std::fs::read(repo.join("crates/sk-app/src/prelude.js"))
        .expect("cannot read inspector prelude");
    preludes.push(0);
    preludes.extend(
        std::fs::read(repo.join("crates/sk-app/src/window.prelude.js"))
            .expect("cannot read window prelude"),
    );
    require(&manifest, "preludeSha256", &digest(preludes.as_slice()));
    for name in ["prelude.js", "window.prelude.js"] {
        println!(
            "cargo:rerun-if-changed={}",
            repo.join("crates/sk-app/src").join(name).display()
        );
    }
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
    require(&manifest, "imageSha256", &format!("{:x}", hash.finalize()));
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
    println!("cargo:rerun-if-env-changed=SK_DEVTOOLS_NATIVE_IMAGE_DLL");
    let image = match std::env::var("SK_DEVTOOLS_NATIVE_IMAGE_DLL").ok().filter(|name| !name.is_empty()) {
        Some(dll) => {
            assert!(
                dll != "." && dll != ".." && !dll.chars().any(|c| matches!(c, '/' | '\\' | ':' | '\0' | '"')),
                "SK_DEVTOOLS_NATIVE_IMAGE_DLL must be a file name beside the executable"
            );
            format!("const EMBEDDED_IMAGE_DEFLATE: &[u8] = &[];\nconst EMBEDDED_IMAGE_DLL: Option<&str> = Some({dll:?});\n")
        }
        None => "const EMBEDDED_IMAGE_DEFLATE: &[u8] = include_bytes!(concat!(env!(\"OUT_DIR\"), \"/devtools/inspector.image.deflate\"));\nconst EMBEDDED_IMAGE_DLL: Option<&str> = None;\n".to_string(),
    };
    std::fs::write(generated, format!("const EMBEDDED_MANIFEST: Option<&str> = Some(include_str!(concat!(env!(\"OUT_DIR\"), \"/devtools/inspector.native.json\")));\n{image}const EMBEDDED_IMAGE_BYTES: u64 = {size};\n"))
        .expect("cannot write embedded inspector declarations");
}
