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
mod container;
#[cfg(sk_devtools)]
include!(concat!(env!("OUT_DIR"), "/devtools/native_inspector.rs"));

const SCHEMA: &str = "skathi.native-aot";
const TRANSFORM: &str = "opaque-host-v1";
const APP_PRELUDES_TRANSFORM: &str = "opaque-host-preludes-v2";
const CLOSED_APP_TRANSFORM: &str = "opaque-host-closed-preludes-v3";
const TARGET: &str = env!("SK_AOT_TARGET");

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
    includes_preludes: bool,
    closed: bool,
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
    #[allow(unused_mut)]
    let mut identity = vec![
        ("target", Json::s(TARGET)),
        ("frontend", Json::s(aot::FRONTEND_REVISION)),
        ("engine", Json::s(aot::ENGINE_VERSION)),
        ("tableId", Json::s(aot::BUILD_TABLE_ID)),
        ("compileOptions", Json::s(aot::COMPILE_OPTIONS)),
        ("transform", Json::s(TRANSFORM)),
        ("closedTransform", Json::s(CLOSED_APP_TRANSFORM)),
        ("preludeSha256", Json::s(aot::digest_hex(&preludes()))),
        (
            "preludeScripts",
            Json::arr(vec![
                Json::s(crate::PRELUDE_JS),
                Json::s(crate::WINDOW_PRELUDE_JS),
            ]),
        ),
    ];
    // The container `sk build` ships beside an app with devtools, from beside this host.
    #[cfg(sk_devtools)]
    if let Some(name) = EMBEDDED_IMAGE_CONTAINER {
        identity.push(("inspectorContainer", Json::s(name)));
    }
    Json::obj(identity)
}

fn validate(
    manifest: &Json,
    kind: Kind,
    source: &[u8],
    preludes: &[u8],
    image_digest: &str,
) -> Result<(Plan, [u8; 16]), String> {
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
    let transform = field(manifest, "transform")?;
    let includes_preludes = match (transform, kind) {
        (TRANSFORM, _) => false,
        (APP_PRELUDES_TRANSFORM | CLOSED_APP_TRANSFORM, Kind::Application) => true,
        _ => return Err("unsupported native AOT transform for this image kind".into()),
    };
    equal(manifest, "sourceBundleSha256", &aot::digest_hex(source))?;
    equal(manifest, "preludeSha256", &aot::digest_hex(preludes))?;
    equal(manifest, "imageSha256", image_digest)?;
    let entry =
        positive_integer(manifest, "entry", u32::MAX as f64).ok_or("invalid AOT entry")? as u32;
    let id = identity(field(manifest, "identity")?)?;
    Ok((
        Plan {
            entry,
            kind,
            includes_preludes,
            closed: transform == CLOSED_APP_TRANSFORM,
        },
        id,
    ))
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

/// Where a selected image's bytes are. None of them can change while the process uses them:
/// files are opened denying writes, and the loader keeps a mapped module's file from being written.
enum Image {
    File(File),
    /// Sections of the running executable.
    Mapped(&'static [u8]),
    /// Sections of a container DLL the loader mapped.
    Module(&'static [u8]),
}
impl Image {
    fn digest(&mut self) -> std::io::Result<String> {
        match self {
            Image::File(file) => {
                use std::io::Seek;
                file.rewind()?;
                aot::digest_reader(file)
            }
            Image::Mapped(bytes) | Image::Module(bytes) => Ok(aot::digest_hex(bytes)),
        }
    }
    fn size(&self) -> std::io::Result<u64> {
        match self {
            Image::File(file) => Ok(file.metadata()?.len()),
            Image::Mapped(bytes) | Image::Module(bytes) => Ok(bytes.len() as u64),
        }
    }
    /// # Safety
    /// The bytes must have been checked against the trusted manifest.
    unsafe fn load(&self, id: [u8; 16], profile: aot::MemoryProfile) -> Result<(), String> {
        unsafe {
            match self {
                Image::File(file) => aot::load_image_file_with_profile(file, id, profile),
                Image::Mapped(bytes) => aot::load_mapped_image_with_profile(bytes, id, profile),
                Image::Module(bytes) => aot::load_module_image_with_profile(bytes, id, profile),
            }
        }
    }
}

/// Hashes the image where it lies; the time it took, when it is the one the manifest names.
fn verify(
    image: &mut Image,
    manifest: &Json,
    description: &str,
) -> Result<std::time::Duration, String> {
    let started = std::time::Instant::now();
    let image_digest = image.digest().map_err(|e| format!("{description}: {e}"))?;
    if field(manifest, "imageSha256")? != image_digest {
        return Err(format!(
            "the native image {description} differs from the one this app was built with: it was \
             changed or damaged after the build"
        ));
    }
    Ok(started.elapsed())
}

/// Loads a [`verify`]d image where it lies, and records the entry [`boot`] runs.
fn select(
    image: Image,
    hashed: std::time::Duration,
    selected: Plan,
    id: [u8; 16],
    profile: aot::MemoryProfile,
    description: &str,
) -> Result<(), String> {
    // The manifest inside a signed executable supplies provenance. A
    // checksum on a manually selected external image is only integrity/ABI.
    unsafe { image.load(id, profile) }?;
    drop(image);
    if selected.closed && !aot::has_closed_library() {
        return Err(format!(
            "{description} requires a closed image with a compiled library"
        ));
    }
    if !selected.closed && !aot::admits_host_files() {
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
    eprintln!(
        "[native-aot] selected {} image {description} (entry {}, memory {profile:?}, sha256 checked in {:.1} ms)",
        selected.kind.name(),
        selected.entry,
        hashed.as_secs_f64() * 1000.0
    );
    PLAN.with(|plan| *plan.borrow_mut() = Some(selected));
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
pub(crate) fn inspector_sidecar() -> Option<PathBuf> {
    Some(container::beside_executable_image()?.join(EMBEDDED_IMAGE_CONTAINER?))
}

#[cfg(sk_devtools)]
fn prepare_inspector() -> Result<(), String> {
    let policy = engine_policy(std::env::var_os("SK_DEVTOOLS_ENGINE").as_deref())?;
    #[cfg(feature = "no-jit")]
    {
        if policy == EnginePolicy::Jit {
            return Err(
                "this host has no JIT; unset SK_DEVTOOLS_ENGINE to use the native inspector".into(),
            );
        }
        if !inspector_sidecar().is_some_and(|file| file.is_file()) {
            return Err("this host needs its native inspector image beside the executable".into());
        }
    }
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
    let (plan, id) = validate(
        &manifest,
        Kind::Inspector,
        source.as_bytes(),
        &preludes(),
        expected,
    )?;
    let (mut image, description) = if let Some(path) = external {
        let image = image_path(&manifest, path.parent().unwrap_or_else(|| Path::new(".")))?;
        let file = open_image(&image).map_err(|e| format!("{}: {e}", image.display()))?;
        let size = file.metadata().map_err(|e| e.to_string())?.len();
        if size == 0 || size > cache::MAX_IMAGE_BYTES {
            return Err("native AOT image exceeds the supported size".into());
        }
        (Image::File(file), image.display().to_string())
    } else if let Some(bytes) = container::in_executable(container::INSPECTOR_SECTION) {
        (Image::Mapped(bytes), "in this executable".to_string())
    } else if let Some(dll) = EMBEDDED_IMAGE_DLL {
        (
            Image::File(cache::materialize(
                "inspector.image",
                image_resource(dll)?,
                expected,
                EMBEDDED_IMAGE_BYTES,
            )?),
            format!("{dll} sha256:{expected}"),
        )
    } else if let Some(path) = inspector_sidecar() {
        match container::load(&path, container::INSPECTOR_SECTION) {
            Ok(bytes) => (Image::Module(bytes), path.display().to_string()),
            Err(error)
                if !cfg!(feature = "no-jit")
                    && error.kind() == std::io::ErrorKind::NotFound
                    && policy == EnginePolicy::Auto =>
            {
                eprintln!(
                    "[native-aot] no {} beside the executable: the inspector runs under the JIT",
                    path.display()
                );
                return Ok(());
            }
            Err(error) => return Err(format!("{}: {error}", path.display())),
        }
    } else {
        (
            Image::File(cache::materialize(
                "inspector.image",
                EMBEDDED_IMAGE_DEFLATE,
                expected,
                EMBEDDED_IMAGE_BYTES,
            )?),
            format!("embedded sha256:{expected}"),
        )
    };
    let hashed = verify(&mut image, &manifest, &description)?;
    select(image, hashed, plan, id, profile, &description)
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

/// The manifest of the app's image from its payload, and the script it is checked against.
fn application_image(payload: &crate::payload::AppPayload) -> Result<Option<(Json, &str)>, String> {
    let Some(text) = payload.native_manifest() else {
        return Ok(None);
    };
    if text.len() as u64 > MAX_MANIFEST_BYTES {
        return Err("the native AOT manifest is too large".into());
    }
    let manifest = json::parse(text).ok_or("the native AOT manifest is not valid JSON")?;
    let script = payload
        .script()
        .ok_or("the package has a native image but no app.prod.js to check it against")?;
    Ok(Some((manifest, script)))
}

/// Where the manifest says `sk build` put the image: a container beside the payload, or
/// sections of this executable.
fn application_place(manifest: &Json, beside: Option<&Path>) -> Result<(Image, String), String> {
    match field(manifest, "place")? {
        "executable" => container::in_executable(container::APP_SECTION)
            .map(|bytes| (Image::Mapped(bytes), "in this executable".to_string()))
            .ok_or_else(|| "this executable carries no native image sections".to_string()),
        "beside" => {
            let name = field(manifest, "image")?;
            if name.is_empty() || name.contains(['/', '\\', ':', '\0']) || name.starts_with('.') {
                return Err(format!(
                    "the native image container {name:?} is not a plain file name"
                ));
            }
            let dir = beside.ok_or("the native image's folder is unknown")?;
            let path = image_directory(dir).join(name);
            let bytes =
                container::load(&path, container::APP_SECTION).map_err(|error| {
                    match error.kind() {
                        std::io::ErrorKind::NotFound => format!("{} is missing", path.display()),
                        _ => format!("{}: {error}", path.display()),
                    }
                })?;
            Ok((Image::Module(bytes), path.display().to_string()))
        }
        other => Err(format!("unsupported native image place {other:?}")),
    }
}

/// The aot-nojit workload's engine has no JIT: it runs an app only from its image.
#[cfg(feature = "no-jit")]
const WITHOUT_JIT: &str =
    "this exe is built on the aot-nojit workload, whose engine has no JIT, so it runs only \
    the app's native image";

#[cfg(feature = "prod")]
fn prepare_application() -> Result<(), String> {
    let Some(loaded) = crate::payload::get() else {
        #[cfg(feature = "no-interpreter")]
        return Err("this host needs an app payload with native preludes; publish the app with sk build --aot".into());
        #[cfg(not(feature = "no-interpreter"))]
        return Ok(());
    };
    let engine = app_engine(std::env::var_os("SK_APP_ENGINE").as_deref());
    #[cfg(feature = "no-jit")]
    if engine.clone()? == AppEngine::Jit {
        return Err(format!(
            "SK_APP_ENGINE=jit asks for the JIT, but {WITHOUT_JIT}. Unset SK_APP_ENGINE, or publish \
             the app without \"publishJit\": false (the no-devtools workload) to run it under the JIT."
        ));
    }
    let image = application_image(&loaded.payload).map_err(cannot_run)?;
    let Some((manifest, script)) = image else {
        #[cfg(feature = "no-jit")]
        return Err(format!(
            "the app has no native image, and {WITHOUT_JIT}. Publish it with --aot (\"aot\": true), \
             or without \"publishJit\": false."
        ));
        #[cfg(not(feature = "no-jit"))]
        return Ok(());
    };
    if engine.map_err(cannot_run)? == AppEngine::Jit {
        eprintln!(
            "[native-aot] SK_APP_ENGINE=jit: running app.prod.js instead of the native image"
        );
        return Ok(());
    }
    prepare_image(&manifest, script, loaded.dir.as_deref())
}

fn cannot_run(error: String) -> String {
    format!(
        "this app's native image cannot run on this host ({error}). \
         Rebuild the app with `sk build --aot` from the sk that matches this host."
    )
}

fn image_directory(payload_dir: &Path) -> std::borrow::Cow<'_, Path> {
    #[cfg(target_os = "macos")]
    if payload_dir.ends_with("Contents/Resources") {
        if let Some(contents) = payload_dir.parent() {
            if contents.parent().and_then(Path::extension) == Some(OsStr::new("app")) {
                return contents.join("Frameworks").into();
            }
        }
    }
    payload_dir.into()
}

/// The image the manifest names is missing or not the one built with the app.
fn damaged(error: String) -> String {
    format!(
        "this app cannot start: {error}. Reinstall the app with every file it ships with, \
         or publish it again."
    )
}

#[cfg(feature = "prod")]
fn prepare_image(manifest: &Json, script: &str, beside: Option<&Path>) -> Result<(), String> {
    let profile = memory_profile(
        std::env::var_os("SK_AOT_MEMORY_PROFILE").as_deref(),
        aot::MemoryProfile::Default,
    )
    .map_err(cannot_run)?;
    let expected = field(manifest, "imageSha256").map_err(cannot_run)?;
    let (plan, id) = validate(
        manifest,
        Kind::Application,
        script.as_bytes(),
        &preludes(),
        expected,
    )
    .map_err(cannot_run)?;
    if cfg!(feature = "no-interpreter") && !plan.includes_preludes {
        return Err(cannot_run("the native image requires the host to evaluate its preludes from source, but this host has no interpreter".into()));
    }
    let size = positive_integer(manifest, "imageBytes", cache::MAX_IMAGE_BYTES as f64)
        .ok_or_else(|| cannot_run("invalid native image size".into()))? as u64;
    let (mut image, description) = application_place(manifest, beside).map_err(damaged)?;
    if image
        .size()
        .map_err(|e| damaged(format!("{description}: {e}")))?
        != size
    {
        return Err(damaged(format!(
            "the native image {description} differs from the one this app was built with: it was \
             changed or damaged after the build"
        )));
    }
    let hashed = verify(&mut image, manifest, &description).map_err(damaged)?;
    select(image, hashed, plan, id, profile, &description).map_err(cannot_run)
}

/// Whether [`prepare`] selected an image of `kind` in this process.
pub(crate) fn selected(kind: Kind) -> bool {
    PLAN.with(|plan| plan.borrow().as_ref().is_some_and(|p| p.kind == kind))
}

#[cfg(not(feature = "no-interpreter"))]
pub(crate) fn includes_preludes() -> bool {
    PLAN.with(|plan| plan.borrow().as_ref().is_some_and(|p| p.includes_preludes))
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
            ("target", Json::s(TARGET)),
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
    fn only_the_application_transforms_can_include_the_host_preludes() {
        for (kind, transform, expected) in [
            (Kind::Application, TRANSFORM, Some(false)),
            (Kind::Inspector, TRANSFORM, Some(false)),
            (Kind::Application, APP_PRELUDES_TRANSFORM, Some(true)),
            (Kind::Inspector, APP_PRELUDES_TRANSFORM, None),
            (Kind::Application, CLOSED_APP_TRANSFORM, Some(true)),
            (Kind::Inspector, CLOSED_APP_TRANSFORM, None),
        ] {
            let mut document = valid(kind);
            change(&mut document, "transform", Json::s(transform));
            let result = validate(
                &document,
                kind,
                b"bundle",
                b"preludes",
                &aot::digest_hex(b"image"),
            );
            let result = result
                .ok()
                .map(|(plan, _)| (plan.includes_preludes, plan.closed));
            assert_eq!(
                result,
                expected.map(|preludes| (preludes, transform == CLOSED_APP_TRANSFORM))
            );
        }
    }
    #[cfg(feature = "no-interpreter")]
    #[test]
    fn legacy_app_image_is_refused_before_loading_a_file_or_starting_jsc() {
        let mut document = valid(Kind::Application);
        change(
            &mut document,
            "preludeSha256",
            Json::s(aot::digest_hex(&preludes())),
        );
        assert!(prepare_image(&document, "bundle", None)
            .unwrap_err()
            .contains("this host has no interpreter"));
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
        assert_eq!(
            host.get("preludeScripts"),
            Some(&Json::arr(vec![
                Json::s(crate::PRELUDE_JS),
                Json::s(crate::WINDOW_PRELUDE_JS)
            ]))
        );
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
    fn the_application_manifest_is_found_beside_its_script_in_the_payload() {
        use sk_payload::{kind, name};
        let manifest = valid(Kind::Application).to_json();
        let full = payload(&[
            (kind::SCRIPT, name::SCRIPT, b"bundle"),
            (
                kind::NATIVE_MANIFEST,
                name::NATIVE_MANIFEST,
                manifest.as_bytes(),
            ),
        ]);
        let (found, script) = application_image(&full).unwrap().unwrap();
        assert_eq!(field(&found, "kind").unwrap(), "application");
        assert_eq!(script, "bundle");

        let plain = payload(&[(kind::SCRIPT, name::SCRIPT, b"bundle")]);
        assert!(application_image(&plain).unwrap().is_none());

        let no_script = payload(&[(
            kind::NATIVE_MANIFEST,
            name::NATIVE_MANIFEST,
            manifest.as_bytes(),
        )]);
        assert!(application_image(&no_script).is_err());
        let broken = payload(&[
            (kind::SCRIPT, name::SCRIPT, b"bundle"),
            (kind::NATIVE_MANIFEST, name::NATIVE_MANIFEST, b"{"),
        ]);
        assert!(application_image(&broken).is_err());
    }
    #[test]
    fn an_application_container_is_a_plain_file_beside_the_payload() {
        let mut document = valid(Kind::Application);
        if let Json::Obj(entries) = &mut document {
            entries.push(("place".into(), Json::s("beside")));
        }
        for name in [
            "",
            "../app.image.dll",
            r"x\app.image.dll",
            "C:app.image.dll",
            ".hidden",
        ] {
            change(&mut document, "image", Json::s(name));
            let error = application_place(&document, Some(Path::new("probe")))
                .err()
                .unwrap();
            assert!(error.contains("not a plain file name"), "{name:?}: {error}");
        }
        change(&mut document, "image", Json::s("app.image.dll"));
        let error = application_place(&document, Some(Path::new("probe-missing")))
            .err()
            .unwrap();
        assert!(error.contains("is missing"), "{error}");
        change(&mut document, "place", Json::s("cache"));
        assert!(application_place(&document, None).is_err());
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
