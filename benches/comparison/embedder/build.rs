fn main() {
    for arg in std::env::var("DEP_SKATHI_LINK_ARGS")
        .unwrap()
        .split_whitespace()
    {
        println!("cargo:rustc-link-arg={arg}");
    }
    if let Ok(dir) = std::env::var("DEP_SKATHI_WINDOWS_RESOURCES") {
        let rc = std::path::Path::new(&dir).join("gpui.rc");
        embed_resource::compile(rc, embed_resource::ParamsIncludeDirs([&dir]))
            .manifest_required()
            .unwrap();
    }
    // COMPARE_PAYLOAD overrides the app payload that is compiled into the exe.
    let payload = std::env::var("COMPARE_PAYLOAD").unwrap_or_else(|_| {
        let here = std::path::PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap());
        here.join("../app/dist/release/.sk/app.skpk")
            .display()
            .to_string()
    });
    println!("cargo:rerun-if-env-changed=COMPARE_PAYLOAD");
    println!("cargo:rerun-if-changed={payload}");
    println!("cargo:rustc-env=COMPARE_PAYLOAD={payload}");
}
