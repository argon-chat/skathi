---
title: Features and size
label: Features and size
description: Every Cargo feature of the skathi crate, what the app loses with it off, and what it costs in exe size, crates and build-time downloads. With feature sets for a UI-only program, a media app and a background tool, and a CI check of what you linked.
section: Embedding
order: 8.5
---

The default features give your exe everything Skathi's own host has. Most programs use a fraction of it: a settings window needs no video player, a sync client no WebGPU. Each subsystem is a Cargo feature of `skathi`, and turning one off takes its crates out of the build, its downloads and native compiles out of `cargo build`, and its code out of the exe.

```toml title="Cargo.toml"
[dependencies]
skathi = { version = "0.4", default-features = false, features = ["prod", "types", "storages", "clipboard"] }
```

`default-features = false` drops everything; list what the app uses. Features are additive across your dependency graph, so one crate that depends on `skathi` with the defaults turns them back on for all.

## When a feature is off (since 0.4)

Nothing pretends. An app built for a browser checks what exists, and Skathi answers truthfully:

- **A web API is absent.** `typeof localStorage === "undefined"`, `navigator.clipboard` is `undefined`, `AudioContext` and `navigator.gpu` do not exist. Feature detection written for browsers takes its fallback path.
- **An element is an empty box.** `<video>`, `<lottie>`, `<sk-sticker>` and `<iframe>` keep their CSS box, so layout does not shift, and draw nothing. The first one the app creates prints one console warning that names the feature, for example:

  ```text
  [skathi compat] <video> plays nothing: this app's host was built without Skathi's `video` feature.
  Enable it in the `skathi` dependency's features to use <video>.
  ```

  `<video>` also reads as a media element with nothing loaded (`readyState` 0, `canPlayType()` returns `""`), and `<iframe>` fires `error`.

- **An app API says it is unavailable.** `appHotkeys.available` is `false`, as on a platform without global hotkeys, and `setBindings` throws `NotSupportedError`.

Nothing panics, and the same payload runs on a full host and on a trimmed one.

## The features

Sizes are the release exe of a minimal embedder on Windows x64, a `main` that spawns the app and waits, built with the size profile from [Shipping](/embedding/shipping/#binary-size): opt-level `"s"` with the Skathi crates at 3, thin LTO, `rust-lld`. "Exe" is what removing that one feature from the defaults takes off the exe (or, for the features that are off by default, what turning it on does); "Crates" is how many packages leave `cargo tree` for Windows, build dependencies included. 1 MB is 10⁶ bytes.

With every default feature on, the exe is **101.5 MB** and the graph **460 crates**. With all of them off but `prod`, it is **84.9 MB** and **354 crates**; add `icu-root` and `no-devtools` and it is **74.0 MB**.

| Feature           | Default | What the app gets                                                                           | With it off                                                                             | Exe     | Crates | At build time                                                                         |
| ----------------- | ------- | ------------------------------------------------------------------------------------------- | --------------------------------------------------------------------------------------- | ------- | ------ | ------------------------------------------------------------------------------------- |
| `prod`            | on      | Runs a payload                                                                              | The exe boots the dev output in `SK_DIST` ([dev loop](/embedding/dev-loop/))            |         |        |                                                                                       |
| `types`           | on      | `sk types` reads your native modules' TypeScript from the exe                               | Modules work; `sk types` says the exe was built without it                              | −0.7 MB | −5     | specta                                                                                |
| `audio`           | on      | [Web Audio](/web-platform/web-audio/), `navigator.mediaDevices`, `getUserMedia`             | All three are absent                                                                    | −5.1 MB | −57    | firewheel, cpal, symphonia                                                            |
| `spatial-audio`   | on      | [Spatial audio](/web-platform/spatial-audio/): `panningModel = "spatial"` on Windows        | `"spatial"` is ignored and `AudioContext.spatialAudio` is absent                        | −0.1 MB | −1     | none                                                                                  |
| `storages`        | on      | [localStorage](/web-platform/local-storage/), sessionStorage, `appStorage`, cookies on disk | `localStorage` is undefined, `appStorage` inert; cookies and IndexedDB last for the run | −1.1 MB | −1     | none                                                                                  |
| `clipboard`       | on      | [`navigator.clipboard`](/web-platform/async-clipboard/)                                     | `navigator.clipboard` is undefined; copy and paste in text fields still work            | −0.2 MB | −1     | none                                                                                  |
| `indexeddb`       | on      | [IndexedDB](/web-platform/indexed-db/) and `navigator.storage`, encrypted SQLite on disk    | `indexedDB` is undefined                                                                | −1.4 MB | 0      | with `storages`: downloads SQLite3 Multiple Ciphers and compiles it (C)               |
| `webgpu`          | on      | [WebGPU](/web-platform/webgpu/)                                                             | `navigator.gpu` is undefined                                                            | −5.3 MB | −26    | wgpu                                                                                  |
| `native-dx12`     | on      | Windows: the Direct3D 12 renderer                                                           | Direct3D 11 renders instead; every API stays                                            | −0.1 MB | 0      | none                                                                                  |
| `lottie`          | on      | `<lottie>` and `<sk-sticker>` (Lottie JSON, `.tgs`, and WebM with `video`)                  | Both are empty boxes and warn once                                                      | −2.9 MB | −9     | downloads a prebuilt ThorVG (C++), cached; compiles it when the download fails        |
| `video`           | on      | [`<video>`](/web-platform/video/) through the OS player; WebM stickers with `lottie`        | `<video>` is an empty box and warns once; a WebM sticker fires `error`                  | −0.6 MB | −3     | with `lottie`: compiles libvpx's VP9 decoder (C)                                      |
| `hotkeys`         | on      | [`appHotkeys`](/app-api/app-hotkeys/) and `skathi::hotkeys`                                 | `appHotkeys.available` is `false`; `skathi::hotkeys` does not exist                     | −0.3 MB | −1     | none                                                                                  |
| `webview`         | on      | `<iframe>`, a document in WebView2 under the app                                            | `<iframe>` is an empty box, fires `error` and warns once                                | −0.1 MB | −4     | webview2-com                                                                          |
| `icu-root`        | off     | Links only ICU's root locale; the rest can ship as `icu.skpk`                               | Every locale is in the exe                                                              | −9.1 MB | 0      | writes `icudt<NN>l.dat` (12 MB) beside the exe, for `sk build` to cut `icu.skpk` from |
| `no-devtools`     | off     | Compiles out the devtools protocol, the inspector and F12                                   | Present; they start off for a payload published without devtools                        | −1.6 MB | 0      | none                                                                                  |
| `battery`         | off     | The Battery Status API                                                                      | `navigator.getBattery` is absent                                                        |         |        | none                                                                                  |
| `runtime-shaders` | off     | macOS: Metal shaders compiled at start, for a machine without Xcode's Metal toolchain       | Shaders are compiled at build time                                                      |         |        | none                                                                                  |

Removed together, the optional features save 16.5 MB, a little less than the column adds up to: some code serves more than one of them.

What every build needs whatever the features: the JavaScriptCore prebuilt (a download of about 1.6 GB unpacked on Windows, cached), mimalloc, and on Windows the MSVC tools and `clang-cl`. None of the features above adds CMake, nasm or another toolchain: ThorVG, libvpx and SQLite are compiled by `cc` with the C and C++ compiler the build already needs.

On a clean target directory, with the JavaScriptCore prebuilt and the ThorVG archive already in their caches, the release build of that embedder took 3 min 39 s with the defaults and 2 min 36 s with everything off but `prod`, on a 24-thread machine. The second one fetches and compiles no ThorVG, libvpx or SQLite: their crates are not in the graph, and without `indexeddb` the engine's build script skips SQLite.

### Platform notes

- **`native-dx12`** only does something on Windows; elsewhere it changes nothing.
- **`spatial-audio`** renders objects on Windows only, through the spatial sound format the user turned on for the device; on macOS and Linux, and without such a format, `"spatial"` panners render with HRTF and `spatialAudio.reason` says why.
- **`webview`**: `<iframe>` has a backend on Windows only (WebView2, which ships with Windows 11 and current Windows 10). On macOS and Linux it is an empty box with or without the feature.
- **`video`** decodes with the OS: Media Foundation on Windows, AVFoundation on macOS, GStreamer on Linux, loaded at run time so the exe starts without it. The codecs are the machine's, not the feature's.
- **`storages`** on Linux brings the Secret Service client, and on macOS the Keychain and `ring`.
- **`hotkeys`** on macOS needs the Accessibility permission at run time.

The numbers in the table are measured on Windows. The crates behind a feature are the same on every platform, so what turns off is the same; the sizes are not.

## What stays

Some parts look optional and are not features, because the core needs them or they cost too little to be worth a switch:

- **`--shading` effects** (sk-effects): the effect registry is part of the CSS engine, and its shaders are checked in as text (about 160 KB of source); nothing is compiled at build time.
- **Image downscaling** (sk-resample): `<img>` uses it to keep large images small in memory.
- **The tray, single instance, deep links, file dialogs, `appShell`** (sk-desktop) and **permissions** (sk-permissions): the embedding API is built on them, and on Windows they add no crate beyond the `windows` bindings the renderer already uses.
- **The SDK's own text** (sk-locale): a few KB of strings for the consent and crash dialogs.

## Recipes

### A UI-only embedder

Forms, lists, settings, a dashboard: the app is HTML and CSS, the logic is in your Rust. Keep storage so the app remembers its state, and the clipboard for copy buttons:

```toml title="Cargo.toml"
[dependencies]
skathi = { version = "0.4", default-features = false, features = ["prod", "types", "storages", "clipboard"] }
```

For the release build, also `icu-root` (if the app formats in one language, or ships `icu.skpk`) and `no-devtools`. Measured: 86.0 MB and 361 crates as listed, 75.2 MB with those two.

`no-devtools` and `"publishDevtools"` in the app's `sk.config.json` are separate switches. A payload published without devtools (the default) starts a host that has them with the devtools off and no port open, but `SK_DEVTOOLS=1` or `appDevtools.enable()` can still turn them on. Only a host built with `no-devtools` has none to turn on. See [release builds](/devtools/release-builds/#release-builds).

### A media-heavy app

A player, a chat client with stickers, a visualiser. Start from the defaults and drop what the app does not touch:

```toml title="Cargo.toml"
[dependencies]
skathi = { version = "0.4", default-features = false, features = [
    "prod", "types", "storages", "clipboard", "indexeddb",
    "audio", "video", "lottie", "webgpu", "native-dx12",
] }
```

That is 101.1 MB and 454 crates, close to the defaults: the media engines are most of what is optional. `video` without `lottie` gives `<video>` and leaves libvpx out: it only decodes WebM stickers. If your own Rust renders the video (libmpv, a game engine), you may not need `video` at all: [`<sk-surface>`](/embedding/native-frames/) works in every feature set.

### A headless-ish tool

A background service with a tray icon, global hotkeys, and a window that opens now and then. Start it hidden with `Builder::visible(false)` and show it from the tray:

```toml title="Cargo.toml"
[dependencies]
skathi = { version = "0.4", default-features = false, features = ["prod", "types", "storages", "hotkeys", "native-dx12"] }
```

86.2 MB and 361 crates. The tray needs no feature: it is part of the embedding API. See [Window, tray and hotkeys](/embedding/desktop/).

## Check in CI what you linked

A dependency added later can turn a feature back on, since features unify across the graph. Two `cargo tree` commands catch it.

What `skathi` was built with:

```sh
cargo tree -e features -i skathi --target x86_64-pc-windows-msvc
```

Fail the build when a subsystem you dropped is back. `cargo tree -i` exits with an error when the crate is not in the graph:

```sh title="ci/check-features.sh"
#!/bin/sh
status=0
for crate in sk-thorvg-sys sk-vpx-sys sk-video sk-webview wgpu firewheel; do
  if cargo tree -e normal,build -i "$crate" --target x86_64-pc-windows-msvc >/dev/null 2>&1; then
    echo "linked, but should not be: $crate"
    cargo tree -e features -i "$crate" --target x86_64-pc-windows-msvc | head -20
    status=1
  fi
done
exit $status
```

| Crate           | Comes with         |
| --------------- | ------------------ |
| `sk-thorvg-sys` | `lottie`           |
| `sk-vpx-sys`    | `lottie` + `video` |
| `sk-video`      | `video`            |
| `sk-webview`    | `webview`          |
| `sk-hotkeys`    | `hotkeys`          |
| `sk-audio`      | `spatial-audio`    |
| `sk-storage`    | `storages`         |
| `sk-clipboard`  | `clipboard`        |
| `wgpu`          | `webgpu`           |
| `firewheel`     | `audio`            |
| `specta`        | `types`            |

The second half of the check is the exe itself: record its size in CI and fail on a jump you did not expect.

Next: [Shipping](/embedding/shipping/).
