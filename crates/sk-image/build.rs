fn main() {
    if std::env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("msvc") {
        // Reproducible (no timestamp, no PDB path), no entry point, no C runtime.
        for arg in ["/Brepro", "/DEBUG:NONE", "/NOENTRY", "/NODEFAULTLIB"] {
            println!("cargo:rustc-cdylib-link-arg={arg}");
        }
    }
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("macos") {
        for arg in [
            "-nostdlib",
            "-lSystem",
            "-Wl,-headerpad,0x800",
            "-Wl,-install_name,@rpath/libskimage.dylib",
            "-Wl,-no_fixup_chains",
            "-Wl,-no_adhoc_codesign",
        ] {
            println!("cargo:rustc-cdylib-link-arg={arg}");
        }
    }
}
