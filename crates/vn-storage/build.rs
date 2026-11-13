// Bun's JavaScriptCore archive has weak imports for optional Bun hooks; rustc-link-arg from
// dependencies does not reach this crate's test executable, so ld64 needs the allowance here.
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
