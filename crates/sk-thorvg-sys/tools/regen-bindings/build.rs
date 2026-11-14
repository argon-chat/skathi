//! Bakes the build's target triple in so `main.rs` can hand it to libclang.
fn main() {
    println!(
        "cargo:rustc-env=TVG_HOST_TARGET={}",
        std::env::var("TARGET").unwrap()
    );
}
