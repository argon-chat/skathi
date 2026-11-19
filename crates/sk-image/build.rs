fn main() {
    if std::env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("msvc") {
        // Reproducible (no timestamp, no PDB path), no entry point, no C runtime.
        for arg in ["/Brepro", "/DEBUG:NONE", "/NOENTRY", "/NODEFAULTLIB"] {
            println!("cargo:rustc-cdylib-link-arg={arg}");
        }
    }
}
