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
        // JSC tells constructors apart by native function pointer, which identical-code folding
        // merges; JS recursion needs an 8 MiB stack.
        args.extend(["/OPT:NOICF".into(), "/STACK:8388608,4096".into()]);
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
