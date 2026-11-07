//! Opt-in inspector experiment. A selected image is trusted executable code.
//! Its compiler, host table, preludes and embedded Vue bundle must all match.
use crate::json::{self, Json};
use std::{
    cell::RefCell,
    path::{Path, PathBuf},
};
use vn_runtime::{
    redjs::{aot, api::Error},
    JsRuntime,
};

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
    bytes: &[u8],
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
    equal(manifest, "imageSha256", &aot::digest_hex(bytes))?;
    let entry = manifest
        .get("entry")
        .and_then(Json::as_f64)
        .filter(|n| n.is_finite() && *n >= 1.0 && *n <= u32::MAX as f64 && n.fract() == 0.0)
        .ok_or("invalid AOT entry")? as u32;
    let id = identity(field(manifest, "identity")?)?;
    Ok((entry, id))
}

/// On the actual VM thread, before ANY JSC initialization. Failure cannot
/// switch this process back to JIT: a caller must stop and launch a clean one.
pub(crate) fn prepare(devtools: bool) -> Result<(), String> {
    // A parent client passes this selector through to its F12 child; it stays JIT.
    if !devtools {
        return Ok(());
    }
    let Some(path) = std::env::var_os("VN_DEVTOOLS_NATIVE_IMAGE") else {
        return Ok(());
    };
    let path = std::fs::canonicalize(PathBuf::from(path))
        .map_err(|e| format!("native AOT manifest path: {e}"))?;
    let text = std::fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))?;
    if text.len() > 65536 {
        return Err("AOT manifest is too large".into());
    }
    let manifest = json::parse(&text).ok_or("invalid native AOT manifest JSON")?;
    let source =
        crate::inspector::inspector_js().map_err(|e| format!("embedded inspector: {e}"))?;
    let mut preludes = crate::PRELUDE_JS.as_bytes().to_vec();
    preludes.push(0);
    preludes.extend_from_slice(crate::WINDOW_PRELUDE_JS.as_bytes());
    let image = image_path(&manifest, path.parent().unwrap_or_else(|| Path::new(".")))?;
    let bytes = std::fs::read(&image).map_err(|e| format!("{}: {e}", image.display()))?;
    let (entry, id) = validate(&manifest, source.as_bytes(), &preludes, &bytes)?;
    // All inputs are application-selected trusted build artifacts. The manifest
    // is a compatibility/integrity check, not an authenticity signature.
    unsafe { aot::load_image(&image, id) }?;
    PLAN.with(|plan| *plan.borrow_mut() = Some(Plan { entry }));
    eprintln!(
        "[native-aot] selected inspector image {} (entry {entry})",
        image.display()
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
    ctx.global_set("__VN_AOT_HOST__", ctx.global())
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
    use super::*;
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
        assert!(validate(&valid(), b"bundle", b"preludes", b"image").is_ok());
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
            assert!(validate(&document, b"bundle", b"preludes", b"image")
                .unwrap_err()
                .contains(name));
        }
        assert!(validate(&valid(), b"changed", b"preludes", b"image").is_err());
        assert!(validate(&valid(), b"bundle", b"preludes", b"corrupt").is_err());
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
            assert!(validate(&document, b"bundle", b"preludes", b"image").is_err());
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
