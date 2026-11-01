//! Compiles the vendored libvpx VP9 decoder with `cc`.
//!
//! No `configure`, `make`, nasm or yasm runs here, and nothing is fetched: the
//! headers configure and rtcd.pl would generate are checked in under
//! `config/` (`tools/vendor-libvpx.ts` regenerates them), and `sources.txt`
//! lists the C files. Two configurations:
//!
//! * `x86_64-msvc` — x86_64 with MSVC: libvpx's SSE2/SSSE3/AVX2 intrinsics with
//!   runtime CPU detection. libvpx's assembly is not built: the rtcd entries
//!   with only an asm version point at C (`config/asm-only.txt`), and the asm
//!   helpers the intrinsics call are `csrc/vn_vpx_asm_shim.c`.
//! * `generic` — every other target: plain C.
//!
//! `VN_VPX_CONFIG=generic` forces the C build on x86_64 too. The tests use it
//! as the reference the intrinsics build must match bit for bit (VENDOR.md).
//!
//! Every C file is compiled with `config/prefix/<configuration>.h`
//! force-included: it renames each symbol libvpx defines to `vnvpx_*`, so this
//! copy links next to libwebrtc's libvpx (`tools/prefix-symbols.ts` generates
//! it; `VN_VPX_PREFIX=0` leaves it out, for that tool only: the Rust side
//! links to the prefixed names).
//!
//! libvpx is always compiled optimised, whatever the cargo profile: a debug
//! build of the decoder is ten times slower and nobody steps through it.

use std::env;
use std::path::{Path, PathBuf};

fn main() {
    let manifest = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap());
    let libvpx = manifest.join("libvpx");
    let arch = env::var("CARGO_CFG_TARGET_ARCH").unwrap_or_default();
    let target_env = env::var("CARGO_CFG_TARGET_ENV").unwrap_or_default();
    let msvc = target_env == "msvc";

    println!("cargo:rerun-if-env-changed=VN_VPX_CONFIG");
    println!("cargo:rerun-if-env-changed=VN_VPX_PREFIX");
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=sources.txt");
    println!("cargo:rerun-if-changed=config");
    println!("cargo:rerun-if-changed=csrc");
    println!("cargo:rerun-if-changed=libvpx");
    println!("cargo::rustc-check-cfg=cfg(vn_vpx_x86)");

    let native_x86 = arch == "x86_64" && msvc;
    let requested = env::var("VN_VPX_CONFIG").ok().filter(|v| !v.is_empty());
    let config = match requested.as_deref() {
        None if native_x86 => "x86_64-msvc",
        None | Some("generic") => "generic",
        Some("x86_64-msvc") if native_x86 => "x86_64-msvc",
        Some(other) => panic!("VN_VPX_CONFIG={other}: expected `generic`, or `x86_64-msvc` on an x86_64 MSVC target"),
    };
    let x86 = config == "x86_64-msvc";
    if x86 {
        println!("cargo:rustc-cfg=vn_vpx_x86");
    }

    let (common, x86_sources) = sources(&manifest.join("sources.txt"));
    let prefix = (env::var("VN_VPX_PREFIX").as_deref() != Ok("0"))
        .then(|| manifest.join("config").join("prefix").join(format!("{config}.h")));

    let base = |build: &mut cc::Build| {
        if let Some(header) = &prefix {
            if msvc {
                build.flag(format!("/FI{}", header.display()));
            } else {
                build.flag("-include").flag(header);
            }
        }
        build
            .include(manifest.join("config").join(config))
            .include(manifest.join("config"))
            .include(&libvpx)
            .opt_level(2)
            .debug(false)
            .warnings(false)
            // A release build of libvpx: its asserts are internal invariants,
            // and one that a damaged stream trips must not abort the process.
            .define("NDEBUG", None);
        if msvc {
            build.define("_CRT_SECURE_NO_WARNINGS", None);
            // configure's generic vpx_config.h spells INLINE `inline`, which
            // MSVC's C front end accepts only from C11 on.
            if !x86 {
                build.flag("/std:c11");
            }
        }
    };

    let mut build = cc::Build::new();
    base(&mut build);
    for file in &common {
        build.file(libvpx.join(file));
    }
    build.file(manifest.join("csrc/vn_vpx_mem.c"));
    build.file(manifest.join("csrc/vn_vpx_layout.c"));
    if env::var_os("CARGO_FEATURE_SHIM_STATS").is_some() {
        build.define("VN_VPX_SHIM_STATS", None);
    }

    if x86 {
        // AVX2 files get /arch:AVX2 so the whole function body is VEX-encoded
        // (no SSE/AVX transition stalls); libvpx calls them only after its
        // runtime check. The rest compiles at the x86_64 baseline: MSVC takes
        // SSSE3/SSE4.1 intrinsics without an /arch switch.
        let mut avx2 = cc::Build::new();
        base(&mut avx2);
        avx2.flag("/arch:AVX2");
        let mut any_avx2 = false;
        for file in &x86_sources {
            if file.ends_with("_avx2.c") {
                avx2.file(libvpx.join(file));
                any_avx2 = true;
            } else {
                build.file(libvpx.join(file));
            }
        }
        build.file(manifest.join("csrc/vn_vpx_asm_shim.c"));
        if any_avx2 {
            for object in avx2.compile_intermediates() {
                build.object(object);
            }
        }
    }

    build.compile("vn_vpx");
}

/// `sources.txt`: the `[c]` section and the `[x86_64]` section.
fn sources(path: &Path) -> (Vec<String>, Vec<String>) {
    let text = std::fs::read_to_string(path).expect("sources.txt");
    let (mut common, mut x86) = (Vec::new(), Vec::new());
    let mut section = "";
    for line in text.lines().map(str::trim) {
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if line.starts_with('[') {
            section = line;
            continue;
        }
        match section {
            "[c]" => common.push(line.to_string()),
            "[x86_64]" => x86.push(line.to_string()),
            other => panic!("sources.txt: `{line}` is under unknown section {other:?}"),
        }
    }
    (common, x86)
}
