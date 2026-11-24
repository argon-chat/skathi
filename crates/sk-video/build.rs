fn main() {
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("macos") {
        let source = "src/avf/audio_tap.m";
        println!("cargo:rerun-if-changed={source}");
        cc::Build::new()
            .file(source)
            .flag("-fobjc-arc")
            .compile("sk_media_audio_tap");
        println!("cargo:rustc-link-lib=framework=MediaToolbox");
        println!("cargo:rustc-link-lib=framework=AudioToolbox");
    }
}
