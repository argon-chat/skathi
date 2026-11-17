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

- `type annotations needed` at `.module("…", |m| …)`

  **Cause:** `Builder::module` takes a `#[skathi::module]` type or a closure, so Rust cannot infer the closure's parameter.

  **Fix:** Write it `|m: &mut skathi::Module|`.

- `a #[task] runs on a worker after the call has returned, so it cannot borrow from it`

  **Cause:** A `#[task]` method takes `&str` or a slice. Its arguments are read in the call and used later, on a worker.

  **Fix:** Take the owned type the error names: `String`, `Vec<u8>`, `skathi::Bytes`. See [borrowed parameters](/embedding/native-modules/values/#borrowed-parameters).

- `Failed to resolve import "native:…"` from Vite or vitest

  **Cause:** Only Skathi's builders know the `native:` scheme; a Vite web build or a vitest run of the same code does not.

  **Fix:** Add the [stand-in plugin](/embedding/native-modules/types-and-testing/#the-same-code-in-a-web-build) to that tool's config.

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

- `native:notes: "delete" is not a usable export name`, `… is registered twice`, `event … is declared twice`

  **Cause:** A module function or event name the app could not import: a JS keyword, `available`, `on`, `off`, `default` or `then`, or a name used twice. `spawn` returns it as `Error::Module` before any window.

  **Fix:** Rename it in Rust; the JS side imports it by that name.

### While it runs

- The window freezes while a native function runs

  **Cause:** A module method runs on the Skathi thread, so the frame waits for it.

  **Fix:** Mark it `#[task]` (`m.task` in the closure form): it runs on a worker and the app gets a promise. See [Tasks](/embedding/native-modules/errors-and-tasks/#tasks-do-the-slow-work).

- `native:notes.… is provided by the program that embeds this app, and this app is not embedded`

  **Cause:** The app called a module function in Skathi's own host, `sk dev` without `--exe`, or a browser preview.

  **Fix:** Check `available` before calling, or run the app in your exe. See [One app, with or without a program](/embedding/native-modules/#one-app-with-or-without-a-program).

- `the program that embeds this app has no native module "…"`

  **Cause:** The exe declares no module of that name: a typo, or an exe built before the module was added.

  **Fix:** Match the name in `import … from "native:…"` with `Builder::module`, and rebuild the exe.

- `Cannot find module 'native:notes'` in the type check

  **Cause:** `src/native-modules.d.ts` is missing or older than the exe.

  **Fix:** Run `sk types --exe <your exe>`, or start the app with `sk dev --exe`, and commit the file.

- `sk types` says the exe `does not answer --sk-host-types`

  **Cause:** The exe was built against a skathi older than 0.4. One built without the `types` feature answers, but says so and fails.

  **Fix:** Build it against 0.4 or later, with `types` on.

- `sk types` opens a log, starts crash reporting or registers the app with the OS

  **Cause:** The exe answers `--sk-host-types` in `spawn`, and everything `main` does before it still runs.

  **Fix:** Check for the flag at the top of `main` and build only the modules. See [answer sk types before anything else](/embedding/native-modules/types-and-testing/#answer-sk-types-before-anything-else).

- An event arrives in one component but not in another, or not after a reload

  **Cause:** An event is delivered to the handlers subscribed when it arrives, and dropped when there are none. A component that mounted later missed it.

  **Fix:** Pair the event with a function that returns the current state, and read it right after subscribing. See [state plus an event](/embedding/native-modules/events/#state-plus-an-event).

- A task rejects with `the task was dropped before it ran`

  **Cause:** The executor given to `task_executor` dropped the job instead of running it, often because its runtime shut down.

  **Fix:** Run every job the executor receives, or keep the runtime alive as long as the app.

- `emit_to` returns `no native module declares that event`

  **Cause:** No module of that name declares the event, or one of the two names has a typo.

  **Fix:** Declare it on the module you emit to: `events(name = Type)` in `#[skathi::module]`, or `m.event`.

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
