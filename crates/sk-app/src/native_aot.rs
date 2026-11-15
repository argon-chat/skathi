//! Native AOT images: the embedded inspector's, and a packaged app's own (`sk build --aot`).
//! A selected image is trusted executable code. Its compiler, host table, preludes and
//! JavaScript source must all match this host.
#![cfg_attr(not(feature = "prod"), allow(dead_code))]
#[cfg(sk_devtools)]
use crate::inspector::{engine_policy, EnginePolicy};
use crate::json::{self, Json};
use sk_runtime::{
    redjs::{aot, api::Error},
    JsRuntime,
};
use std::{cell::RefCell, ffi::OsStr, fs::File, path::Path};
#[cfg(sk_devtools)]
use std::{io::Read, path::PathBuf};

mod cache;
#[cfg(sk_devtools)]
include!(concat!(env!("OUT_DIR"), "/devtools/native_inspector.rs"));

const SCHEMA: &str = "skathi.native-aot";
const TRANSFORM: &str = "opaque-host-v1";
const TARGET: &str = "x86_64-windows";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Kind {
    #[cfg_attr(not(sk_devtools), allow(dead_code))]
    Inspector,
    Application,
}
impl Kind {
    fn name(self) -> &'static str {
        match self {
            Kind::Inspector => "inspector",
            Kind::Application => "application",
        }
    }
}

struct Plan {
    entry: u32,
    kind: Kind,
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
#[cfg(sk_devtools)]
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
fn positive_integer(manifest: &Json, name: &str, max: f64) -> Option<f64> {
    manifest
        .get(name)
        .and_then(Json::as_f64)
        .filter(|n| n.is_finite() && *n >= 1.0 && *n <= max && n.fract() == 0.0)
}

fn preludes() -> Vec<u8> {
    let mut preludes = crate::PRELUDE_JS.as_bytes().to_vec();
    preludes.push(0);
    preludes.extend_from_slice(crate::WINDOW_PRELUDE_JS.as_bytes());
    preludes
}

/// What an image must have been compiled against to run on this host; `--sk-host-info` reports it.
pub(crate) fn host_identity() -> Json {
    Json::obj(vec![
        ("target", Json::s(TARGET)),
        ("frontend", Json::s(aot::FRONTEND_REVISION)),
        ("engine", Json::s(aot::ENGINE_VERSION)),
        ("tableId", Json::s(aot::BUILD_TABLE_ID)),
        ("compileOptions", Json::s(aot::COMPILE_OPTIONS)),
        ("transform", Json::s(TRANSFORM)),
        ("preludeSha256", Json::s(aot::digest_hex(&preludes()))),
    ])
}

fn validate(
    manifest: &Json,
    kind: Kind,
    source: &[u8],
    preludes: &[u8],
    image_digest: &str,
) -> Result<(u32, [u8; 16]), String> {
    equal(manifest, "schema", SCHEMA)?;
    if manifest.get("version").and_then(Json::as_f64) != Some(1.0) {
        return Err("unsupported AOT manifest version".into());
    }
    equal(manifest, "stage", "native-image")?;
    equal(manifest, "kind", kind.name())?;
    equal(manifest, "target", TARGET)?;
    equal(manifest, "frontend", aot::FRONTEND_REVISION)?;
    equal(manifest, "engine", aot::ENGINE_VERSION)?;
    equal(manifest, "tableId", aot::BUILD_TABLE_ID)?;
    equal(manifest, "compileOptions", aot::COMPILE_OPTIONS)?;
    equal(manifest, "transform", TRANSFORM)?;
    equal(manifest, "sourceBundleSha256", &aot::digest_hex(source))?;
    equal(manifest, "preludeSha256", &aot::digest_hex(preludes))?;
    equal(manifest, "imageSha256", image_digest)?;
    let entry =
        positive_integer(manifest, "entry", u32::MAX as f64).ok_or("invalid AOT entry")? as u32;
    let id = identity(field(manifest, "identity")?)?;
    Ok((entry, id))
}

const MAX_MANIFEST_BYTES: u64 = 65_536;

#[cfg(sk_devtools)]
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

fn memory_profile(
    value: Option<&OsStr>,
    default: aot::MemoryProfile,
) -> Result<aot::MemoryProfile, String> {
    match value {
        None => Ok(default),
        Some(value) => match value.to_str() {
            Some("compact") => Ok(aot::MemoryProfile::Compact),
            Some("default") => Ok(aot::MemoryProfile::Default),
            _ => Err("SK_AOT_MEMORY_PROFILE must be compact or default".into()),
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
/// The compressed image a build with `SK_DEVTOOLS_NATIVE_IMAGE_DLL` keeps in
/// a resource-only DLL beside the executable: its `SKINSPECTOR` RCDATA. The
/// DLL is mapped as data, never run, and stays mapped for the process; what
/// it holds is trusted only once it inflates to the manifest's `imageSha256`.
#[cfg(all(windows, sk_devtools))]
fn image_resource(dll: &str) -> Result<&'static [u8], String> {
    use windows::core::{w, HSTRING, PCWSTR};
    use windows::Win32::System::LibraryLoader::{
        FindResourceW, LoadLibraryExW, LoadResource, LockResource, SizeofResource,
        LOAD_LIBRARY_AS_DATAFILE, LOAD_LIBRARY_AS_IMAGE_RESOURCE,
    };
    const RT_RCDATA: PCWSTR = PCWSTR(10 as _);
    let exe = std::env::current_exe().map_err(|e| format!("native inspector: {e}"))?;
    let path = exe
        .parent()
        .ok_or("native inspector: the executable has no folder")?
        .join(dll);
    let missing = |what: &str| format!("{}: {what}", path.display());
    unsafe {
        let module = LoadLibraryExW(
            &HSTRING::from(path.as_os_str()),
            None,
            LOAD_LIBRARY_AS_DATAFILE | LOAD_LIBRARY_AS_IMAGE_RESOURCE,
        )
        .map_err(|e| missing(&e.to_string()))?;
        let resource = FindResourceW(Some(module), w!("SKINSPECTOR"), RT_RCDATA);
        if resource.is_invalid() {
            return Err(missing("no SKINSPECTOR resource"));
        }
        let global = LoadResource(Some(module), resource).map_err(|e| missing(&e.to_string()))?;
        let pointer = LockResource(global) as *const u8;
        let length = SizeofResource(Some(module), resource) as usize;
        if pointer.is_null() || length == 0 {
            return Err(missing("an empty SKINSPECTOR resource"));
        }
        // SAFETY: the module is never freed, and a data mapping is read-only.
        Ok(std::slice::from_raw_parts(pointer, length))
    }
}
#[cfg(all(not(windows), sk_devtools))]
fn image_resource(dll: &str) -> Result<&'static [u8], String> {
    Err(format!("native inspector: {dll} is a Windows resource DLL"))
}

/// Hashes the opened file once more, loads it, and records the entry [`boot`] runs.
fn select(
    mut file: File,
    manifest: &Json,
    kind: Kind,
    entry: u32,
    id: [u8; 16],
    profile: aot::MemoryProfile,
    description: &str,
) -> Result<(), String> {
    let image_digest = aot::digest_reader(&mut file).map_err(|e| format!("{description}: {e}"))?;
    equal(manifest, "imageSha256", &image_digest)?;
    use std::io::Seek;
    file.rewind().map_err(|e| e.to_string())?;
    // The manifest inside a signed executable supplies provenance. A
    // checksum on a manually selected external image is only integrity/ABI.
    unsafe { aot::load_image_file_with_profile(&file, id, profile) }?;
    drop(file);
    // Its typeof and global reads would assume that the preludes and host functions never ran.
    if !aot::admits_host_files() {
        return Err(format!("{description} was compiled without admitting the host's own code; rebuild it with this SDK's compiler"));
    }
    if let Some(settings) = aot::memory_settings() {
        eprintln!(
            "[native-aot] structure heap {} KiB; cell cage {} MiB; source view read-only {}",
            settings.structure_heap_size_kb,
            settings.marked_block_cage_size_mb,
            settings.initial_image_read_only
        );
    }
    PLAN.with(|plan| *plan.borrow_mut() = Some(Plan { entry, kind }));
    eprintln!(
        "[native-aot] selected {} image {description} (entry {entry}, memory {profile:?})",
        kind.name()
    );
    Ok(())
}

/// On the actual VM thread, before ANY JSC initialization: a failure cannot
/// switch a selected process back to JIT.
pub(crate) fn prepare(devtools: bool) -> Result<(), String> {
    if devtools {
        #[cfg(sk_devtools)]
        return prepare_inspector();
    }
    #[cfg(feature = "prod")]
    return prepare_application();
    #[allow(unreachable_code)]
    Ok(())
}

#[cfg(sk_devtools)]
fn prepare_inspector() -> Result<(), String> {
    let policy = engine_policy(std::env::var_os("SK_DEVTOOLS_ENGINE").as_deref())?;
    if policy == EnginePolicy::Jit {
        if aot::is_active() {
            return Err("cannot switch an initialized native inspector to JIT".into());
        }
        eprintln!("[native-aot] explicit JIT inspector selected before initialization");
        return Ok(());
    }
    // This explicit diagnostic override remains application-selected native
    // code. Ordinary releases select only the manifest signed into their exe.
    let external = std::env::var_os("SK_DEVTOOLS_NATIVE_IMAGE").map(PathBuf::from);
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
    let profile = memory_profile(
        std::env::var_os("SK_AOT_MEMORY_PROFILE").as_deref(),
        aot::MemoryProfile::Compact,
    )?;
    let manifest = json::parse(&text).ok_or("invalid native AOT manifest JSON")?;
    let source =
        crate::inspector::inspector_js().map_err(|e| format!("embedded inspector: {e}"))?;
    let expected = field(&manifest, "imageSha256")?;
    let (entry, id) = validate(
        &manifest,
        Kind::Inspector,
        source.as_bytes(),
        &preludes(),
        expected,
    )?;
    let (file, description) = if let Some(path) = external {
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
            cache::materialize(
                "inspector.image",
                compressed,
                expected,
                EMBEDDED_IMAGE_BYTES,
            )?,
            format!("embedded sha256:{expected}"),
        )
    };
    select(
        file,
        &manifest,
        Kind::Inspector,
        entry,
        id,
        profile,
        &description,
    )
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum AppEngine {
    Auto,
    Jit,
}
fn app_engine(value: Option<&OsStr>) -> Result<AppEngine, String> {
    match value.map(OsStr::to_str) {
        None | Some(Some("auto")) => Ok(AppEngine::Auto),
        Some(Some("jit")) => Ok(AppEngine::Jit),
        _ => Err("SK_APP_ENGINE must be auto or jit".into()),
    }
}

/// The app's image from its payload, checked against the payload's own script and this host.
fn application_image<'a>(
    payload: &'a crate::payload::AppPayload,
) -> Result<Option<(Json, &'a [u8], &'a str)>, String> {
    let Some(text) = payload.native_manifest() else {
        return Ok(None);
    };
    if text.len() as u64 > MAX_MANIFEST_BYTES {
        return Err("the native AOT manifest is too large".into());
    }
    let manifest = json::parse(text).ok_or("the native AOT manifest is not valid JSON")?;
    let compressed = payload
        .native_image()
        .ok_or("the package has a native AOT manifest but no image")?;
    let script = payload
        .script()
        .ok_or("the package has a native image but no app.prod.js to check it against")?;
    Ok(Some((manifest, compressed, script)))
}

#[cfg(feature = "prod")]
fn prepare_application() -> Result<(), String> {
    let Some(loaded) = crate::payload::get() else {
        return Ok(());
    };
    let Some((manifest, compressed, script)) = application_image(&loaded.payload)? else {
        return Ok(());
    };
    if app_engine(std::env::var_os("SK_APP_ENGINE").as_deref())? == AppEngine::Jit {
        eprintln!(
            "[native-aot] SK_APP_ENGINE=jit: running app.prod.js instead of the native image"
        );
        return Ok(());
    }
    let profile = memory_profile(
        std::env::var_os("SK_AOT_MEMORY_PROFILE").as_deref(),
        aot::MemoryProfile::Default,
    )?;
    let expected = field(&manifest, "imageSha256")?;
    let (entry, id) = validate(
        &manifest,
        Kind::Application,
        script.as_bytes(),
        &preludes(),
        expected,
    )?;
    let size = positive_integer(&manifest, "imageBytes", cache::MAX_IMAGE_BYTES as f64)
        .ok_or("invalid native image size")? as u64;
    let file = cache::materialize("app.image", compressed, expected, size)?;
    select(
        file,
        &manifest,
        Kind::Application,
        entry,
        id,
        profile,
        &format!("sha256:{expected}"),
    )
}

/// Whether [`prepare`] selected an image of `kind` in this process.
pub(crate) fn selected(kind: Kind) -> bool {
    PLAN.with(|plan| plan.borrow().as_ref().is_some_and(|p| p.kind == kind))
}

/// Runs the selected image's entry. `Ok(false)`: no image of `kind` was selected.
pub(crate) fn boot(rt: &JsRuntime, kind: Kind) -> Result<bool, String> {
    let Some(entry) = PLAN.with(|plan| {
        plan.borrow()
            .as_ref()
            .filter(|p| p.kind == kind)
            .map(|p| p.entry)
    }) else {
        return Ok(false);
    };
    let ctx = rt.ctx();
    // JS sees the realm's globalThis proxy, not JSC's raw JSGlobalObject.
    // Preserve identity for host WeakMaps and window/document event routing.
    let host = ctx
        .global_get("globalThis")
        .map_err(|error| error.to_string())?;
    ctx.global_set("__SK_AOT_HOST__", host)
        .map_err(|e| e.to_string())?;
    let started = std::time::Instant::now();
    match ctx.run_aot_module(entry) {
        Ok(aot::ModuleEvaluation::Fulfilled(_)) => {
            eprintln!(
                "[native-aot] {} native entry completed in {} ms",
                kind.name(),
                started.elapsed().as_millis()
            );
            Ok(true)
        }
        Ok(aot::ModuleEvaluation::Pending(_)) => Err(format!(
            "the {} native entry is awaiting host work; boot is not complete",
            kind.name()
        )),
        Err(Error::Exception(value)) => Err(ctx
            .to_rust_string(value)
            .unwrap_or_else(|_| "native JS exception".into())),
        Err(error) => Err(error.to_string()),
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    struct TempArtifact(std::path::PathBuf);
    impl TempArtifact {
        fn new(bytes: &[u8]) -> Self {
            use std::io::Write;
            static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
            let path = std::env::temp_dir().join(format!(
                "sk-aot-manifest-{}-{}.bin",
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

    #[cfg(sk_devtools)]
    #[test]
    fn rejects_oversized_manifest_before_parsing_or_starting_jsc() {
        let exact = TempArtifact::new(&vec![b' '; MAX_MANIFEST_BYTES as usize]);
        assert_eq!(
            read_manifest(&exact.0).unwrap().len(),
            MAX_MANIFEST_BYTES as usize
        );
        let oversized = TempArtifact::new(&vec![b' '; 2 * MAX_MANIFEST_BYTES as usize]);
        assert!(read_manifest(&oversized.0).is_err());
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
    fn valid(kind: Kind) -> Json {
        Json::obj(vec![
            ("schema", Json::s(SCHEMA)),
            ("version", Json::n(1.0)),
            ("stage", Json::s("native-image")),
            ("kind", Json::s(kind.name())),
            ("target", Json::s("x86_64-windows")),
            ("frontend", Json::s(aot::FRONTEND_REVISION)),
            ("engine", Json::s(aot::ENGINE_VERSION)),
            ("tableId", Json::s(aot::BUILD_TABLE_ID)),
            ("compileOptions", Json::s(aot::COMPILE_OPTIONS)),
            ("transform", Json::s(TRANSFORM)),
            ("sourceBundleSha256", Json::s(aot::digest_hex(b"bundle"))),
            ("preludeSha256", Json::s(aot::digest_hex(b"preludes"))),
            ("imageSha256", Json::s(aot::digest_hex(b"image"))),
            ("imageBytes", Json::n(5.0)),
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
        for kind in [Kind::Inspector, Kind::Application] {
            assert!(validate(
                &valid(kind),
                kind,
                b"bundle",
                b"preludes",
                &aot::digest_hex(b"image")
            )
            .is_ok());
            for name in [
                "kind",
                "target",
                "frontend",
                "engine",
                "tableId",
                "compileOptions",
                "transform",
                "sourceBundleSha256",
                "preludeSha256",
                "imageSha256",
            ] {
                let mut document = valid(kind);
                change(&mut document, name, Json::s("stale"));
                assert!(validate(
                    &document,
                    kind,
                    b"bundle",
                    b"preludes",
                    &aot::digest_hex(b"image")
                )
                .is_err());
            }
            assert!(validate(
                &valid(kind),
                kind,
                b"changed",
                b"preludes",
                &aot::digest_hex(b"image")
            )
            .is_err());
            assert!(validate(
                &valid(kind),
                kind,
                b"bundle",
                b"preludes",
                &aot::digest_hex(b"corrupt")
            )
            .is_err());
        }
    }
    #[test]
    fn an_inspector_image_is_not_an_application_image() {
        assert!(validate(
            &valid(Kind::Inspector),
            Kind::Application,
            b"bundle",
            b"preludes",
            &aot::digest_hex(b"image"),
        )
        .is_err());
        assert!(validate(
            &valid(Kind::Application),
            Kind::Inspector,
            b"bundle",
            b"preludes",
            &aot::digest_hex(b"image"),
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
            let mut document = valid(Kind::Inspector);
            change(&mut document, "entry", Json::n(entry));
            assert!(validate(
                &document,
                Kind::Inspector,
                b"bundle",
                b"preludes",
                &aot::digest_hex(b"image")
            )
            .is_err());
        }
    }
    #[test]
    fn host_identity_names_what_validate_requires() {
        let host = host_identity();
        let mut document = valid(Kind::Application);
        for name in [
            "target",
            "frontend",
            "engine",
            "tableId",
            "compileOptions",
            "transform",
            "preludeSha256",
        ] {
            let value = host.get(name).and_then(Json::as_str).unwrap().to_string();
            change(&mut document, name, Json::s(value));
        }
        assert!(validate(
            &document,
            Kind::Application,
            b"bundle",
            &preludes(),
            &aot::digest_hex(b"image")
        )
        .is_ok());
    }

    fn payload(entries: &[(u16, &str, &[u8])]) -> crate::payload::AppPayload {
        crate::payload::AppPayload::from_bytes(sk_payload::write_unaligned(entries)).unwrap()
    }
    #[test]
    fn the_application_image_is_found_beside_its_script_in_the_payload() {
        use sk_payload::{kind, name};
        let manifest = valid(Kind::Application).to_json();
        let full = payload(&[
            (kind::SCRIPT, name::SCRIPT, b"bundle"),
            (
                kind::NATIVE_MANIFEST,
                name::NATIVE_MANIFEST,
                manifest.as_bytes(),
            ),
            (kind::NATIVE_IMAGE, name::NATIVE_IMAGE, b"deflated"),
        ]);
        let (found, image, script) = application_image(&full).unwrap().unwrap();
        assert_eq!(field(&found, "kind").unwrap(), "application");
        assert_eq!(image, b"deflated");
        assert_eq!(script, "bundle");

        let plain = payload(&[(kind::SCRIPT, name::SCRIPT, b"bundle")]);
        assert!(application_image(&plain).unwrap().is_none());

        let no_image = payload(&[
            (kind::SCRIPT, name::SCRIPT, b"bundle"),
            (
                kind::NATIVE_MANIFEST,
                name::NATIVE_MANIFEST,
                manifest.as_bytes(),
            ),
        ]);
        assert!(application_image(&no_image).is_err());
        let no_script = payload(&[
            (
                kind::NATIVE_MANIFEST,
                name::NATIVE_MANIFEST,
                manifest.as_bytes(),
            ),
            (kind::NATIVE_IMAGE, name::NATIVE_IMAGE, b"deflated"),
        ]);
        assert!(application_image(&no_script).is_err());
        let broken = payload(&[
            (kind::SCRIPT, name::SCRIPT, b"bundle"),
            (kind::NATIVE_MANIFEST, name::NATIVE_MANIFEST, b"{"),
            (kind::NATIVE_IMAGE, name::NATIVE_IMAGE, b"deflated"),
        ]);
        assert!(application_image(&broken).is_err());
    }
    #[cfg(sk_devtools)]
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
            let mut document = valid(Kind::Inspector);
            change(&mut document, "image", Json::s(name));
            assert!(
                image_path(&document, Path::new("probe")).is_err(),
                "{name:?}"
            );
        }
        assert_eq!(
            image_path(&valid(Kind::Inspector), Path::new("probe")).unwrap(),
            Path::new("probe").join("devtools.image")
        );
    }
}
