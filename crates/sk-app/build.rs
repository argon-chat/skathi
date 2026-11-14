//! Embeds the devtools inspector into the executable.
//!
//! `apps/devtools` is bundled by `bun prod.ts` into `$OUT_DIR/devtools/app.prod.js` (plus
//! `window.json`), deflated, and embedded by `src/inspector.rs`. It is generated rather than checked
//! in so the embedded bundle can never be older than its sources; without bun the build fails with
//! the command to run. `SK_DEVTOOLS_BUNDLE=<app.prod.js>` supplies a prebuilt bundle instead, for
//! packaging jobs that build JS and Rust in separate stages.

#[path = "build_native_inspector.rs"]
mod native_inspector;

use std::path::{Path, PathBuf};
use std::process::Command;

fn main() {
    println!("cargo::rustc-check-cfg=cfg(sk_devtools)");
    println!("cargo:rerun-if-env-changed=SK_DEVTOOLS_BUNDLE");
    if std::env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("msvc") {
        suppress_release_pdb();
        no_identical_code_folding();
        thread_stack_commit();
    }
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("macos") {
        macos_link();
    }
    host_identity();
    standalone_manifest();

    // Cargo features are additive, so the opt-out is the absence of the cfg.
    let opted_out = std::env::var_os("CARGO_FEATURE_NO_DEVTOOLS").is_some();
    let legacy_opt_in = std::env::var_os("CARGO_FEATURE_DEVTOOLS").is_some();
    if opted_out && legacy_opt_in {
        panic!(
            "\n`--features devtools,no-devtools` asks for both halves of one switch.\n\
             `devtools` is a no-op (every build has them);\n\
             `no-devtools` is the opt-OUT. Drop one — almost certainly `devtools`.\n"
        );
    }
    if opted_out && std::env::var_os("CARGO_FEATURE_NATIVE_INSPECTOR").is_some() {
        panic!("native-inspector and no-devtools are contradictory");
    }
    if opted_out {
        return;
    }
    println!("cargo:rustc-cfg=sk_devtools");

    let manifest = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap());
    let repo = manifest.join("..").join("..");
    let out = PathBuf::from(std::env::var("OUT_DIR").unwrap()).join("devtools");
    std::fs::create_dir_all(&out).expect("cannot create OUT_DIR/devtools");
    let bundle = out.join("app.prod.js");
    let window_json = out.join("window.json");

    watch_tree(&repo.join("apps/devtools"));
    watch_tree(&repo.join("packages/runtime/src"));
    println!(
        "cargo:rerun-if-changed={}",
        disp(&repo.join("packages/cli/prod.ts"))
    );
    println!(
        "cargo:rerun-if-changed={}",
        disp(&repo.join("packages/cli/tailwind.ts"))
    );
    println!(
        "cargo:rerun-if-changed={}",
        disp(&repo.join("packages/cli/package.json"))
    );

    if let Some(prebuilt) = std::env::var_os("SK_DEVTOOLS_BUNDLE") {
        let prebuilt = PathBuf::from(prebuilt);
        println!("cargo:rerun-if-changed={}", disp(&prebuilt));
        let js = std::fs::read(&prebuilt).unwrap_or_else(|e| {
            panic!("SK_DEVTOOLS_BUNDLE={} cannot be read: {e}", disp(&prebuilt))
        });
        std::fs::write(&bundle, js).expect("cannot write the inspector bundle into OUT_DIR");
        let beside = prebuilt.with_file_name("window.json");
        let cfg = std::fs::read_to_string(&beside).unwrap_or_else(|_| "{}\n".into());
        std::fs::write(&window_json, cfg).expect("cannot write window.json into OUT_DIR");
        deflate_bundle(&bundle);
        native_inspector::prepare(&repo, &out);
        return;
    }

    let pkg = repo.join("packages/cli");
    let entry = repo.join("apps/devtools/src/main.ts");
    let status = Command::new("bun")
        .current_dir(&pkg)
        .arg("prod.ts")
        .arg(&entry)
        .arg(&bundle)
        .status();

    match status {
        Ok(s) if s.success() => {}
        Ok(s) => panic!(
            "{}",
            failure(
                &pkg,
                &entry,
                &bundle,
                &format!("`bun prod.ts` exited with {s}")
            )
        ),
        Err(e) => panic!(
            "{}",
            failure(
                &pkg,
                &entry,
                &bundle,
                &format!("`bun` could not be run: {e}")
            )
        ),
    }
    if !bundle.is_file() {
        panic!(
            "{}",
            failure(
                &pkg,
                &entry,
                &bundle,
                "`bun prod.ts` reported success but wrote no bundle"
            )
        );
    }
    // prod.ts writes one beside the bundle; this keeps a missing file from surfacing as an
    // `include_str!` error.
    if !window_json.is_file() {
        std::fs::write(&window_json, "{}\n").expect("cannot write a placeholder window.json");
    }
    deflate_bundle(&bundle);
    native_inspector::prepare(&repo, &out);
}

/// `<bundle>.deflate`, embedded by `src/inspector.rs`: only the inspector process evaluates the
/// bundle, so every other binary carries it at about a quarter of the size. `window.json` stays
/// plain: its window title is the marker `devtools-optout.spec.ts` looks for in the image.
fn deflate_bundle(bundle: &Path) {
    use std::io::Write;
    let js = std::fs::read(bundle).expect("cannot read the inspector bundle back from OUT_DIR");
    let mut enc = flate2::write::DeflateEncoder::new(Vec::new(), flate2::Compression::best());
    enc.write_all(&js).expect("deflate into memory cannot fail");
    let packed = enc.finish().expect("deflate into memory cannot fail");
    let mut out = bundle.as_os_str().to_owned();
    out.push(".deflate");
    std::fs::write(PathBuf::from(out), packed).expect("cannot write the deflated inspector bundle");
}

/// `/DEBUG:NONE` for the `release` profile, so a shipped build does not write a huge `sk_app.pdb`.
/// `strip = "symbols"` does not do this on windows-msvc, and RUSTFLAGS would also strip dev builds;
/// `rustc-link-arg-bins` is scoped to this package's binaries. The profile is read from `OUT_DIR`
/// because `PROFILE` says "release" for every profile that inherits it, `release-symbols` included.
/// `SK_PDB=1` opts back in.
fn suppress_release_pdb() {
    println!("cargo:rerun-if-env-changed=SK_PDB");
    let asked_for_pdb =
        std::env::var("SK_PDB").is_ok_and(|v| !v.is_empty() && v != "0" && v != "false");
    if asked_for_pdb || profile_dir().as_deref() != Some("release") {
        return;
    }
    // The last `/DEBUG` wins, and rustc already passed one.
    println!("cargo:rustc-link-arg-bins=/DEBUG:NONE");
}

/// `/OPT:NOICF` on the final link. JSC tells a constructor from a plain function by comparing native
/// function pointers (`callHostFunctionAsConstructor`), and identical-COMDAT folding made `BigInt`
/// and `Symbol` non-constructors in `--release --features prod` builds. Scoped to this package's
/// binaries; it lands after rustc's `/OPT:REF,ICF` and link.exe takes the last `/OPT`. Costs about
fn no_identical_code_folding() {
    println!("cargo:rustc-link-arg-bins=/OPT:NOICF");
}

/// `/STACK:8388608,4096`: an 8 MiB reserve (what JSC needs for deep recursion) with one page of commit.
/// `.cargo/config.toml` sets a 1 MiB commit for every workspace binary, and threads created with
/// `STACK_SIZE_PARAM_IS_A_RESERVATION` (every Rust thread, the GPU driver's workers) inherit the
/// header's commit, so each committed 1 MiB up front. Done here rather than in rustflags because
/// those rebuild every crate; this lands after them and link.exe takes the last `/STACK`, which
/// must spell out the commit. Test binaries keep the config's value.
fn thread_stack_commit() {
    println!("cargo:rustc-link-arg-bins=/STACK:8388608,4096");
}

/// - `-ObjC`: keeps Objective-C categories from static libraries; a library's own link args never
///   reach the binary's link.
/// - `-rpath /usr/lib/swift`: where frameworks that load the system Swift runtime (ScreenCaptureKit)
///   find it.
/// - `-no_deduplicate`: ld64's identical-code folding, for the same constructor-identity reason as
///   `/OPT:NOICF`.
/// - `-U` for Bun's hooks: weak references in the JSC prebuilt that redjs leaves undefined. ld64
///   wants a definition unless told it may be missing; dyld then binds them to null. The list is
///   redjs's `BUN_HOOKS`.
fn macos_link() {
    println!("cargo:rustc-link-arg-bins=-Wl,-ObjC");
    println!("cargo:rustc-link-arg-bins=-Wl,-rpath,/usr/lib/swift");
    println!("cargo:rustc-link-arg-bins=-Wl,-no_deduplicate");
    for symbol in [
        "Bun__reportUnhandledError",
        "Bun__thisThreadHasVM",
        "WTFTimer__cancel",
        "WTFTimer__create",
        "WTFTimer__deinit",
        "WTFTimer__isActive",
        "WTFTimer__secondsUntilTimer",
        "WTFTimer__update",
    ] {
        println!("cargo:rustc-link-arg-bins=-Wl,-U,_{symbol}");
    }
}

/// `SK_PKG_VERSION` (packages/cli/package.json `"version"`) and `SK_JSC_PIN`
/// (`vendor/redjs/pins.txt` `webkit.version`), reported by `--sk-host-info` and checked against a
/// payload. Emitted before the `no-devtools` early return: both variants are hosts.
fn host_identity() {
    let manifest = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap());
    let repo = manifest.join("..").join("..");
    let pkg = repo.join("packages/cli/package.json");
    let pins = repo.join("vendor/redjs/pins.txt");
    println!("cargo:rerun-if-changed={}", disp(&pkg));
    println!("cargo:rerun-if-changed={}", disp(&pins));
    let version = std::fs::read_to_string(&pkg)
        .ok()
        .and_then(|text| {
            let at = text.find("\"version\"")?;
            let rest = &text[at + "\"version\"".len()..];
            let open = rest.find('"')?;
            let rest = &rest[open + 1..];
            Some(rest[..rest.find('"')?].to_string())
        })
        .unwrap_or_else(|| "0.0.0".into());
    let pin = std::fs::read_to_string(&pins)
        .ok()
        .and_then(|text| {
            text.lines().find_map(|l| {
                let (k, v) = l.split_once('=')?;
                (k.trim() == "webkit.version").then(|| v.trim().to_string())
            })
        })
        .unwrap_or_else(|| "unknown".into());
    println!("cargo:rustc-env=SK_PKG_VERSION={version}");
    println!("cargo:rustc-env=SK_JSC_PIN={pin}");
}

fn profile_dir() -> Option<String> {
    let out = PathBuf::from(std::env::var("OUT_DIR").ok()?);
    let profile = out.ancestors().nth(3)?.file_name()?;
    Some(profile.to_string_lossy().into_owned())
}

fn failure(pkg: &Path, entry: &Path, bundle: &Path, why: &str) -> String {
    format!(
        "\n\
         ────────────────────────────────────────────────────────────────────\n\
         the embedded devtools inspector could not be built: {why}\n\
         \n\
         This build embeds apps/devtools into the executable. It is generated rather\n\
         than checked in so a shipped binary can never carry an inspector older than\n\
         its sources, which is why this is a hard error and not a warning.\n\
         \n\
         Fix it one of three ways:\n\
         \n\
           1. install bun (https://bun.sh) and re-run cargo — this is the same\n\
              toolchain `bun build.ts` already needs to produce dist/;\n\
         \n\
           2. build the bundle elsewhere and point the build at it:\n\
                cd {pkg}\n\
                bun prod.ts {entry} <somewhere>/app.prod.js\n\
                SK_DEVTOOLS_BUNDLE=<somewhere>/app.prod.js cargo build …\n\
         \n\
           3. build a host with no inspector at all:\n\
                cargo build -p sk-app --features no-devtools\n\
              (the opt-OUT. Every other build,\n\
               `--features prod` included, embeds the inspector.)\n\
         \n\
         expected output: {bundle}\n\
         ────────────────────────────────────────────────────────────────────\n",
        pkg = disp(pkg),
        entry = disp(entry),
        bundle = disp(bundle),
    )
}

/// Every source file under `dir`, as `rerun-if-changed` lines. Directories are listed too, so that
/// adding a file counts as a change (Cargo only sees a directory's own mtime).
fn watch_tree(dir: &Path) {
    println!("cargo:rerun-if-changed={}", disp(dir));
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if name == "node_modules" || name.starts_with('.') {
            continue;
        }
        if path.is_dir() {
            watch_tree(&path);
        } else {
            println!("cargo:rerun-if-changed={}", disp(&path));
        }
    }
}

fn disp(p: &Path) -> String {
    p.display().to_string().replace('\\', "/")
}

/// A library must not inject executable identity into its consumer. The generic
/// host still owns the default Windows DPI/common-controls manifest.
fn standalone_manifest() {
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }
    let root = PathBuf::from(std::env::var_os("CARGO_MANIFEST_DIR").unwrap());
    let resources = root.join("../../vendor/zed/crates/gpui/resources/windows");
    println!(
        "cargo:rerun-if-changed={}",
        resources.join("gpui.manifest.xml").display()
    );
    embed_resource::compile_for(
        resources.join("gpui.rc"),
        ["sk-app"],
        embed_resource::ParamsIncludeDirs([&resources]),
    )
    .manifest_required()
    .unwrap();
    // The examples are this crate's own executables, and gpui's imports
    // need the Common Controls 6 the manifest names: without it Windows
    // refuses to load them (STATUS_ENTRYPOINT_NOT_FOUND).
    embed_resource::compile_for_examples(
        resources.join("gpui.rc"),
        embed_resource::ParamsIncludeDirs([&resources]),
    )
    .manifest_required()
    .unwrap();
}
