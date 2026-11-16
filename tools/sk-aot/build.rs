use std::path::{Path, PathBuf};
use std::{env, fs};

fn read(path: &Path) -> String {
    fs::read_to_string(path)
        .unwrap_or_else(|error| panic!("cannot read {}: {error}", path.display()))
}

fn json_field<'a>(text: &'a str, section: &str, field: &str) -> &'a str {
    let rest = &text[text
        .find(&format!("\"{section}\""))
        .unwrap_or_else(|| panic!("toolchain pin has no {section}"))..];
    let rest = &rest[rest
        .find(&format!("\"{field}\""))
        .unwrap_or_else(|| panic!("toolchain pin has no {section}.{field}"))..];
    let rest = &rest[rest.find(':').unwrap() + 1..];
    let start = rest.find('"').unwrap() + 1;
    let end = start + rest[start..].find('"').unwrap();
    &rest[start..end]
}

fn checkout_revision(checkout: &Path) -> String {
    read(&checkout.join(".git").join("HEAD")).trim().to_owned()
}

fn main() {
    let manifest = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap());
    let sdk = manifest
        .parent()
        .and_then(Path::parent)
        .unwrap()
        .to_path_buf();
    let lock = sdk.join("packages/cli/aot/toolchain.lock.json");
    let pin = read(&lock);
    let frontend = json_field(&pin, "frontend", "revision").to_owned();
    let engine = json_field(&pin, "engine", "revision").to_owned();
    let table = json_field(&pin, "table", "id").to_owned();

    let bun = manifest.join(".cache/bun");
    let checked_out = checkout_revision(&bun);
    assert_eq!(
        checked_out,
        frontend,
        "{} is at {checked_out}, the toolchain pin names frontend {frontend}; run bun tools/sk-aot/setup.ts",
        bun.display()
    );
    let pins = read(&sdk.join("vendor/redjs/redjs/pins.txt"));
    assert!(
        pins.lines()
            .any(|line| line.split('#').next().unwrap().trim()
                == format!("webkit.version = {engine}")),
        "vendor/redjs/redjs/pins.txt does not pin engine {engine} of the toolchain pin"
    );
    println!("cargo:rustc-env=SK_AOT_PIN_FRONTEND={frontend}");
    println!("cargo:rustc-env=SK_AOT_PIN_ENGINE={engine}");
    println!("cargo:rustc-env=SK_AOT_PIN_TABLE={table}");
    println!("cargo:rerun-if-changed={}", lock.display());
    println!("cargo:rerun-if-changed={}", bun.join(".git/HEAD").display());
    println!(
        "cargo:rerun-if-changed={}",
        sdk.join("vendor/redjs/redjs/pins.txt").display()
    );

    let redjs = sdk.join("vendor/redjs/redjs");
    let webkit = env::var("BUN_WEBKIT_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| sdk.join("vendor/redjs/prebuilt/bun-webkit"));
    let include = webkit.join("include");
    let codegen = PathBuf::from(env::var("BUN_CODEGEN_DIR").expect("BUN_CODEGEN_DIR"));
    cc::Build::new()
        .cpp(true)
        .compiler("clang-cl")
        .static_crt(true)
        .file("engine/compile.cpp")
        .include(&include)
        .include(include.join("JavaScriptCore"))
        .include(include.join("JavaScriptCore/aot"))
        .include(redjs.join("cpp/compat"))
        .include(redjs.join("cpp/aot"))
        .include(&codegen)
        .include("engine")
        .define("NDEBUG", None)
        .define("STATICALLY_LINKED_WITH_JavaScriptCore", "1")
        .define("STATICALLY_LINKED_WITH_WTF", "1")
        .define("STATICALLY_LINKED_WITH_BMALLOC", "1")
        .define("BUILDING_WITH_CMAKE", "1")
        .define("NOMINMAX", None)
        .define("WIN32", None)
        .define("_WINDOWS", None)
        .define("WIN32_LEAN_AND_MEAN", "1")
        .define("_CRT_SECURE_NO_WARNINGS", None)
        .flag("/std:c++23preview")
        .flag("/W0")
        .flag("-Wno-inconsistent-dllimport")
        .flag("/FIcmakeconfig.h")
        .compile("sk_aot_engine");
    // bun_sys does its file I/O through libuv on Windows: Bun's fork and sources, at Bun's pin.
    let libuv = bun.join("vendor/libuv");
    let shared = [
        "fs-poll",
        "idna",
        "inet",
        "random",
        "strscpy",
        "strtok",
        "thread-common",
        "threadpool",
        "timer",
        "uv-common",
        "uv-data-getter-setters",
        "version",
    ];
    let win = [
        "async",
        "core",
        "detect-wakeup",
        "dl",
        "error",
        "fs",
        "fs-event",
        "getaddrinfo",
        "getnameinfo",
        "handle",
        "loop-watcher",
        "pipe",
        "thread",
        "poll",
        "process",
        "process-stdio",
        "signal",
        "snprintf",
        "stream",
        "tcp",
        "tty",
        "udp",
        "util",
        "winapi",
        "winsock",
    ];
    cc::Build::new()
        .compiler("clang-cl")
        .static_crt(true)
        .files(
            shared
                .iter()
                .map(|name| libuv.join(format!("src/{name}.c"))),
        )
        .files(
            win.iter()
                .map(|name| libuv.join(format!("src/win/{name}.c"))),
        )
        .include(libuv.join("include"))
        .include(libuv.join("src"))
        .define("WIN32_LEAN_AND_MEAN", None)
        .define("_CRT_DECLARE_NONSTDC_NAMES", "0")
        .define("WIN32", None)
        .define("_WINDOWS", None)
        .define("_WIN32_WINNT", "0x0A00")
        .flag("/clang:-fno-strict-aliasing")
        .flag("-Wno-int-conversion")
        .flag("/wd4996")
        .warnings(false)
        .compile("uv");
    for library in [
        "ws2_32", "iphlpapi", "userenv", "dbghelp", "ole32", "shell32", "psapi", "advapi32",
        "user32",
    ] {
        println!("cargo:rustc-link-lib={library}");
    }

    // Bun__removableFeatureState and Bun__removableFeatureName: Bun's trim crate and redjs both
    // define the untrimmed engine's, with the same contents.
    println!("cargo:rustc-link-arg-bins=/FORCE:MULTIPLE");
    // Folded natives (DataView's setInt8 and setUint8) give one boxed call two stubs, which
    // the engine refuses when it creates a realm.
    println!("cargo:rustc-link-arg-bins=/OPT:NOICF");
    println!("cargo:rerun-if-changed=engine");
}
