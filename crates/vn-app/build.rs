//! Builds the devtools inspector INTO the executable.
//!
//! ```text
//!   apps/devtools/**  ──bun prod.ts──▶  $OUT_DIR/devtools/app.prod.js
//!                                       $OUT_DIR/devtools/window.json
//!                                                │ include_str!
//!                                                ▼
//!                                       crates/vn-app/src/inspector.rs
//! ```
//!
//! ## Why a build script and not a checked-in bundle
//!
//! Two generated JS artifacts are already checked in here — `src/prelude.js` and
//! `src/window.prelude.js` — for a reason stated in `main.rs`: `cargo build`
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
//! anything to *run* (`dist/` comes from `bun build.ts`), so the only build
//! this newly constrains is `--features prod,devtools`, which is opt-in.
//!
//! `VN_DEVTOOLS_BUNDLE=<path to app.prod.js>` bypasses the bun invocation with
//! a prebuilt bundle, for a packaging job that builds JS and Rust in separate
//! stages. It is explicit, it is not a fallback, and nothing selects it
//! automatically.
//!
//! ## The cfg this emits
//!
//! `vn_devtools` — "this build has the devtools protocol, the embedded
//! inspector and the F12 key". True for every dev build (the devtools have
//! always been a dev-build feature) and for a `--features prod,devtools`
//! release. `--features prod` alone stays exactly what it was: no listener, no
//! host fns, no bundle, no F12.

use std::path::{Path, PathBuf};
use std::process::Command;

fn main() {
    println!("cargo::rustc-check-cfg=cfg(vn_devtools)");
    println!("cargo:rerun-if-env-changed=VN_DEVTOOLS_BUNDLE");

    let prod = std::env::var_os("CARGO_FEATURE_PROD").is_some();
    let asked = std::env::var_os("CARGO_FEATURE_DEVTOOLS").is_some();
    // Dev builds have carried the devtools since m10 and keep doing so with no
    // feature flag; a prod build opts in by name.
    if prod && !asked {
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
    println!("cargo:rerun-if-changed={}", disp(&repo.join("packages/vue-native/prod.ts")));
    println!("cargo:rerun-if-changed={}", disp(&repo.join("packages/vue-native/tailwind.ts")));
    println!("cargo:rerun-if-changed={}", disp(&repo.join("packages/vue-native/package.json")));

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
        Ok(s) => panic!("{}", failure(&pkg, &entry, &bundle, &format!("`bun prod.ts` exited with {s}"))),
        Err(e) => panic!("{}", failure(&pkg, &entry, &bundle, &format!("`bun` could not be run: {e}"))),
    }
    if !bundle.is_file() {
        panic!("{}", failure(&pkg, &entry, &bundle, "`bun prod.ts` reported success but wrote no bundle"));
    }
    // prod.ts always writes one beside the bundle; belt and braces, because an
    // `include_str!` of a missing file is the error message this file exists to
    // never emit.
    if !window_json.is_file() {
        std::fs::write(&window_json, "{}\n").expect("cannot write a placeholder window.json");
    }
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
                cargo build -p vn-app --features prod        (no devtools)\n\
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
    let Ok(entries) = std::fs::read_dir(dir) else { return };
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
