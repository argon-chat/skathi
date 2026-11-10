//! Builds the devtools inspector INTO the executable.
//!
//! ```text
//!   apps/devtools/**  ──bun prod.ts──▶  $OUT_DIR/devtools/app.prod.js
//!                                                │ deflate (`deflate_bundle`)
//!                                       $OUT_DIR/devtools/app.prod.js.deflate
//!                                       $OUT_DIR/devtools/window.json
//!                                                │ include_bytes! / include_str!
//!                                                ▼
//!                                       crates/vn-app/src/inspector.rs
//! ```
//!
//! ## Why a build script and not a checked-in bundle
//!
//! Two generated JS artifacts are already checked in here — `src/prelude.js` and
//! `src/window.prelude.js` — for a reason stated in `lib.rs`: `cargo build`
//! must not need a JS toolchain, and an `include_str!` of a missing file is a
//! terrible place to discover bun is not installed. Both are a few hundred
//! lines of hand-reviewable code that changes when somebody edits its `.ts`
//! twin.
//!
//! The inspector is a different animal: ~130 KB of minified Vue that changes
//! whenever any of ~20 components does. Checking it in would put a 130 KB
//! binary diff in every devtools commit, make every rebase a conflict, and —
//! the part that actually matters — make "the bundle in git is older than the
//! sources beside it" an ordinary, invisible state. A debugger that silently
//! shows you last week's panels is worse than one that does not build.
//!
//! So it is generated, and the toolchain objection is answered where it is
//! raised rather than by shipping a stale artifact: this script fails the build
//! with the exact command to run, and names the flag that builds a host with no
//! inspector at all. `bun` is already a hard requirement for a dev host to have
//! anything to *run* (`dist/` comes from `bun build.ts`), so this constrains no
//! build that was not already constrained.
//!
//! `VN_DEVTOOLS_BUNDLE=<path to app.prod.js>` bypasses the bun invocation with
//! a prebuilt bundle, for a packaging job that builds JS and Rust in separate
//! stages. It is explicit, it is not a fallback, and nothing selects it
//! automatically.
//!
//! ## The cfg this emits
//!
//! `vn_devtools` — "this build has the devtools protocol, the embedded
//! inspector and the F12 key". **True for every build**, including
//! `--features prod`, since m22: a shipped app carries its own devtools the way
//! a browser does, and the app turns them off at RUNTIME instead
//! (`appDevtools.disable()`, or `"devtools": false` in `vn.config.json` — see
//! docs/DEVTOOLS.md "Shape" and docs/ARCHITECTURE.md §11 decision 12).
//!
//! The one build without them is `--features no-devtools`, the opt-OUT for an
//! app that wants zero devtools code in its image: no listener, no host fns, no
//! bundle, no F12, no `rules` text capture, and `appDevtools` inert. It is the
//! exception, it is never selected automatically, and nothing in this repo
//! builds it except the size/marker check in
//! `research/reports/m22/devtools-release.md`.
//!
//! `--features devtools` (the m21 opt-IN spelling) is still accepted and now
//! means nothing at all — every build has them. It is kept only so command
//! lines and scripts written before the inversion keep producing a working
//! binary; combining it with `no-devtools` is a contradiction and fails here.

#[path = "build_native_inspector.rs"]
mod native_inspector;

use std::path::{Path, PathBuf};
use std::process::Command;

fn main() {
    println!("cargo::rustc-check-cfg=cfg(vn_devtools)");
    println!("cargo:rerun-if-env-changed=VN_DEVTOOLS_BUNDLE");
    // link.exe switches: everything below them is spelled for MSVC.
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

    // The opt-OUT. Cargo features are additive across a graph, so this is the
    // one shape a "remove code" flag can take: the absence of the cfg is the
    // default, and only this flag can produce it.
    let opted_out = std::env::var_os("CARGO_FEATURE_NO_DEVTOOLS").is_some();
    let legacy_opt_in = std::env::var_os("CARGO_FEATURE_DEVTOOLS").is_some();
    if opted_out && legacy_opt_in {
        panic!(
            "\n`--features devtools,no-devtools` asks for both halves of one switch.\n\
             `devtools` is the pre-m22 opt-IN and is now a no-op (every build has them);\n\
             `no-devtools` is the opt-OUT. Drop one — almost certainly `devtools`.\n"
        );
    }
    if opted_out && std::env::var_os("CARGO_FEATURE_NATIVE_INSPECTOR").is_some() {
        panic!("native-inspector and no-devtools are contradictory");
    }
    if opted_out {
        return;
    }
    println!("cargo:rustc-cfg=vn_devtools");

    let manifest = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap());
    let repo = manifest.join("..").join("..");
    let out = PathBuf::from(std::env::var("OUT_DIR").unwrap()).join("devtools");
    std::fs::create_dir_all(&out).expect("cannot create OUT_DIR/devtools");
    let bundle = out.join("app.prod.js");
    let window_json = out.join("window.json");

    // Everything the bundle is made of. A miss here is the failure mode this
    // whole file exists to prevent, so the walk is deliberately generous: the
    // inspector's own sources AND the framework runtime it is compiled against.
    watch_tree(&repo.join("apps/devtools"));
    watch_tree(&repo.join("packages/vue-native/src"));
    println!(
        "cargo:rerun-if-changed={}",
        disp(&repo.join("packages/vue-native/prod.ts"))
    );
    println!(
        "cargo:rerun-if-changed={}",
        disp(&repo.join("packages/vue-native/tailwind.ts"))
    );
    println!(
        "cargo:rerun-if-changed={}",
        disp(&repo.join("packages/vue-native/package.json"))
    );

    if let Some(prebuilt) = std::env::var_os("VN_DEVTOOLS_BUNDLE") {
        let prebuilt = PathBuf::from(prebuilt);
        println!("cargo:rerun-if-changed={}", disp(&prebuilt));
        let js = std::fs::read(&prebuilt).unwrap_or_else(|e| {
            panic!("VN_DEVTOOLS_BUNDLE={} cannot be read: {e}", disp(&prebuilt))
        });
        std::fs::write(&bundle, js).expect("cannot write the inspector bundle into OUT_DIR");
        let beside = prebuilt.with_file_name("window.json");
        let cfg = std::fs::read_to_string(&beside).unwrap_or_else(|_| "{}\n".into());
        std::fs::write(&window_json, cfg).expect("cannot write window.json into OUT_DIR");
        deflate_bundle(&bundle);
        native_inspector::prepare(&repo, &out);
        return;
    }

    let pkg = repo.join("packages/vue-native");
    let entry = repo.join("apps/devtools/src/main.ts");
    // `bun` on PATH is `bun.exe` on Windows and a shim on some installs; let
    // the OS resolve it rather than guessing an extension.
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
    // prod.ts always writes one beside the bundle; belt and braces, because an
    // `include_str!` of a missing file is the error message this file exists to
    // never emit.
    if !window_json.is_file() {
        std::fs::write(&window_json, "{}\n").expect("cannot write a placeholder window.json");
    }
    deflate_bundle(&bundle);
    native_inspector::prepare(&repo, &out);
}

/// `<bundle>.deflate`: the inspector bundle as raw deflate, which is what
/// `src/inspector.rs` embeds.
///
/// Only the inspector process ever evaluates the bundle (F12 re-spawns this
/// exe with `--devtools`), yet every vue-native binary carried it as ~340 KB
/// of plain JS. Deflated it is about a quarter of that, and inflating it once
/// when an inspector boots costs a few milliseconds. `window.json` stays plain:
/// it is 1 KB, and its window title is the marker
/// `packages/vn-test/specs/devtools-optout.spec.ts` looks for in the image.
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

/// `/DEBUG:NONE` for the `release` profile, so a shipped build stops writing a
/// 326 MB `vn_app.pdb` nobody ships.
///
/// This lives here because there is nowhere else it CAN live. `[profile.release]
/// strip = "symbols"` is the setting that looks like it does this and does not:
/// `-C strip` is a Unix `strip(1)` operation, a windows-msvc image has no symbol
/// table to remove (symbols live in the PDB), and Cargo does not translate it
/// into a linker switch — measured, byte-identical exe and a full-size PDB
/// (research/reports/m22/devtools-release.md §5). `RUSTFLAGS` would work and is
/// global, which would take the PDB away from dev builds too, and Cargo has no
/// stable per-profile `rustflags`. `cargo:rustc-link-arg-bins` is scoped to
/// exactly this package's binaries, which is exactly the scope wanted.
///
/// The profile is read out of `OUT_DIR` (`…/<profile-dir>/build/<pkg>/out`)
/// rather than out of `PROFILE`, because `PROFILE` says "release" for every
/// profile that inherits it — including `release-symbols`, whose entire purpose
/// is the opposite of this.
///
/// `VN_PDB=1` opts back in for a `release` build, for the case where the binary
/// already exists and rebuilding it under another profile would change the
/// addresses in the crash dump you are holding.
fn suppress_release_pdb() {
    println!("cargo:rerun-if-env-changed=VN_PDB");
    let asked_for_pdb =
        std::env::var("VN_PDB").is_ok_and(|v| !v.is_empty() && v != "0" && v != "false");
    if asked_for_pdb || profile_dir().as_deref() != Some("release") {
        return;
    }
    // Last one wins for `/DEBUG`, and rustc has already put its own on the
    // command line — so this is an override rather than an omission.
    println!("cargo:rustc-link-arg-bins=/DEBUG:NONE");
}

/// `/OPT:NOICF` on the final link: no identical-COMDAT folding in the image
/// that carries JavaScriptCore.
///
/// rustc passes `/OPT:REF,ICF` to link.exe for every optimized MSVC build
/// (debug gets `/OPT:REF,NOICF`). JSC tells a constructor from a plain function
/// by comparing native function POINTERS against `callHostFunctionAsConstructor`
/// (`JSFunctionInlines.h`), so when the linker folds a real constructor's
/// `[[Construct]]` stub into that function because their bytes match, the
/// constructor stops being one. MEASURED on the d6375c72 engine, same source,
/// only this line differing (research/reports/m24/jsc-bump.md §ICF): a
/// `--release --features prod` exe reported `BigInt` and `Symbol` as
/// non-constructors and `class X extends BigInt {}` threw "The superclass is
/// not a constructor" — a silent divergence from every browser. Bun hit the same
/// fold; WebKit's own build links `/OPT:NOICF`.
///
/// Scope: `-bins`, i.e. only vn-app's executable — nothing else's fingerprint
/// moves. It lands after rustc's own `/OPT:REF,ICF` and link.exe takes the last
/// `/OPT` it sees (the probe above is how that was checked, not assumed). redjs
/// says the same for the images IT links (its build.rs).
///
/// The price is real: ICF had been sharing a lot of identical (mostly
/// monomorphized Rust) code, and the `--release --features prod` exe goes from
/// 100 529 664 to 108 826 112 bytes (+8.3 MB). lld-link with `/OPT:SAFEICF`
/// (folds only functions whose address is never taken — Bun's setting) measured
/// correct and 104 149 504 bytes; switching linkers is an open decision, not
/// something this line does.
fn no_identical_code_folding() {
    println!("cargo:rustc-link-arg-bins=/OPT:NOICF");
}

/// `/STACK:8388608,4096`: an 8 MiB stack RESERVE, one page of COMMIT, in the
/// executable's PE header.
///
/// `.cargo/config.toml` sets `/STACK:8388608,1048576` for every binary of the
/// workspace, and the reserve half of that is what JSC needs (~45k JS frames
/// before `RangeError` instead of ~8k). The commit half had a cost nobody had
/// asked for: the header's `SizeOfStackCommit` is not a main-thread setting. A
/// thread created with `STACK_SIZE_PARAM_IS_A_RESERVATION` — every Rust
/// `std::thread`, and the GPU driver's workers — gets the header's commit, so
/// each one committed 1 MiB up front. Measured on `apps/hello`
/// (research/reports/m26/binary-size-1.md §5): 122 threads, 103 of them
/// NVIDIA driver workers, 135 MB of stack commit with ~8 MB of it ever touched.
/// One page of commit took that to 8 MB, and the process's commit charge from
/// 374 to 244 MB, with the same private working set. A stack still grows on
/// demand through its guard page, and the reserve (what JSC and stacker read
/// as the stack's size) is unchanged — deep recursion, warmed-JIT recursion,
/// 32 KB JIT frames and a 500k-argument `apply` all hit the same limits.
///
/// Why here and not in `.cargo/config.toml`: rustflags are part of every
/// crate's fingerprint, so editing them rebuilds every crate of every profile;
/// a build-script link arg relinks this one binary. It also lands AFTER the
/// rustflags on link.exe's command line, and link.exe takes the last `/STACK`
/// (both checked, not assumed). The commit must be spelled out: a later
/// `/STACK:<reserve>` with no commit keeps the earlier commit. Test binaries
/// keep the config's 1 MiB commit, which costs them nothing that matters.
fn thread_stack_commit() {
    println!("cargo:rustc-link-arg-bins=/STACK:8388608,4096");
}

/// The macOS counterparts of the link.exe switches above, for vn-app's
/// executable.
///
/// - `-ObjC`: libwebrtc's Objective-C categories (vn-voice) are only kept when
///   the final link asks for them. webrtc-sys prints the flag itself, but a
///   library's link args never reach the binary's link.
/// - `-rpath /usr/lib/swift`: frameworks that load the system Swift runtime
///   (ScreenCaptureKit, through libwebrtc) find it there, as Zed's own build
///   sets it.
/// - `-no_deduplicate`: ld64's identical-code folding, which would do to JSC's
///   constructor identity what `/OPT:NOICF` prevents on Windows (see
///   `no_identical_code_folding`). Not yet measured on a Mac: check that
///   `class X extends BigInt {}` still throws only where a browser does.
///
/// - `-U` for Bun's hooks: weak references in the JSC prebuilt that redjs
///   leaves undefined, so JSC takes its generic RunLoop and error paths. ld64
///   wants a definition even for a weak reference unless told it may be
///   missing; dyld then binds it to null. The list is redjs's `BUN_HOOKS`
///   (its build.rs passes the same flags for the images it links itself).
///
/// The main thread's 8 MiB stack is already macOS's default, so no
/// `-stack_size` is needed for JSC's headroom.
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

/// What `vn-app --vn-host-info` reports and a payload is checked against
/// (src/payload.rs, docs/PACKAGING.md §host):
///
/// * `VN_PKG_VERSION` — `packages/vue-native/package.json` `"version"`. The
///   renderer half of the contract: a payload records the version of the
///   vue-native it was bundled against, and a host built from a different one
///   says so instead of failing in some unrelated way.
/// * `VN_JSC_PIN` — `vendor/redjs/pins.txt` `webkit.version`, the engine.
///
/// Emitted before the `no-devtools` early return: both variants are hosts.
fn host_identity() {
    let manifest = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap());
    let repo = manifest.join("..").join("..");
    let pkg = repo.join("packages/vue-native/package.json");
    let pins = repo.join("vendor/redjs/pins.txt");
    println!("cargo:rerun-if-changed={}", disp(&pkg));
    println!("cargo:rerun-if-changed={}", disp(&pins));
    let version = std::fs::read_to_string(&pkg)
        .ok()
        .and_then(|text| {
            // The first `"version": "…"` — package.json's own, which precedes
            // every dependency map by convention and in this file.
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
    println!("cargo:rustc-env=VN_PKG_VERSION={version}");
    println!("cargo:rustc-env=VN_JSC_PIN={pin}");
}

/// The name of the profile directory this build is writing into, e.g.
/// `release`, `debug`, `release-symbols`.
fn profile_dir() -> Option<String> {
    // OUT_DIR = <target>/<triple>/<profile>/build/<pkg>-<hash>/out
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
         This build embeds apps/devtools into the executable (docs/DEVTOOLS-UI.md\n\
         v2 §2). It is generated rather than checked in so a shipped binary can\n\
         never carry an inspector older than its sources — which is why this is\n\
         a hard error and not a warning.\n\
         \n\
         Fix it one of three ways:\n\
         \n\
           1. install bun (https://bun.sh) and re-run cargo — this is the same\n\
              toolchain `bun build.ts` already needs to produce dist/;\n\
         \n\
           2. build the bundle elsewhere and point the build at it:\n\
                cd {pkg}\n\
                bun prod.ts {entry} <somewhere>/app.prod.js\n\
                VN_DEVTOOLS_BUNDLE=<somewhere>/app.prod.js cargo build …\n\
         \n\
           3. build a host with no inspector at all:\n\
                cargo build -p vn-app --features no-devtools\n\
              (the opt-OUT — docs/DEVTOOLS.md \"Shape\". Every other build,\n\
               `--features prod` included, embeds the inspector.)\n\
         \n\
         expected output: {bundle}\n\
         ────────────────────────────────────────────────────────────────────\n",
        pkg = disp(pkg),
        entry = disp(entry),
        bundle = disp(bundle),
    )
}

/// Every source file under `dir`, as `rerun-if-changed` lines.
///
/// Cargo re-runs a build script when a *directory* mtime moves too, but only
/// for the directory itself — a change two levels down would not be seen. So
/// the walk is explicit, and it lists directories as well so that ADDING a file
/// is also a change.
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
        ["vn-app"],
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
