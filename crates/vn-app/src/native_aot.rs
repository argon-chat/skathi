//! Embedded native inspector. A selected image is trusted executable code.
//! Its compiler, host table, preludes and embedded Vue bundle must all match.
use crate::json::{self, Json};
use crate::inspector::{engine_policy, EnginePolicy};
use std::{
    cell::RefCell,
    ffi::OsStr,
    fs::File,
    io::Read,
    path::{Path, PathBuf},
};
use vn_runtime::{
    redjs::{aot, api::Error},
    JsRuntime,
};

mod cache;
include!(concat!(env!("OUT_DIR"), "/devtools/native_inspector.rs"));

const SCHEMA: &str = "vue-native.native-aot";
const TRANSFORM: &str = "opaque-host-v1";
struct Plan {
    entry: u32,
}
thread_local! { static PLAN: RefCell<Option<Plan>> = const { RefCell::new(None) }; }

fn field<'a>(manifest: &'a Json, name: &str) -> Result<&'a str, String> {
    manifest
        .get(name)
        .and_then(Json::as_str)
        .ok_or_else(|| format!("native AOT manifest lacks {name}"))
}
fn equal(manifest: &Json, name: &str, expected: &str) -> Result<(), String> {
    if field(manifest, name)? == expected {
        Ok(())
    } else {
        Err(format!("native AOT {name} differs from this host build"))
    }
}
fn identity(text: &str) -> Result<[u8; 16], String> {
    if text.len() != 32 || !text.is_ascii() {
        return Err("invalid AOT identity".into());
    }
    let mut result = [0; 16];
    for (index, byte) in result.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&text[index * 2..index * 2 + 2], 16)
            .map_err(|_| "invalid AOT identity")?;
    }
    Ok(result)
}
fn image_path(manifest: &Json, beside: &Path) -> Result<PathBuf, String> {
    let name = field(manifest, "image")?;
    if name.is_empty()
        || name.chars().any(|c| matches!(c as u32, 0 | 47 | 58 | 92))
        || name == "."
        || name == ".."
    {
        return Err("AOT image must be a file beside its manifest".into());
    }
    Ok(beside.join(name))
}

fn validate(
    manifest: &Json,
    source: &[u8],
    preludes: &[u8],
    image_digest: &str,
) -> Result<(u32, [u8; 16]), String> {
    equal(manifest, "schema", SCHEMA)?;
    if manifest.get("version").and_then(Json::as_f64) != Some(1.0) {
        return Err("unsupported AOT manifest version".into());
    }
    equal(manifest, "stage", "native-image")?;
    equal(manifest, "kind", "inspector")?;
    equal(manifest, "target", "x86_64-windows")?;
    equal(manifest, "frontend", aot::FRONTEND_REVISION)?;
    equal(manifest, "engine", aot::ENGINE_VERSION)?;
    equal(manifest, "tableId", aot::BUILD_TABLE_ID)?;
    equal(manifest, "compileOptions", aot::COMPILE_OPTIONS)?;
    equal(manifest, "transform", TRANSFORM)?;
    equal(manifest, "sourceBundleSha256", &aot::digest_hex(source))?;
    equal(manifest, "preludeSha256", &aot::digest_hex(preludes))?;
    equal(manifest, "imageSha256", image_digest)?;
    let entry = manifest
        .get("entry")
        .and_then(Json::as_f64)
        .filter(|n| n.is_finite() && *n >= 1.0 && *n <= u32::MAX as f64 && n.fract() == 0.0)
        .ok_or("invalid AOT entry")? as u32;
    let id = identity(field(manifest, "identity")?)?;
    Ok((entry, id))
}

const MAX_MANIFEST_BYTES: u64 = 65_536;

fn read_manifest(path: &Path) -> Result<String, String> {
    let file = File::open(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let mut text = String::new();
    file.take(MAX_MANIFEST_BYTES + 1)
        .read_to_string(&mut text)
        .map_err(|e| format!("{}: {e}", path.display()))?;
    if text.len() as u64 > MAX_MANIFEST_BYTES {
        return Err("AOT manifest is too large".into());
    }
    Ok(text)
}

fn memory_profile(value: Option<&OsStr>) -> Result<aot::MemoryProfile, String> {
    match value {
        None => Ok(aot::MemoryProfile::Compact),
        Some(value) => match value.to_str() {
            Some("compact") => Ok(aot::MemoryProfile::Compact),
            Some("default") => Ok(aot::MemoryProfile::Default),
            _ => Err("VN_AOT_MEMORY_PROFILE must be compact or default".into()),
        },
    }
}

/// Hash and load this exact file object, including when a symlink is retargeted.
/// The native loader duplicates the handle; in-place writes stay disabled.
/// Atomic replacement leaves this original file object valid for execution.
fn open_image(path: &Path) -> std::io::Result<File> {
    let mut options = std::fs::OpenOptions::new();
    options.read(true);
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        options.access_mode(0x8000_0000 | 0x2000_0000); // GENERIC_READ | GENERIC_EXECUTE
        options.share_mode(1 | 4); // FILE_SHARE_READ | FILE_SHARE_DELETE; no writes
    }
    options.open(path)
}
/// The compressed image a build with `VN_DEVTOOLS_NATIVE_IMAGE_DLL` keeps in
/// a resource-only DLL beside the executable: its `VNINSPECTOR` RCDATA. The
/// DLL is mapped as data, never run, and stays mapped for the process; what
/// it holds is trusted only once it inflates to the manifest's `imageSha256`.
#[cfg(windows)]
fn image_resource(dll: &str) -> Result<&'static [u8], String> {
    use windows::core::{w, HSTRING, PCWSTR};
    use windows::Win32::System::LibraryLoader::{
        FindResourceW, LoadLibraryExW, LoadResource, LockResource, SizeofResource,
        LOAD_LIBRARY_AS_DATAFILE, LOAD_LIBRARY_AS_IMAGE_RESOURCE,
    };
    const RT_RCDATA: PCWSTR = PCWSTR(10 as _);
    let exe = std::env::current_exe().map_err(|e| format!("native inspector: {e}"))?;
    let path = exe.parent().ok_or("native inspector: the executable has no folder")?.join(dll);
    let missing = |what: &str| format!("{}: {what}", path.display());
    unsafe {
        let module = LoadLibraryExW(
            &HSTRING::from(path.as_os_str()),
            None,
            LOAD_LIBRARY_AS_DATAFILE | LOAD_LIBRARY_AS_IMAGE_RESOURCE,
        )
        .map_err(|e| missing(&e.to_string()))?;
        let resource = FindResourceW(Some(module), w!("VNINSPECTOR"), RT_RCDATA);
        if resource.is_invalid() {
            return Err(missing("no VNINSPECTOR resource"));
        }
        let global = LoadResource(Some(module), resource).map_err(|e| missing(&e.to_string()))?;
        let pointer = LockResource(global) as *const u8;
        let length = SizeofResource(Some(module), resource) as usize;
        if pointer.is_null() || length == 0 {
            return Err(missing("an empty VNINSPECTOR resource"));
        }
        // SAFETY: the module is never freed, and a data mapping is read-only.
        Ok(std::slice::from_raw_parts(pointer, length))
    }
}
#[cfg(not(windows))]
fn image_resource(dll: &str) -> Result<&'static [u8], String> {
    Err(format!("native inspector: {dll} is a Windows resource DLL"))
}
/// On the actual VM thread, before ANY JSC initialization. The parent exits
/// before even reading selectors or referencing compressed native-image bytes.
/// Failure cannot switch a selected process back to JIT: launch a clean child.
pub(crate) fn prepare(devtools: bool) -> Result<(), String> {
    if !devtools {
        return Ok(());
    }
    let policy = engine_policy(std::env::var_os("VN_DEVTOOLS_ENGINE").as_deref())?;
    if policy == EnginePolicy::Jit {
        if aot::is_active() {
            return Err("cannot switch an initialized native inspector to JIT".into());
        }
        eprintln!("[native-aot] explicit JIT inspector selected before initialization");
        return Ok(());
    }
    // This explicit diagnostic override remains application-selected native
    // code. Ordinary releases select only the manifest signed into their exe.
    let external = std::env::var_os("VN_DEVTOOLS_NATIVE_IMAGE").map(PathBuf::from);
    let (text, external) = if let Some(path) = external {
        let path =
            std::fs::canonicalize(path).map_err(|e| format!("native AOT manifest path: {e}"))?;
        (read_manifest(&path)?, Some(path))
    } else if let Some(text) = EMBEDDED_MANIFEST {
        (text.to_string(), None)
    } else if policy == EnginePolicy::Native {
        return Err(
            "native inspector requested but this build has no embedded native image".into(),
        );
    } else {
        return Ok(());
    };
    let profile = memory_profile(std::env::var_os("VN_AOT_MEMORY_PROFILE").as_deref())?;
    let manifest = json::parse(&text).ok_or("invalid native AOT manifest JSON")?;
    let source =
        crate::inspector::inspector_js().map_err(|e| format!("embedded inspector: {e}"))?;
    let mut preludes = crate::PRELUDE_JS.as_bytes().to_vec();
    preludes.push(0);
    preludes.extend_from_slice(crate::WINDOW_PRELUDE_JS.as_bytes());
    // Validate ABI and the actual embedded JS/preludes before cache writes or
    // native initialization. The image hash is checked against the exact File.
    let expected = field(&manifest, "imageSha256")?;
    let (entry, id) = validate(&manifest, source.as_bytes(), &preludes, expected)?;
    let (mut file, description) = if let Some(path) = external {
        let image = image_path(&manifest, path.parent().unwrap_or_else(|| Path::new(".")))?;
        let file = open_image(&image).map_err(|e| format!("{}: {e}", image.display()))?;
        let size = file.metadata().map_err(|e| e.to_string())?.len();
        if size == 0 || size > cache::MAX_IMAGE_BYTES {
            return Err("native AOT image exceeds the supported size".into());
        }
        (file, image.display().to_string())
    } else {
        let compressed = match EMBEDDED_IMAGE_DLL {
            Some(dll) => image_resource(dll)?,
            None => EMBEDDED_IMAGE_DEFLATE,
        };
        (
            cache::materialize(compressed, expected, EMBEDDED_IMAGE_BYTES)?,
            format!("embedded sha256:{expected}"),
        )
    };
    let image_digest = aot::digest_reader(&mut file).map_err(|e| format!("{description}: {e}"))?;
    equal(&manifest, "imageSha256", &image_digest)?;
    use std::io::Seek;
    file.rewind().map_err(|e| e.to_string())?;
    // The manifest embedded in a signed executable supplies provenance. A
    // checksum on a manually selected external image is only integrity/ABI.
    unsafe { aot::load_image_file_with_profile(&file, id, profile) }?;
    drop(file);
    if let Some(settings) = aot::memory_settings() {
        eprintln!(
            "[native-aot] structure heap {} KiB; cell cage {} MiB; source view read-only {}",
            settings.structure_heap_size_kb,
            settings.marked_block_cage_size_mb,
            settings.initial_image_read_only
        );
    }
    PLAN.with(|plan| *plan.borrow_mut() = Some(Plan { entry }));
    eprintln!(
        "[native-aot] selected inspector image {description} (entry {entry}, memory {profile:?})"
    );
    Ok(())
}
/// Returns false for ordinary JIT boots. A selected native image never falls
/// through to source evaluation if its entry throws or remains pending.
pub(crate) fn boot(rt: &JsRuntime) -> Result<bool, String> {
    let Some(entry) = PLAN.with(|plan| plan.borrow().as_ref().map(|p| p.entry)) else {
        return Ok(false);
    };
    let ctx = rt.ctx();
    // JS sees the realm's globalThis proxy, not JSC's raw JSGlobalObject.
    // Preserve identity for host WeakMaps and window/document event routing.
    let host = ctx.global_get("globalThis").map_err(|error| error.to_string())?;
    ctx.global_set("__VN_AOT_HOST__", host)
        .map_err(|e| e.to_string())?;
    match ctx.run_aot_module(entry) {
        Ok(aot::ModuleEvaluation::Fulfilled(_)) => {
            eprintln!("[native-aot] inspector native entry completed");
            Ok(true)
        }
        Ok(aot::ModuleEvaluation::Pending(_)) => {
            Err("inspector native entry is awaiting host work; boot is not complete".into())
        }
        Err(Error::Exception(value)) => Err(ctx
            .to_rust_string(value)
            .unwrap_or_else(|_| "native JS exception".into())),
        Err(error) => Err(error.to_string()),
    }
}
#[cfg(test)]
mod tests {
    #[test]
    fn engine_escape_is_explicit_and_invalid_policies_fail_closed() {
        assert_eq!(engine_policy(None).unwrap(), EnginePolicy::Auto);
        assert_eq!(
            engine_policy(Some(OsStr::new("auto"))).unwrap(),
            EnginePolicy::Auto
        );
        assert_eq!(
            engine_policy(Some(OsStr::new("native"))).unwrap(),
            EnginePolicy::Native
        );
        assert_eq!(
            engine_policy(Some(OsStr::new("jit"))).unwrap(),
            EnginePolicy::Jit
        );
        for policy in ["", "default", "fallback", "JIT"] {
            assert!(engine_policy(Some(OsStr::new(policy))).is_err());
        }
    }
    use super::*;
    struct TempArtifact(std::path::PathBuf);
    impl TempArtifact {
        fn new(bytes: &[u8]) -> Self {
            use std::io::Write;
            static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
            let path = std::env::temp_dir().join(format!(
                "vn-aot-manifest-{}-{}.bin",
                std::process::id(),
                NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
            ));
            let mut file = std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&path)
                .unwrap();
            file.write_all(bytes).unwrap();
            Self(path)
        }
    }
    impl Drop for TempArtifact {
        fn drop(&mut self) {
            let _ = std::fs::remove_file(&self.0);
        }
    }

    #[test]
    fn rejects_oversized_manifest_before_parsing_or_starting_jsc() {
        let exact = TempArtifact::new(&vec![b' '; MAX_MANIFEST_BYTES as usize]);
        assert_eq!(
            read_manifest(&exact.0).unwrap().len(),
            MAX_MANIFEST_BYTES as usize
        );
        let oversized = TempArtifact::new(&vec![b' '; 2 * MAX_MANIFEST_BYTES as usize]);
        assert_eq!(
            read_manifest(&oversized.0).unwrap_err(),
            "AOT manifest is too large"
        );
        let invalid_utf8 = TempArtifact::new(&[0xff]);
        assert!(read_manifest(&invalid_utf8.0).is_err());
    }

    #[cfg(windows)]
    #[test]
    fn verified_image_handle_pins_original_bytes_across_atomic_replacement() {
        let artifact = TempArtifact::new(b"trusted native image");
        let mut image = open_image(&artifact.0).unwrap();
        assert_eq!(
            aot::digest_reader(&mut image).unwrap(),
            aot::digest_hex(b"trusted native image")
        );
        assert!(std::fs::OpenOptions::new()
            .write(true)
            .open(&artifact.0)
            .is_err());
        let old = TempArtifact::new(b"placeholder");
        std::fs::remove_file(&old.0).unwrap();
        std::fs::rename(&artifact.0, &old.0).unwrap();
        std::fs::write(&artifact.0, b"a new artifact at the same path").unwrap();
        use std::io::Seek;
        image.rewind().unwrap();
        assert_eq!(
            aot::digest_reader(&mut image).unwrap(),
            aot::digest_hex(b"trusted native image")
        );
        assert!(std::fs::OpenOptions::new()
            .write(true)
            .open(&old.0)
            .is_err());
        drop(image);
        assert!(std::fs::OpenOptions::new().write(true).open(&old.0).is_ok());
    }
    fn valid() -> Json {
        Json::obj(vec![
            ("schema", Json::s(SCHEMA)),
            ("version", Json::n(1.0)),
            ("stage", Json::s("native-image")),
            ("kind", Json::s("inspector")),
            ("target", Json::s("x86_64-windows")),
            ("frontend", Json::s(aot::FRONTEND_REVISION)),
            ("engine", Json::s(aot::ENGINE_VERSION)),
            ("tableId", Json::s(aot::BUILD_TABLE_ID)),
            ("compileOptions", Json::s(aot::COMPILE_OPTIONS)),
            ("transform", Json::s(TRANSFORM)),
            ("sourceBundleSha256", Json::s(aot::digest_hex(b"bundle"))),
            ("preludeSha256", Json::s(aot::digest_hex(b"preludes"))),
            ("imageSha256", Json::s(aot::digest_hex(b"image"))),
            ("entry", Json::n(1.0)),
            ("identity", Json::s("00000000000000000000000000000000")),
            ("image", Json::s("devtools.image")),
        ])
    }
    fn change(document: &mut Json, name: &str, value: Json) {
        let Json::Obj(entries) = document else {
            unreachable!()
        };
        *entries.iter_mut().find(|(key, _)| key == name).unwrap() = (name.to_string(), value);
    }
    #[test]
    fn rejects_stale_artifacts_before_loading_native_code() {
        assert!(validate(&valid(), b"bundle", b"preludes", &aot::digest_hex(b"image")).is_ok());
        for name in [
            "kind",
            "frontend",
            "engine",
            "tableId",
            "compileOptions",
            "transform",
            "sourceBundleSha256",
            "preludeSha256",
            "imageSha256",
        ] {
            let mut document = valid();
            change(&mut document, name, Json::s("stale"));
            assert!(validate(
                &document,
                b"bundle",
                b"preludes",
                &aot::digest_hex(b"image")
            )
            .unwrap_err()
            .contains(name));
        }
        assert!(validate(
            &valid(),
            b"changed",
            b"preludes",
            &aot::digest_hex(b"image")
        )
        .is_err());
        assert!(validate(
            &valid(),
            b"bundle",
            b"preludes",
            &aot::digest_hex(b"corrupt")
        )
        .is_err());
    }
    #[test]
    fn entries_are_finite_positive_one_based_ids() {
        for entry in [
            0.0,
            -1.0,
            1.5,
            f64::NAN,
            f64::INFINITY,
            u32::MAX as f64 + 1.0,
        ] {
            let mut document = valid();
            change(&mut document, "entry", Json::n(entry));
            assert!(validate(
                &document,
                b"bundle",
                b"preludes",
                &aot::digest_hex(b"image")
            )
            .is_err());
        }
    }
    #[test]
    fn image_must_be_a_normal_filename_beside_manifest() {
        for name in [
            "",
            ".",
            "..",
            "../x",
            "x/y",
            r"x\y",
            r"C:foo.image",
            "foo:stream",
            "x\0y",
        ] {
            let mut document = valid();
            change(&mut document, "image", Json::s(name));
            assert!(
                image_path(&document, Path::new("probe")).is_err(),
                "{name:?}"
            );
        }
        assert_eq!(
            image_path(&valid(), Path::new("probe")).unwrap(),
            Path::new("probe").join("devtools.image")
        );
    }
}
