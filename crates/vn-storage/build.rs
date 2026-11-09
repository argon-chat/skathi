// The SDK runtime links Bun's JavaScriptCore archive, whose optional Bun hooks
// are weak imports. Dependency rustc-link-arg directives do not propagate to
// this crate's unit-test executable, so ld64 needs the same explicit allowance.
fn main() {
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("macos") {
        return;
    }
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
        println!("cargo:rustc-link-arg=-Wl,-U,_{symbol}");
    }
}
