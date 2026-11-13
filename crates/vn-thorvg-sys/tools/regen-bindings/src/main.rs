//! Regenerates `crates/vn-thorvg-sys/src/bindings.rs` from the vendored
//! `thorvg_capi.h`. Run it after `tools/vendor-thorvg.sh` bumps the pin.
//!
//! Deliberately a separate, workspace-excluded crate: `bindgen` needs
//! `libclang`, and the whole point of checking the output in is that a plain
//! `cargo build` never does.
//!
//! ```text
//! LIBCLANG_PATH="C:\Program Files\LLVM\bin" \
//!   cargo run --release --manifest-path crates/vn-thorvg-sys/tools/regen-bindings/Cargo.toml
//! ```
//!
//! Optional args: `<header> <out>`. Optional env: `TVG_TARGET`, the clang
//! target triple (defaults to the host triple this tool was built for).

use std::path::PathBuf;

/// The host triple, baked in by `build.rs`: libclang's own default target can disagree with the host
/// (a 32-bit default trips bindgen's pointer-size assertion), so it is always passed explicitly.
const HOST_TARGET: &str = env!("TVG_HOST_TARGET");

fn main() {
    // …/tools/regen-bindings -> …/crates/vn-thorvg-sys
    let crate_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(|path| path.parent())
        .expect("regen-bindings must live at <crate>/tools/regen-bindings")
        .to_path_buf();

    let mut args = std::env::args().skip(1);
    let header = args
        .next()
        .map(PathBuf::from)
        .unwrap_or_else(|| crate_dir.join("thorvg/src/bindings/capi/thorvg_capi.h"));
    let out = args
        .next()
        .map(PathBuf::from)
        .unwrap_or_else(|| crate_dir.join("src/bindings.rs"));

    let target = std::env::var("TVG_TARGET").unwrap_or_else(|_| HOST_TARGET.to_string());

    let bindings = bindgen::Builder::default()
        .header(header.to_string_lossy())
        // The engine is a static archive, so the header must not decorate declarations with dllimport (as in `build.rs`).
        .clang_arg("-DTVG_STATIC")
        .clang_arg(format!("--target={target}"))
        .allowlist_function("tvg_.*")
        .allowlist_type("Tvg_.*")
        .allowlist_var("TVG_.*")
        .rustified_enum("Tvg_Result")
        .rustified_enum("Tvg_Colorspace")
        // Power-of-two bitflags: `rustified_enum` would make `DEFAULT | SMART_RENDER` unrepresentable.
        .bitfield_enum("Tvg_Engine_Option")
        .rustified_enum("Tvg_Mask_Method")
        .rustified_enum("Tvg_Blend_Method")
        .rustified_enum("Tvg_Type")
        .rustified_enum("Tvg_Stroke_Cap")
        .rustified_enum("Tvg_Stroke_Join")
        .rustified_enum("Tvg_Stroke_Fill")
        .rustified_enum("Tvg_Fill_Rule")
        .rustified_enum("Tvg_Text_Wrap")
        .rustified_enum("Tvg_Filter_Method")
        .use_core()
        .layout_tests(true)
        // Doxygen comments do not survive translation legibly and would triple the file; the header is vendored next door.
        .generate_comments(false)
        .generate()
        .expect("bindgen failed — is LIBCLANG_PATH set?");

    bindings.write_to_file(&out).expect("write bindings");
    println!("wrote {}", out.display());
}
