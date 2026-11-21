---
title: Dev loop and devtools
description: Hot reload inside your own executable. A dev build without the prod feature, sk dev --exe, the inspector on F12, and logs.
section: Embedding
order: 8
---

Rebuilding a payload and restarting your exe for every CSS tweak gets old fast. You do not have to: `sk dev` can run **your** exe instead of its own host, and the app hot-reloads inside it while your native modules, namespaces and surfaces stay right where they are.

## How it works

`sk dev` does two things: it builds the app into a dev output folder and watches the sources, and it starts a host with `SK_DIST` pointing at that folder. A Skathi build **without** the `prod` feature is exactly such a host: it boots whatever `SK_DIST` names, with the hot-reload loader, and takes no payload. Your exe built that way is one too.

## Set it up once

Give your crate a feature that turns `prod` on, on by default:

```toml title="Cargo.toml"
[dependencies]
skathi = { version = "0.4", default-features = false, features = ["types", "audio", "storages", "clipboard", "indexeddb", "webgpu", "native-dx12", "lottie", "video", "hotkeys", "webview"] }

[features]
default = ["release"]
release = ["skathi/prod"]
```

Name the payload only when the feature is on. A dev build given a payload refuses it with `Error::PayloadNeedsProd`:

```rust
let mut builder = skathi::Builder::new()
    .module("notes", Notes::default())
    .setup(install_native_api);
#[cfg(feature = "release")]
{
    builder = builder.payload_file("app/dist/release/.sk/app.skpk");
}
```

## Run it

```sh
cargo build --no-default-features          # a dev build of your exe
cd app && bunx sk dev --exe ../target/debug/notes.exe
```

`sk dev` first asks your exe for its [native modules' types](/embedding/native-modules/types-and-testing/) (`notes.exe --sk-host-types`, which prints them and exits before any window) and writes `app/src/native-modules.d.ts` when they changed. Then it builds the app, starts your exe with the dev output, and watches:

- Edits to `.vue`, `.svelte`, `.ts` and `.css` apply in place, without a restart; component state survives where the framework allows.
- An edit to a `shaders/*.slang` effect recompiles it and reloads it in the running app.
- Your Rust side does not restart. Native modules, the `setup` hook's namespaces and surfaces stay registered across reloads; a reload drops the app's `on` handlers and the task promises still waiting.

A full reload of the page (from devtools, or `location.reload()`) runs your `on_reload` hooks: drop any JS callbacks the old page handed you there, as in [Native APIs](/embedding/native-api/#from-worker-threads-into-the-page).

When you change Rust, stop `sk dev`, rebuild, and start it again: a changed module signature reaches the app's types on that start. `sk types --exe <exe>` refreshes them without running the app, for example in CI before `vue-tsc`.

An exe built against a skathi older than 0.4 has no `--sk-host-types`; `sk dev` sees that and leaves the file alone.

## Devtools

F12 and `appDevtools.open()` start the [inspector](/devtools/overview/) as a separate process, which is normally Skathi's own `sk-app.exe`. In your exe there is none, so point the builder at your own exe and route the flag to the inspector:

```rust
fn main() {
    if std::env::args().nth(1).as_deref() == Some("--devtools") {
        return skathi::run_exe();
    }
    let builder = skathi::Builder::new().inspector_exe(std::env::current_exe().unwrap());
    // …
}
```

`skathi::run_exe()` runs the full standalone host, which here starts as the inspector. Without `inspector_exe`, F12 says why nothing opened; the devtools protocol works either way, so the [devtools command line](/devtools/scripting/) (`sk ctl`) still connects. The `no-devtools` feature compiles both out of a release build.

## Logs and the allocator

The app's `console.*` output goes to the process's stdout and stderr, like the rest of your program's. Start from a terminal to see it, or capture it with your own logging.

The devtools `mem` command counts the Rust heap only when the host's allocator does the counting. Make it your global allocator to get those numbers in your exe:

```rust
#[global_allocator]
static GLOBAL: skathi::HostAllocator = skathi::HostAllocator;
```

It is optional; without it everything works and `mem` reports what it can see.

## In CI

The [testing guide](/guides/testing/)'s end-to-end specs drive a real host. To run them against your exe, build it with `prod`, build the payload with `sk publish --payload-only`, and start your exe from the spec as you start any program. `SK_WINDOW_FOCUS=never` keeps its window from taking the foreground on a developer's machine.

Next: [Shipping](/embedding/shipping/).
