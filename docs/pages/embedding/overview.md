---
title: Embedding Skathi in a Rust program
label: Overview
description: Run a Skathi app inside your own Rust executable with the skathi crate. What embedding gives you, what it costs, how the pieces fit, and where to go next.
section: Embedding
order: 0
---

`sk build` gives you a finished exe: Skathi's own host with your app stamped in. That covers most apps. Sometimes the program comes first, though: a sync engine, a game launcher, a player built around libmpv, a tool with years of Rust behind it. It needs a UI, and it should not become a sidecar process that talks to a webview over a socket.

For that, the [`skathi`](https://docs.rs/skathi) crate puts the whole engine (gpui, JavaScriptCore, the CSS engine, the runtime) inside **your** executable. Your `main` stays yours. The app is the same Vue or Svelte project you would build with `sk`, packed into a payload, and the two halves talk through async functions and events, with no HTTP, IPC or webview in between.

<div class="tradeoffs">
<div class="tradeoff" data-tone="gain">

**What you get**

- **Your process, your `main`.** Your threads, your allocator, your logging, your crash handling. Skathi runs on a thread of its own and hands back a `Handle`.
- **Calls without plumbing.** `appHost.call("db.query", sql)` in the app is a Rust closure in yours. The result comes back as a promise.
- **Native APIs, not just messages.** In `setup` you build real JS objects backed by Rust functions, and post results from worker threads straight into the page.
- **Native frames in the page.** A renderer you own (a video player, a game viewport, another process) draws into `<sk-surface>`, and the app's Vue or Svelte controls sit over it.
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
- **Packaging is yours.** The icon, version resource, installer and signing of your exe are your build's job, not `sk publish`'s.

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

## Three ways to reach the app

From the simplest to the most direct:

|               | Functions and events                                     | Native APIs                                         | Native frames                               |
| ------------- | -------------------------------------------------------- | --------------------------------------------------- | ------------------------------------------- |
| In the app    | `appHost.call(name, …)`, `appHost.on(…)`                 | a global you install, `nativeClock.now()`           | `<sk-surface name="…">`                     |
| In Rust       | `Builder::function`, `Handle::emit`                      | `HostContext::expose`, `js::function`, `dispatcher` | `SurfaceHandle::present_*`                  |
| What crosses  | JSON                                                     | JS values, built on the JS thread                   | pixels or GPU textures                      |
| Sync or async | async: every call is a promise                           | sync calls; results posted from workers             | the newest frame wins                       |
| Good for      | requests, commands, notifications                        | hot calls, streams of data, callbacks               | video, viewports, anything a renderer draws |
| Read          | [Functions and events](/embedding/functions-and-events/) | [Native APIs](/embedding/native-api/)               | [Native frames](/embedding/native-frames/)  |

Start with functions and events. Most embedders never need more, and the app stays easy to run without a host.

## In this section

- [Your first embedder](/embedding/first-embedder/): a Cargo project that runs an app, from an empty folder.
- [Functions and events](/embedding/functions-and-events/): `appHost` on one side, `Builder::function` and `Handle::emit` on the other, and keeping the UI responsive.
- [Native APIs](/embedding/native-api/): the `setup` hook, native namespaces, posting from worker threads, cookies and secrets.
- [Native frames](/embedding/native-frames/) and [GPU frames on Windows](/embedding/gpu-frames/): `<sk-surface>` and the producer's side of it.
- [Recipe: libmpv](/embedding/libmpv/): a video player with Vue controls over the picture.
- [Window, tray and hotkeys](/embedding/desktop/): showing, hiding and closing the app from Rust, the tray, global hotkeys, single instance and deep links.
- [Dev loop and devtools](/embedding/dev-loop/): hot reload inside your own exe.
- [Shipping](/embedding/shipping/): the payload, the profile, binary size, macOS.
- [Troubleshooting](/embedding/troubleshooting/): the errors and what they mean.

## When to embed, and when not to

Embed when the native program is the product and the UI is its face: a player, a launcher, a client with a heavy Rust core, a tool that renders its own content. Build with `sk` when the app is the product: you get the exe, its icon and packaging without owning a Rust build. Both use the same app project, so moving from one to the other later is a change on the Rust side only.
