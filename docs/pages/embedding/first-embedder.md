---
title: Your first embedder
description: From an empty folder to a Rust executable that runs a Skathi app. The payload, Cargo.toml, the build script, main, and what the first build downloads.
section: Embedding
order: 1
---

By the end of this page you have a Rust program, `notes`, whose window is a Vue app, and a function the app calls in Rust. Two folders side by side:

```text
notes/
├─ app/                ← the Skathi project (Vue or Svelte)
│  └─ dist/release/.sk/app.skpk
├─ .cargo/config.toml
├─ build.rs
├─ Cargo.toml
└─ src/main.rs
```

## Before you start

| Platform    | You need                                                                                                           |
| ----------- | ------------------------------------------------------------------------------------------------------------------ |
| Windows x64 | Rust (MSVC toolchain), the MSVC build tools and Windows SDK, `clang-cl` from LLVM on `PATH`, [bun](https://bun.sh) |
| macOS arm64 | Rust, Xcode with its Metal toolchain, `MACOSX_DEPLOYMENT_TARGET=13.0` or later, bun                                |
| Linux x64   | Rust, clang, the development packages of Wayland, X11 (xcb, xkbcommon), fontconfig and ALSA, bun                   |

bun builds the app; the Rust side does not need it.

## 1. The app

Any template works; `minimal` is the smallest:

```sh
bun create skathi app --template minimal --yes
cd app && bun install
bunx sk publish --payload-only
```

`--payload-only` stops after the payload: `app/dist/release/.sk/app.skpk` holds the production bundle, styles, assets and `sk.config.json`. `sk build --payload-only` writes a debug one to `dist/debug/.sk/`.

> **Note:** The CLI and the crate are released together and must have the same version. The payload carries the JS half of the host protocol, and the crate refuses a payload from another version with `this app was built by Skathi … and this library is …`.

## 2. Cargo.toml

```toml title="Cargo.toml"
[package]
name = "notes"
version = "0.1.0"
edition = "2024"

[dependencies]
skathi = "0.4"

[build-dependencies]
embed-resource = "3"
```

With a Cargo git dependency, a checkout without a bundled inspector builds with a warning. Set `SK_DEVTOOLS_BUNDLE` to a prebuilt `app.prod.js` to include the inspector, or enable `no-devtools` to opt out explicitly and silence the warning.

The default features give the app everything Skathi's own host has: Web Audio, storage, the clipboard, IndexedDB, WebGPU, the Direct3D 12 renderer, `<video>`, `<lottie>` and stickers, `<iframe>` and global hotkeys. [Features](#features) below lists what to turn off or on, and [Features and size](/embedding/features/) what each one costs.

## 3. The static CRT (Windows)

JavaScriptCore is linked against the static C runtime, and so must the exe be:

```toml title=".cargo/config.toml"
[target.x86_64-pc-windows-msvc]
rustflags = ["-C", "target-feature=+crt-static"]
```

The build stops with a message that says so when this is missing.

## 4. The build script

A library cannot add linker arguments or a Windows manifest to an executable it does not own. The crate reports both to your build script (as `DEP_SKATHI_LINK_ARGS` and `DEP_SKATHI_WINDOWS_RESOURCES`), and the build script passes them on:

```rust title="build.rs"
fn main() {
    for arg in std::env::var("DEP_SKATHI_LINK_ARGS").unwrap().split_whitespace() {
        println!("cargo:rustc-link-arg={arg}");
    }
    if let Ok(dir) = std::env::var("DEP_SKATHI_WINDOWS_RESOURCES") {
        let rc = std::path::Path::new(&dir).join("gpui.rc");
        embed_resource::compile(rc, embed_resource::ParamsIncludeDirs([&dir]))
            .manifest_required()
            .unwrap();
    }
}
```

What it adds:

- **Windows:** an 8 MiB main stack, which JavaScript's recursion limit is sized for; the rule for identical-code folding (see [Binary size](/embedding/shipping/#binary-size)); and a manifest with Common Controls 6 and per-monitor DPI awareness.
- **macOS:** `-ObjC`, the Swift runtime path, and the weak symbols the JavaScriptCore prebuilt leaves undefined.

> **Note:** An application with its own Windows manifest uses that instead of `gpui.rc`, and must keep the Common Controls 6 dependency: without it Windows refuses to start the exe.

## 5. main

```rust title="src/main.rs"
#[skathi::data]
struct Build {
    version: String,
}

struct Notes;

#[skathi::module(events(ready = Build))]
impl Notes {
    pub fn greet(&self, first: String, last: String) -> String {
        format!("hello, {first} {last}")
    }
}

fn main() {
    let ui = skathi::Builder::new()
        .payload_file("app/dist/release/.sk/app.skpk")
        .module("notes", Notes)
        .spawn()
        .unwrap_or_else(|e| {
            eprintln!("{e}");
            std::process::exit(2)
        });
    let build = Build { version: env!("CARGO_PKG_VERSION").into() };
    ui.emit_to("notes", "ready", &build).ok();
    std::process::exit(if ui.wait().clean { 0 } else { 1 });
}
```

`notes` is a [native module](/embedding/native-modules/): every `pub fn` of the `impl` is a function the app calls, `ready` an event it can listen for, and `#[skathi::data]` makes `Build` something both sides understand. In the app, anywhere:

```ts
import { greet, on } from "native:notes";

const greeting = greet("Ada", "Lovelace"); // "hello, Ada Lovelace"
on("ready", (build) => console.log(`notes ${build.version}`));
```

For TypeScript to know `native:notes`, let your exe describe it once it is built (step 6): `bunx sk types --exe ../target/release/notes.exe` in `app/` writes `src/native-modules.d.ts`.

On macOS, `spawn` is replaced by `run_on_main_thread`; see [Shipping](/embedding/shipping/#macos-owns-the-main-thread).

## 6. Run it

```sh
cargo run --release
```

The first build downloads the pinned JavaScriptCore prebuilt (about 1.6 GB unpacked on Windows), mimalloc, SQLite3 Multiple Ciphers and a ThorVG archive, and checks each against its hash. Later builds use the cache. `REDJS_CACHE_DIR` and `SK_THORVG_CACHE_DIR` move it, for example onto a CI cache volume.

The window that opens belongs to your process. Close it and `wait` returns; your `main` decides what happens next.

## Features

| Feature           | Default | What it adds                                                                     |
| ----------------- | ------- | -------------------------------------------------------------------------------- |
| `prod`            | on      | Runs a payload. Off, the exe boots the dev output named by `SK_DIST` (`sk dev`). |
| `types`           | on      | The TypeScript for native modules (`sk types`)                                   |
| `audio`           | on      | Web Audio                                                                        |
| `spatial-audio`   | on      | Windows Sonic, Dolby Atmos and DTS objects for `PannerNode`s; implies `audio`    |
| `storages`        | on      | localStorage, cookies and secrets on disk                                        |
| `clipboard`       | on      | `navigator.clipboard`                                                            |
| `indexeddb`       | on      | IndexedDB                                                                        |
| `webgpu`          | on      | WebGPU                                                                           |
| `native-dx12`     | on      | The Direct3D 12 renderer on Windows; off, Direct3D 11                            |
| `lottie`          | on      | `<lottie>` and `<sk-sticker>`                                                    |
| `video`           | on      | `<video>`, and WebM stickers with `lottie`                                       |
| `hotkeys`         | on      | Global hotkeys: `appHotkeys` and `skathi::hotkeys`                               |
| `webview`         | on      | `<iframe>` (WebView2 on Windows, WKWebView on macOS)                             |
| `icu-root`        | off     | Links only ICU's root locale, about 12 MB less                                   |
| `battery`         | off     | The Battery Status API                                                           |
| `no-devtools`     | off     | Compiles out the devtools protocol and the inspector                             |
| `runtime-shaders` | off     | macOS: compiles the Metal shaders at startup instead of at build time            |

With `icu-root`, `Intl` and `toLocale*` format in the root locale unless the full data is shipped as `icu.skpk` beside the payload. Without it the exe carries every locale.

What each feature adds to the exe and the build, what an app sees with it off, and feature sets for common kinds of program are on [Features and size](/embedding/features/).

The `prod` feature is also how the [dev loop](/embedding/dev-loop/) works: a build without it runs the app `sk dev` is building, with hot reload.

## The async-task patch

Skathi's own builds patch `async-task` to a merged but unreleased fix (smol-rs/async-task#108) that keeps some idle tasks from being polled again and again. Until it is released, your workspace can do the same:

```toml title="Cargo.toml"
[patch.crates-io]
async-task = { git = "https://github.com/smol-rs/async-task", rev = "41fd3359c75e221bd8b187a8ee361aa309c1714c" }
```

Next: [Native modules](/embedding/native-modules/).
