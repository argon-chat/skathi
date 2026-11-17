---
title: Embedding Skathi in a Rust program
label: Embedding
description: Run a Skathi app inside your own Rust executable with the skathi crate. Functions the app calls, events you send it, native namespaces, native frames in <sk-surface>, the dev loop with hot reload, and what it costs.
section: Guides
order: 9
---

`sk build` gives you a finished exe: Skathi's own host with your app stamped in. That covers most apps. Sometimes the program comes first, though: a sync engine, a game launcher, a player built around libmpv, a tool with years of Rust behind it. It needs a UI, and it should not become a sidecar process that talks to a webview over a socket.

For that, the [`skathi`](https://docs.rs/skathi) crate puts the whole engine (gpui, JavaScriptCore, the CSS engine, the runtime) inside **your** executable. Your `main` stays yours. The app is the same Vue or Svelte project you would build with `sk`, packed into a payload, and the two halves talk through async functions and events, with no HTTP, IPC or webview in between.

<div class="tradeoffs">
<div class="tradeoff" data-tone="gain">

**What you get**

- **Your process, your `main`.** Your threads, your allocator, your logging, your crash handling. Skathi runs on a thread of its own and hands back a `Handle`.
- **Calls without plumbing.** `appHost.call("db.query", sql)` in the app is a Rust closure in yours. The result comes back as a promise.
- **Native frames in the page.** A renderer you own (a video player, a game viewport, another process) draws into [`<sk-surface>`](#native-frames-in-the-page), and the app's Vue or Svelte controls sit over it.
- **One file if you want it.** `payload_static(include_bytes!(…))` compiles the app into the binary.
- **The same app runs both ways.** `appHost.available` tells it whether a host is there, so `sk dev` and `sk build` keep working on their own.

</div>
<div class="tradeoff" data-tone="cost">

**What it costs**

- **A heavier build.** The first `cargo build` downloads the pinned JavaScriptCore prebuilt (about 1.6 GB unpacked on Windows). On Windows it needs MSVC, `clang-cl` and the static CRT.
- **A build script.** Your exe must carry linker arguments and, on Windows, a manifest. The crate tells your `build.rs` what to pass.
- **Once per process.** Skathi starts once and cannot be restarted in the same process.
- **The UI thread is shared.** Function handlers run on the Skathi thread; one that blocks freezes the UI.
- **Versions in lockstep.** The `sk` CLI that builds the payload and the crate must have the same version.

</div>
</div>

## How the pieces fit

<div class="pipeline">
<p class="pipeline-phase">On your machine</p>

1. **The app** `src/**/*.vue`, `*.svelte`, `*.ts`

   An ordinary Skathi project, made with `bun create skathi`. It calls the host through `appHost`.

2. **The payload** `sk publish --payload-only`

   The production bundle, styles, assets and `sk.config.json`, packed into `dist/release/.sk/app.skpk`. No exe is stamped.

3. **Your executable** `cargo build --release`

   Your Rust code plus the `skathi` crate. `build.rs` adds the linker arguments and the Windows manifest.

<p class="pipeline-phase" data-tone="run">When your program runs</p>

4. **`Builder`**

   Your `main` names the payload, registers the functions the app may call, and optionally a `setup` hook.

5. **`spawn()`** returns a `Handle`

   Skathi starts on its own thread, loads the payload and opens the app's window. `spawn` returns at once.

6. **Both sides talk**

   The app calls your functions with `appHost.call`; you send it events with `Handle::emit`; frames you present show in `<sk-surface>`.

7. **`wait()`** returns an `Exit`

   When the app closes, `wait` returns whether the teardown was clean. The rest of the process is yours.

</div>

## Your first embedder

Start with the app. Any template works; `minimal` is the smallest:

```sh
bun create skathi app --template minimal --yes
cd app && bun install
bunx sk publish --payload-only   # writes app/dist/release/.sk/app.skpk
```

Then the Rust side, next to it:

```toml title="Cargo.toml"
[package]
name = "notes"
version = "0.1.0"
edition = "2021"

[dependencies]
skathi = "0.3"
serde_json = "1"

[build-dependencies]
embed-resource = "3"
```

```toml title=".cargo/config.toml"
[target.x86_64-pc-windows-msvc]
rustflags = ["-C", "target-feature=+crt-static"]
```

The crate cannot add linker arguments or a manifest to an executable it does not own, so it reports them to your build script, and the build script passes them on:

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

And `main`:

```rust title="src/main.rs"
fn main() {
    let ui = skathi::Builder::new()
        .payload_file("app/dist/release/.sk/app.skpk")
        .function("greet", |call| {
            let args: Vec<String> = serde_json::from_str(call.args()).unwrap_or_default();
            call.resolve(serde_json::json!(format!("hello, {}", args.join(" "))).to_string());
        })
        .spawn()
        .unwrap_or_else(|e| {
            eprintln!("{e}");
            std::process::exit(2)
        });
    ui.emit("ready", r#"{"build":"1.0"}"#).ok();
    std::process::exit(if ui.wait().clean { 0 } else { 1 });
}
```

`cargo run --release`, and the app opens in a window of your exe. On macOS, `spawn` is replaced by [`run_on_main_thread`](#macos-owns-the-main-thread).

> **Note:** An application with its own Windows manifest uses that instead of `gpui.rc`, and must keep the Common Controls 6 dependency: without it Windows refuses to start the exe.

## Talking to the app

Everything crosses as JSON: arguments arrive as one JSON array, results and event data are one JSON value each. Functions, `undefined` and class instances do not survive the trip.

### Functions the app calls

`Builder::function(name, handler)` registers a function; the app calls it with [`appHost.call`](/app-api/app-host/):

```ts title="src/notes.ts"
import { appHost } from "@skathi-native/runtime";

interface Note {
  id: number;
  title: string;
}

export async function search(text: string): Promise<Note[]> {
  return appHost.call<Note[]>("notes.search", text, { limit: 50 });
}
```

The handler gets a `Call` with the arguments, `["text", {"limit":50}]` here, and settles the promise with `resolve(json)` or `reject(message)`. A rejection arrives in the app as an `AppHostError` whose `function` names the call.

Handlers run **on the Skathi thread**, inside the JS call. That is fine for a lookup in memory, and wrong for anything that touches a disk, a network or a lock. A handler with real work takes the `Reply` out of the call and answers from another thread:

```rust
.function("notes.search", move |call| {
    let args: (String, serde_json::Value) = match serde_json::from_str(call.args()) {
        Ok(args) => args,
        Err(e) => return call.reject(format!("bad arguments: {e}")),
    };
    let reply = call.into_reply();
    let index = index.clone();
    std::thread::spawn(move || match index.search(&args.0) {
        Ok(notes) => reply.resolve(serde_json::to_string(&notes).unwrap()),
        Err(e) => reply.reject(e.to_string()),
    });
})
```

A `Reply` can be sent to any thread, and one dropped without an answer rejects, so a promise in the app never hangs because a worker gave up.

### Events you send

`Handle::emit(event, json)` delivers to every [`appHost.on`](/app-api/host-on/) handler for that event in the app's next frame. The `Handle` is cheap to clone and works from any thread:

```rust
let ui = builder.spawn()?;
let events = ui.clone();
std::thread::spawn(move || {
    for progress in sync.progress() {
        if events.emit("sync.progress", &progress.to_json()).is_err() {
            break; // the app has closed
        }
    }
});
```

```ts
const stop = appHost.on("sync.progress", (data) => {
  progress.value = data as SyncProgress;
});
```

Events emitted before the app is ready wait for it. `emit` refuses data that is not one JSON value, and anything after the app closed.

### Running both ways

The same project still runs under `sk dev` and as its own exe, where there is no host: `appHost.available` is `false`, `call` rejects and `on` never fires. An app that should work in both checks first:

```ts
const name =
  appHost.available && appHost.functions.includes("user.name") ? await appHost.call<string>("user.name") : "Guest";
```

## Going native: the setup hook

Functions and events are the easy path. For more, `Builder::setup` runs once on the Skathi thread after storage is ready and before any app JS or window, with a `HostContext`:

| On `HostContext`                                           | What it is for                                                                                                  |
| ---------------------------------------------------------- | --------------------------------------------------------------------------------------------------------------- |
| `js()`, `expose(name, value)`                              | Direct JavaScriptCore: build objects and native functions and install them as a global, with no JSON in between |
| `dispatcher()`                                             | A bounded queue from your worker threads onto the JS thread; post owned Rust data, build JS values inside       |
| `on_frame`, `on_reload`, `on_shutdown`                     | Deliver queued work once per frame; clean up page-owned callbacks on reload; release them on shutdown           |
| `handle()`                                                 | The running app's `Handle`, for UI that outlives setup, such as a tray icon                                     |
| `surface(name)`                                            | The surface `<sk-surface name="…">` shows; see [Native frames in the page](#native-frames-in-the-page)          |
| `set_cookie`, `read_secret`, `write_secret`, `storage_dir` | Provision cookies (HttpOnly too) before the first request; secrets sealed by the OS, never in plain text        |

A native namespace is a plain JS object whose methods are Rust functions. They run synchronously and return real JS values, which is the right tool for something the app calls often and cheaply:

```rust
use skathi::js;

let builder = skathi::Builder::new().setup(|host| {
    let cx = host.js();
    let clock = cx.object();
    let now = js::function(cx, "now", 0, |_, _| Ok(js::Value::number(monotonic_ms())))
        .map_err(|e| e.to_string())?;
    cx.set(clock, &cx.atom("now"), now).map_err(|e| e.to_string())?;
    host.expose("nativeClock", clock).map_err(|e| e.to_string())?;
    Ok(())
});
```

The app calls `nativeClock.now()`. A Rust error throws a JS exception, and a panic is caught and thrown as one too.

> **Warning:** A JS `Value` is only safe on the JS thread's stack or while protected. Never move one into a worker thread, even though it is `Copy`. Workers carry Rust data, and the dispatcher turns it into JS on the right thread.

Register native functions once, in `setup`: each one takes a slot for the life of the process.

## Native frames in the page

This is the part a webview cannot do. A renderer you own presents frames to a surface by name, and every `<sk-surface name="…">` in the app shows the newest one, as an ordinary box: `object-fit`, `border-radius`, clipping, transforms and z-order apply, and the app's controls sit over it.

```vue
<template>
  <div class="player">
    <sk-surface name="player" class="video" @surfaceresize="resized" />
    <PlayerControls class="controls" />
  </div>
</template>
```

```rust
let surface = skathi::surface("player"); // or host.surface("player") in setup
std::thread::spawn(move || loop {
    let info = surface.info(); // the painted size in physical pixels, and whether it is visible
    if info.visible {
        let frame = renderer.render(info.width, info.height);
        surface.present_bgra(&frame.pixels, frame.width, frame.height, frame.stride).ok();
    }
    renderer.wait_for_next_frame();
});
```

`present_bgra` works on every platform. On Windows a frame can stay on the GPU: a Direct3D 11 or 12 texture on the renderer's own device, or one shared from another device, API (Vulkan included) or process. A present never waits for the UI, the newest frame wins, and frames presented while the element is hidden or scrolled away cost no upload.

[Showing native GPU content](/guides/native-gpu-content/) covers the GPU paths, synchronisation, device loss and a libmpv recipe.

## The dev loop

You do not have to rebuild a payload for every edit. A build of your exe **without** the crate's `prod` feature boots the dev output that `sk dev` writes, with hot reload, inside your program. Give your crate a feature for it:

```toml title="Cargo.toml"
[dependencies]
skathi = { version = "0.3", default-features = false, features = ["audio", "storages", "clipboard", "indexeddb", "webgpu", "native-dx12"] }

[features]
default = ["release"]
release = ["skathi/prod"]
```

```rust
let mut builder = skathi::Builder::new().function("greet", greet);
#[cfg(feature = "release")]
{
    builder = builder.payload_file("app/dist/release/.sk/app.skpk");
}
```

Then build the dev exe once and let `sk dev` run it instead of its own host:

```sh
cargo build --no-default-features
cd app && bunx sk dev --exe ../target/debug/notes.exe
```

`sk dev` watches the sources and starts your exe with the dev output; edits to `.vue`, `.svelte`, `.ts` and `.css` apply without a restart, and your functions are there the whole time. A dev build given a payload refuses it, with `Error::PayloadNeedsProd`.

### Devtools

F12 and `appDevtools.open()` start the inspector as a separate process. In your exe there is no `sk-app.exe` to start, so point the builder at your own exe and route the flag to it:

```rust
fn main() {
    if std::env::args().nth(1).as_deref() == Some("--devtools") {
        return skathi::run_exe();
    }
    let builder = skathi::Builder::new().inspector_exe(std::env::current_exe().unwrap());
    // …
}
```

Without `inspector_exe`, F12 says why nothing opened; the devtools protocol works either way. The `no-devtools` feature compiles both out.

## Shipping

- **Next to the exe.** `payload_file` maps the `.skpk` read-only for the app's lifetime. An app split into asset packs (`fonts.skpk`, `assets.skpk`, `icu.skpk`) keeps them beside `app.skpk`; the builder finds them there. Update the payload by replacing the file atomically, never in place.
- **Inside the exe.** `payload_static(include_bytes!("../app/dist/release/.sk/app.skpk"))` makes a single file. The app is then rebuilt into the binary on every change.
- **Profile and storage.** `data_dir` sets where localStorage, cookies, IndexedDB and the caches live, and `storage_encrypted(true)` seals them with the OS provider. `prepare_data_dir` runs before storage loads, for migrations.
- **Single instance and deep links.** `desktop_launch(true)` lets your exe apply the payload's `singleInstance` and protocol settings to the real command line. A second launch that was forwarded to the first returns `Error::LaunchExit(0)`: return that code from `main`.
- **Hidden at start.** `visible(false)` boots the app without showing its window, for a tray app; `Handle::show()` shows it later. [`appTray`](/app-api/app-tray/) and [`appHotkeys`](/app-api/app-hotkeys/) work in an embedded app as they do in Skathi's own host, and `skathi::tray` and `skathi::hotkeys` give the same services to your Rust code.

### macOS owns the main thread

AppKit needs the process main thread, so on macOS `spawn` returns `Error::MainThreadRequired`. `run_on_main_thread` runs the app there instead and keeps the thread until the app closes; the callback runs after shutdown, stops your workers and returns the exit code:

```rust
#[cfg(target_os = "macos")]
if let Err(e) = builder.run_on_main_thread(|exit| i32::from(!exit.clean)) {
    eprintln!("{e}");
    std::process::exit(2);
}
```

There is no `Handle` returned on macOS: take it from `HostContext::handle()` in `setup`.

## Features

| Feature           | Default | What it adds                                                                     |
| ----------------- | ------- | -------------------------------------------------------------------------------- |
| `prod`            | on      | Runs a payload. Off, the exe boots the dev output named by `SK_DIST` (`sk dev`). |
| `audio`           | on      | Web Audio                                                                        |
| `storages`        | on      | localStorage, cookies and secrets on disk                                        |
| `clipboard`       | on      | `navigator.clipboard`                                                            |
| `indexeddb`       | on      | IndexedDB                                                                        |
| `webgpu`          | on      | WebGPU                                                                           |
| `native-dx12`     | on      | The Direct3D 12 renderer on Windows; off, Direct3D 11 (no `present_d3d12`)       |
| `icu-root`        | off     | Links only ICU's root locale, about 12 MB less; see the note below               |
| `battery`         | off     | The Battery Status API                                                           |
| `no-devtools`     | off     | Compiles out the devtools protocol and the inspector                             |
| `runtime-shaders` | off     | macOS: compiles the Metal shaders at startup instead of at build time            |

> **Note:** With `icu-root`, `Intl` and `toLocale*` format in the root locale unless the full data is shipped as `icu.skpk` beside the payload. Without the feature the exe carries every locale.

## Troubleshooting

<div class="fixes">

### When it starts

- `no app: give the builder a payload`

  **Cause:** A `prod` build with no `payload_file`, `payload_bytes` or `payload_static`, and no `SK_PAYLOAD`.

  **Fix:** Name the payload, or set `SK_PAYLOAD=<path to app.skpk>`.

- `a payload needs sk-app's prod feature; a dev build boots the dev dist instead`

  **Cause:** A build without `prod` was given a payload.

  **Fix:** Build with `prod` to run the payload, or leave the payload out and start the exe through `sk dev --exe`.

- `this app was built by Skathi … and this library is …`

  **Cause:** The payload was built by an `sk` of another version than the crate.

  **Fix:** Use the CLI of the crate's version: `@skathi-native/cli` and `skathi` are released together.

- `Skathi already ran in this process; it runs once`

  **Cause:** A second `spawn` in the same process. The JS realm and gpui cannot be torn down and started again.

  **Fix:** Start Skathi once, and keep the `Handle` for the life of the process.

- `Skathi's arguments were read before spawn`

  **Cause:** Another Skathi call read the command line before `spawn`.

  **Fix:** Start Skathi before anything else of it runs. Pass the app's own arguments with `Builder::args`; they reach the app as `appLaunch.argv`.

- `AppKit requires the process main thread`

  **Cause:** `spawn` on macOS.

  **Fix:** Use `run_on_main_thread` from `main`.

### While it runs

- The window freezes while a function runs

  **Cause:** The handler does its work on the Skathi thread.

  **Fix:** `call.into_reply()`, move the `Reply` to a worker and answer from there.

- An `AppHostError` the handler never sent

  **Cause:** The `Reply` was dropped without an answer, or a reload rejected the calls still waiting.

  **Fix:** Answer every `Reply` on every path, errors included.

- `emit` returns `the event's data is not JSON`

  **Cause:** `data` is not one JSON value; a bare string needs its quotes, as `"\"ready\""`.

  **Fix:** Serialise with `serde_json`.

</div>

## Platform support

| Platform    | Embedding | Starts with          | `<sk-surface>` frames     |
| ----------- | --------- | -------------------- | ------------------------- |
| Windows x64 | Yes       | `spawn`              | GPU textures and CPU BGRA |
| macOS arm64 | Yes       | `run_on_main_thread` | CPU BGRA                  |
| Linux x64   | Yes       | `spawn`              | CPU BGRA                  |

On Linux, set the app's desktop entry id with `Builder::desktop_id` so the desktop matches the window to its icon and dock slot. The build needs clang and the development packages of Wayland, X11 (xcb, xkbcommon), fontconfig and ALSA; on macOS, Xcode with its Metal toolchain and `MACOSX_DEPLOYMENT_TARGET=13.0` or later.

## When to embed, and when not to

Embed when the native program is the product and the UI is its face: a player, a launcher, a client with a heavy Rust core, a tool that renders its own content. Build with `sk` when the app is the product: you get the exe, packaging, signing and updates without owning a Rust build. Both use the same app project, so moving from one to the other later is a change on the Rust side only.
