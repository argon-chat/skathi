//! Compiles the vendored ThorVG C++ subset into a static archive with `cc`.
//!
//! Two things this build script deliberately does **not** do, because both
//! would break the project's one-command-`cargo build` guarantee:
//!
//! * **No `bindgen`.** The FFI declarations are checked in at
//!   `src/bindings.rs`; `tools/regen-bindings/` regenerates them when the
//!   pinned ThorVG version moves. libclang is therefore a *contributor*
//!   dependency, never a build dependency — the same split
//!   `crates/vn-effects` uses for `slangc`.
//! * **No network fetch, no `cmake`/`meson`/`ninja`, no `pkg-config`.** The
//!   C++ sources live under `thorvg/`, vendored pristine from the pinned
//!   upstream release by `tools/vendor-thorvg.sh`.
//!
//! The compile targets hosted desktop platforms only (MSVC / GNU / Apple).
//! Upstream's bare-metal and cross-toolchain machinery is out of scope; a
//! host that is not one of the three is simply built with cc-rs defaults.

use std::env;
use std::path::{Path, PathBuf};

fn main() {
    let manifest_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap());
    let out_dir = PathBuf::from(env::var("OUT_DIR").unwrap());
    let thorvg = manifest_dir.join("thorvg");
    let src = thorvg.join("src");

    write_config_h(&out_dir);

    let is_msvc = env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("msvc");
    let target_os = env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    let target_vendor = env::var("CARGO_CFG_TARGET_VENDOR").unwrap_or_default();

    let mut build = cc::Build::new();
    build.cpp(true).std("c++14").warnings(false);

    // Statically linked, so the C API must carry no dllimport/dllexport
    // decoration — MSVC otherwise rejects the definitions in tvgCapi.cpp as
    // `dllimport` (C2491). This is meson's `-DTVG_STATIC`.
    build.define("TVG_STATIC", None);

    if is_msvc {
        // The Windows SDK's `min`/`max` macros mangle tvgMath.h's
        // `Point min(...)` / `max(...)` declarations into a syntax error
        // (C2059). Upstream's meson passes the same define on Windows.
        build.define("NOMINMAX", None);
        // ThorVG uses exceptions in its loaders; /EHsc selects the standard
        // C++ exception model. cc-rs picks /MT from `+crt-static` on its own,
        // which is what the workspace's .cargo/config.toml demands.
        build.flag("/EHsc");
        // `pragma GCC`/`#warning` noise from the vendored JerryScript.
        build.flag("/wd4068");
    } else {
        // Mirrors upstream's `src/meson.build` flag set. Pure code-shape
        // choices — ThorVG's non-MSVC builds are exception- and RTTI-free.
        for flag in [
            "-fno-exceptions",
            "-fno-rtti",
            "-fno-stack-protector",
            "-fno-math-errno",
            "-Wno-unknown-pragmas",
        ] {
            build.flag_if_supported(flag);
        }
    }

    // POSIX surface (strdup, strncasecmp) on libcs that gate it.
    build.define("_DEFAULT_SOURCE", None);

    if cfg!(feature = "avx") && env::var("CARGO_CFG_TARGET_ARCH").as_deref() == Ok("x86_64") {
        // Off by default, matching upstream's `simd=false`: AVX raises the
        // CPU baseline to Sandy Bridge and upstream does not consider the
        // win worth that. MSVC compiles the `<immintrin.h>` intrinsics in
        // tvgSwRasterAvx.h without an /arch: switch.
        if !is_msvc {
            build.flag_if_supported("-mavx");
        }
    }

    // JerryScript's `JERRY_VLA` fallback calls alloca() without including
    // <alloca.h>, which MSVC does not have at all. C99 VLAs are what the
    // macro is standing in for.
    if cfg!(feature = "expressions") {
        build.define("JERRY_VLA(type,name,size)", "type name[size]");
    }

    for dir in include_dirs(&thorvg, &src, &out_dir) {
        build.include(dir);
    }
    for file in sources(&src) {
        build.file(file);
    }
    build.compile("thorvg");

    println!("cargo:rustc-link-lib=static=thorvg");
    // cc-rs's automatic C++ runtime emission is unreliable; name it per
    // platform. MSVC links its C++ runtime through the CRT choice already.
    if target_vendor == "apple" || target_os == "freebsd" {
        println!("cargo:rustc-link-lib=dylib=c++");
    } else if !is_msvc && (target_os == "linux" || target_os == "android") {
        println!("cargo:rustc-link-lib=dylib=stdc++");
    }

    println!("cargo:rerun-if-changed=thorvg");
    println!("cargo:rerun-if-changed=build.rs");
}

/// The `.cpp` set, matching the enabled cargo features.
///
/// GPU engines are never compiled: `thorvg/src/renderer/gpu_engine/` is not
/// even vendored, because linking GL/WebGPU would need system headers.
fn sources(src: &Path) -> Vec<PathBuf> {
    let mut sources = Vec::new();
    push_cpp(&mut sources, &src.join("common"));
    push_cpp(&mut sources, &src.join("renderer"));
    push_cpp(&mut sources, &src.join("renderer/cpu_engine"));
    push_cpp(&mut sources, &src.join("bindings/capi"));
    push_cpp(&mut sources, &src.join("loaders/raw"));

    push_cpp(&mut sources, &src.join("loaders/lottie"));
    if cfg!(feature = "expressions") {
        push_cpp_recursive(&mut sources, &src.join("loaders/lottie/jerryscript"));
    } else {
        // The expressions TU includes JerryScript headers that are not on
        // the include path in this configuration.
        sources.retain(|path| !path.ends_with("tvgLottieExpressions.cpp"));
    }
    if cfg!(feature = "png") {
        push_cpp(&mut sources, &src.join("loaders/png"));
    }
    if cfg!(feature = "jpg") {
        push_cpp(&mut sources, &src.join("loaders/jpg"));
    }
    if cfg!(feature = "fonts") {
        push_cpp(&mut sources, &src.join("loaders/sfnt"));
    }
    sources
}

fn include_dirs(thorvg: &Path, src: &Path, out_dir: &Path) -> Vec<PathBuf> {
    let mut dirs = vec![
        out_dir.to_path_buf(),  // the generated config.h
        thorvg.join("inc"),     // the public C++ header
        src.join("renderer"),
        src.join("renderer/cpu_engine"),
        src.join("common"),
        src.join("bindings/capi"),
        src.join("loaders/raw"),
        src.join("loaders/lottie"),
        src.join("loaders/lottie/rapidjson"),
    ];
    if cfg!(feature = "expressions") {
        let jerry = src.join("loaders/lottie/jerryscript");
        for sub in [
            "jerry-core",
            "jerry-core/include",
            "jerry-core/ecma/base",
            "jerry-core/ecma/builtin-objects",
            "jerry-core/ecma/builtin-objects/typedarray",
            "jerry-core/ecma/operations",
            "jerry-core/jcontext",
            "jerry-core/jmem",
            "jerry-core/jrt",
            "jerry-core/lit",
            "jerry-core/parser/js",
            "jerry-core/parser/regexp",
            "jerry-core/vm",
            "jerry-port/common",
        ] {
            dirs.push(jerry.join(sub));
        }
    }
    if cfg!(feature = "png") {
        dirs.push(src.join("loaders/png"));
    }
    if cfg!(feature = "jpg") {
        dirs.push(src.join("loaders/jpg"));
    }
    if cfg!(feature = "fonts") {
        dirs.push(src.join("loaders/sfnt"));
    }
    dirs
}

/// ThorVG's meson generates `config.h`; cargo features stand in for the meson
/// options here. The values track upstream's own defaults (`meson_options.txt`)
/// so the vendored engine behaves like a stock `meson setup` build — the
/// exceptions are the GPU engines and the SVG/WebP loaders, which are not
/// vendored at all.
fn write_config_h(out_dir: &Path) {
    let mut config = String::from(
        "/* Generated by vn-thorvg-sys build.rs — do not edit. */\n\
         #pragma once\n\n\
         #define THORVG_VERSION_STRING \"1.1.0\"\n\
         #define THORVG_CAPI_BINDING_SUPPORT 1\n\
         #define THORVG_CPU_ENGINE_SUPPORT 1\n\
         #define THORVG_LOTTIE_LOADER_SUPPORT 1\n\
         #define THORVG_FILE_IO_SUPPORT 1\n\
         #define THORVG_PARTIAL_RENDER_SUPPORT 1\n",
    );
    if cfg!(feature = "threads") {
        config.push_str("#define THORVG_THREAD_SUPPORT 1\n");
    }
    if cfg!(feature = "expressions") {
        config.push_str("#define THORVG_LOTTIE_EXPRESSIONS_SUPPORT 1\n");
    }
    if cfg!(feature = "png") {
        config.push_str("#define THORVG_PNG_LOADER_SUPPORT 1\n");
    }
    if cfg!(feature = "jpg") {
        config.push_str("#define THORVG_JPG_LOADER_SUPPORT 1\n");
    }
    if cfg!(feature = "fonts") {
        config.push_str("#define THORVG_SFNT_LOADER_SUPPORT 1\n");
        config.push_str("#define THORVG_TTF_LOADER_SUPPORT 1\n");
        config.push_str("#define THORVG_OTF_LOADER_SUPPORT 1\n");
    }
    if cfg!(feature = "avx") {
        config.push_str("#define THORVG_AVX_VECTOR_SUPPORT 1\n");
    }
    std::fs::write(out_dir.join("config.h"), config).expect("write config.h");
}

fn push_cpp(out: &mut Vec<PathBuf>, dir: &Path) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        panic!("vendored ThorVG source directory missing: {}", dir.display());
    };
    let mut found: Vec<PathBuf> = entries
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "cpp"))
        .collect();
    // read_dir order is filesystem-dependent; sort so the archive's member
    // order (and therefore the build's reproducibility) does not drift.
    found.sort();
    out.extend(found);
}

fn push_cpp_recursive(out: &mut Vec<PathBuf>, dir: &Path) {
    let mut stack = vec![dir.to_path_buf()];
    let mut found = Vec::new();
    while let Some(current) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&current) else {
            panic!(
                "vendored ThorVG source directory missing: {}",
                current.display()
            );
        };
        for path in entries.flatten().map(|entry| entry.path()) {
            if path.is_dir() {
                stack.push(path);
            } else if path.extension().is_some_and(|ext| ext == "cpp") {
                found.push(path);
            }
        }
    }
    found.sort();
    out.extend(found);
}
