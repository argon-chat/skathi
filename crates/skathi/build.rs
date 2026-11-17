// Linker arguments only reach the package that emits them, so the embedder's build script
// passes these to its own executables.
fn main() {
    let os = std::env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    let msvc = std::env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("msvc");
    let mut args: Vec<String> = Vec::new();
    if os == "windows" && msvc {
        let resources = std::env::var("DEP_SKATHI_GPUI_WINDOWS_RESOURCES")
            .expect("skathi-gpui reports its Windows resources");
        println!("cargo::metadata=windows_resources={resources}");
        // JSC tells constructors apart by native function pointer: link.exe's folding merges them,
        // lld's SAFEICF leaves address-taken functions alone (as sk-app/build.rs). JS recursion
        // needs an 8 MiB stack.
        let lld = std::env::var("RUSTC_LINKER").is_ok_and(|linker| {
            let stem = std::path::Path::new(&linker)
                .file_stem()
                .map(|s| s.to_string_lossy().to_ascii_lowercase());
            matches!(stem.as_deref(), Some("rust-lld" | "lld-link"))
        });
        if lld && std::env::var("PROFILE").as_deref() == Ok("release") {
            args.extend(["/OPT:SAFEICF".into(), "/OPT:lldtailmerge".into()]);
        } else {
            args.push("/OPT:NOICF".into());
        }
        args.push("/STACK:8388608,4096".into());
        println!("cargo:rerun-if-env-changed=RUSTC_LINKER");
        println!("cargo:rerun-if-changed=build.rs");
    }
    if os == "macos" {
        args.extend(
            [
                "-Wl,-ObjC",
                "-Wl,-rpath,/usr/lib/swift",
                "-Wl,-no_deduplicate",
            ]
            .map(String::from),
        );
        // Weak references in the JavaScriptCore prebuilt that nothing defines.
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
            args.push(format!("-Wl,-U,_{symbol}"));
        }
    }
    println!("cargo::metadata=link_args={}", args.join(" "));
}
