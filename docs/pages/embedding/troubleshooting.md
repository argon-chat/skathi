---
title: Troubleshooting embedding
label: Troubleshooting
description: The errors an embedded Skathi reports at build time, at start and while running, what causes each, and the fix.
section: Embedding
order: 10
---

<div class="fixes">

### When it builds

- The redjs build stops and asks for the static CRT

  **Cause:** The exe links the dynamic C runtime; JavaScriptCore is built against the static one.

  **Fix:** Add `rustflags = ["-C", "target-feature=+crt-static"]` for `x86_64-pc-windows-msvc` in `.cargo/config.toml`.

- `clang-cl` not found

  **Cause:** Parts of the native build compile with LLVM's MSVC-compatible driver.

  **Fix:** Install LLVM and put its `bin` on `PATH`.

- The first build takes long and downloads gigabytes

  **Cause:** The pinned JavaScriptCore prebuilt, about 1.6 GB unpacked on Windows, plus mimalloc, SQLite and ThorVG.

  **Fix:** Nothing to fix: it happens once per machine. On CI, cache the folders `REDJS_CACHE_DIR` and `SK_THORVG_CACHE_DIR` point at.

- Windows refuses to start the exe

  **Cause:** The manifest with Common Controls 6 is missing: `build.rs` does not compile `gpui.rc`, or your own manifest dropped the dependency.

  **Fix:** Use the [build script](/embedding/first-embedder/#4-the-build-script), or add Common Controls 6 to your manifest.

- A stack overflow on deep recursion in JavaScript

  **Cause:** The link arguments from `DEP_SKATHI_LINK_ARGS`, which set the 8 MiB stack, did not reach the exe.

  **Fix:** Pass every one of them with `cargo:rustc-link-arg`.

### When it starts

- `no app: give the builder a payload`

  **Cause:** A `prod` build with no `payload_file`, `payload_bytes` or `payload_static`, and no `SK_PAYLOAD`.

  **Fix:** Name the payload, or set `SK_PAYLOAD=<path to app.skpk>`. `sk publish --payload-only` writes it to `dist/release/.sk/app.skpk`.

- `a payload needs sk-app's prod feature; a dev build boots the dev dist instead`

  **Cause:** A build without `prod` was given a payload.

  **Fix:** Build with `prod` to run the payload, or leave the payload out and start the exe through [`sk dev --exe`](/embedding/dev-loop/).

- `this app was built by Skathi … and this library is …`

  **Cause:** The payload was built by an `sk` of another version than the crate.

  **Fix:** Use the CLI of the crate's version: `@skathi-native/cli` and `skathi` are released together.

- `Skathi already ran in this process; it runs once`

  **Cause:** A second `spawn` in the same process. The JS realm and gpui cannot be torn down and started again.

  **Fix:** Start Skathi once, and keep the `Handle` for the life of the process. To bring the UI back, hide the window instead of closing the app.

- `Skathi's arguments were read before spawn`

  **Cause:** Another Skathi call read the command line before `spawn`.

  **Fix:** Start Skathi before anything else of it runs. Pass the app's own arguments with `Builder::args`.

- `data directory: data directory must be absolute: …`

  **Cause:** `data_dir` or `SK_DATA_DIR` is a relative path.

  **Fix:** Resolve it first, for example against the user's data folder.

- `desktop launch finished with exit code 0`

  **Cause:** Not a failure: with `desktop_launch(true)` this launch handed its arguments to the running instance.

  **Fix:** Return the code from `main`; see [Single instance and deep links](/embedding/desktop/#single-instance-and-deep-links).

- `AppKit requires the process main thread; call Builder::run_on_main_thread from main`

  **Cause:** `spawn` on macOS.

  **Fix:** Use [`run_on_main_thread`](/embedding/shipping/#macos-owns-the-main-thread).

- `setup` returned an error and no window opened

  **Cause:** An `Err` or a panic in `Builder::setup` stops startup by design.

  **Fix:** The error text is in `spawn`'s `Err`. A failed `expose` usually means the global already exists or starts with `__sk`.

### While it runs

- The window freezes while a function runs

  **Cause:** The handler does its work on the Skathi thread.

  **Fix:** `call.into_reply()`, move the `Reply` to a worker and answer from there. See [Keep the UI thread free](/embedding/functions-and-events/#keep-the-ui-thread-free).

- An `AppHostError` the handler never sent

  **Cause:** The `Reply` was dropped without an answer, the handler answered with text that is not JSON, or a reload rejected the calls still waiting.

  **Fix:** Answer every `Reply` on every path, errors included, with `serde_json` output.

- `appHost.call: the embedder has no function "…"`

  **Cause:** The name is not registered, often a typo or a function registered after `spawn`.

  **Fix:** Register every function on the `Builder`. The message lists the names that exist.

- `emit` returns `the event's data is not JSON`

  **Cause:** `data` is not one JSON value; a bare string needs its quotes, as `"\"ready\""`.

  **Fix:** Serialise with `serde_json`.

- `dispatcher.post` returns `Full`

  **Cause:** 1024 tasks are waiting: workers produce faster than the UI takes them.

  **Fix:** Coalesce, drop stale updates, or batch them in an `on_frame` hook.

- A JS callback held by Rust stops working after a reload, or the app crashes in the engine

  **Cause:** A `js::Value` was kept past the page it came from, used unprotected, or moved to another thread.

  **Fix:** Protect what you keep, drop it in `on_reload`, release it in `on_shutdown`, and only touch values on the JS thread.

- `<sk-surface>` stays empty

  **Cause:** Nothing presented to that name yet, the producer skipped frames because `info().visible` is false, or a present returned an error you ignored.

  **Fix:** Check the names match, log the `SurfaceError`, and compare `stats().presented` with `stats().shown`.

</div>
