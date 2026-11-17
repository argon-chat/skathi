---
title: Skathi, Electron and Tauri
label: Comparison
description: Skathi next to Electron and Tauri 2. Size, memory, processes, start time and the cost of a native call, measured with the same app on one machine, then rendering, platforms, native code, tooling and licensing.
section: Getting started
order: 0.5
---

Electron ships Chromium and Node.js inside every app. Tauri puts a web UI in the WebView the OS provides and gives it a Rust backend in another process. Skathi runs Vue or Svelte in JavaScriptCore and draws it with its own renderer, in one process with your Rust code.

The first tables were measured: the same small app, built three times, on one machine. The rest comes from each project's documentation, linked under [Sources](#sources). The best value in each row is highlighted; ties are all highlighted.

## When to pick which

- **Skathi** for desktop apps written in Vue or Svelte that look and behave the same on Windows, macOS and Linux. One renderer and one JavaScript engine everywhere, one process per app with every window sharing its state, Rust called in the same process, native GPU content inside the UI, and a Rust program that can embed the whole UI. It is desktop-only and implements a documented subset of the web platform.
- **Electron** when you need all of Chromium and Node.js. You pay with a 357 MiB app folder, four processes for one window and another renderer process per window, and a new Chromium major to ship every 8 weeks.
- **Tauri** when the app must also run on iOS and Android, or a 2 MiB download matters most and your users already have WebView2 (878 MiB, installed with Windows). You pay with three WebView engines to test, whose versions the OS picks, and a process hop on every call into Rust: 0.9 ms per call here.

## Measured

One app, three times: a window with a heading, a counter button and a scrolling list of 1,000 rows. Skathi builds a Vue single-file component with `sk publish` into a small Rust program that embeds Skathi with only the features the app uses. Electron and Tauri build the same component with Vite. Twenty seconds after its first frame, each app scrolls its list to the end from a timer; nothing touches the window.

Every size is what the app needs to run.

<div class="wide leaders">

| Size on Windows                       | Skathi                   | Electron               | Tauri 2                                  |
| ------------------------------------- | ------------------------ | ---------------------- | ---------------------------------------- |
| What runs                             | one exe                  | a folder with 73 files | one exe and the WebView2 runtime         |
| On disk                               | **70.8 MiB**             | 357.0 MiB              | 882.3 MiB<sup>1</sup>                    |
| Download for a machine without it     | **28.7 MiB**<sup>2</sup> | 155.0 MiB<sup>2</sup>  | 206.9 MiB<sup>3</sup>                    |
| With the runtime fixed to one version | —                        | —                      | 682.2 MiB on disk, 298.2 MiB to download |

</div>

<div class="table-notes">

1. 4.1 MiB of app and 878.2 MiB of WebView2, which is installed system-wide and shared with the other apps that use it.
2. The zipped app, standing for the download.
3. Tauri's zipped app, 1.7 MiB, and WebView2's 205.2 MiB standalone installer. Where WebView2 is already installed, as on most Windows 10 and 11 machines, Tauri's download is 1.7 MiB and its installer 1.33 MiB.

</div>

<div class="wide leaders">

| Running, median of 5            | Skathi                 | Electron  | Tauri 2              |
| ------------------------------- | ---------------------- | --------- | -------------------- |
| Processes                       | **1**                  | 4         | 7 (app + 6 WebView2) |
| First frame with content        | **359 ms**<sup>1</sup> | 392 ms    | 435 ms               |
| Private memory, idle            | **55.1 MiB**           | 86.3 MiB  | 95.3 MiB             |
| Private memory, after scrolling | **56.3 MiB**           | 87.7 MiB  | 98.6 MiB             |
| Working set, all processes      | **112.1 MiB**          | 307.0 MiB | 356.6 MiB            |
| Dedicated GPU memory            | 24.1 MiB               | 22.7 MiB  | **19.9 MiB**         |
| CPU when idle, % of one core    | **0.2**<sup>2</sup>    | 0.5       | 0.5                  |
| CPU when idle after scrolling   | 0.2<sup>2</sup>        | **0.0**   | 0.3                  |

</div>

<div class="table-notes">

Each row counts every process the app runs, WebView2's included.

1. Skathi shows its window only once its first frame is drawn.
2. Windows counts CPU time in 15.6 ms ticks: 0.2 % is one tick in 10 seconds. Skathi's ten samples were 0, 1 or 2 ticks, and once 4.

</div>

A native call, measured inside the same apps: 10,000 calls of `add(a, b)` one after another, then 50 sends of 1 MiB of bytes to native code, which returns the length. Electron and Tauri calls are awaited promises; Skathi calls return directly.

<div class="wide leaders">

| Native call           | Skathi                              | Electron                      | Tauri 2                      |
| --------------------- | ----------------------------------- | ----------------------------- | ---------------------------- |
| Where the call runs   | **in the same process**             | in the main process, over IPC | in the app process, over IPC |
| `add(a, b)`, per call | **0.06 µs**, 0.04 µs<sup>1</sup>    | 109 µs                        | 932 µs                       |
| 1 MiB to native code  | **<0.001 ms**, 0.014 ms<sup>2</sup> | 1.14 ms<sup>3</sup>           | 8.9 ms<sup>3</sup>           |

</div>

<div class="table-notes">

1. 0.06 µs through a [native module](/embedding/native-modules/), 0.04 µs through a [native API](/embedding/native-api/).
2. A `Uint8Array` to a native module function: under 0.001 ms as a `&[u8]` it reads in place, 0.014 ms as a `Vec<u8>`, copied once. A native API reads it in place in 0.002 ms.
3. Electron sends a structured clone of a `Uint8Array`, Tauri an `ArrayBuffer` as a raw body.

</div>

### How it was measured

- **Machine.** AMD Ryzen 9 9950X (16 cores, 32 threads), 96 GB of RAM, GeForce RTX 5070 and RTX 3070 Ti (driver 32.0.16.1692), a 2560×1440 primary display, Windows 11 Pro Insider build 29683.
- **WebView2.** The installed Evergreen runtime 157.0.4341.46 is 878.2 MiB in 918 files. The standalone x64 installer is 205.2 MiB and the bootstrapper 1.8 MiB. The fixed-version runtime 157.0.4341.60 for x64 is a 296.5 MiB cab that unpacks to 678.1 MiB.
- **Versions.** Skathi 0.4 development build, `dc81c4ac`. Electron 45.3.3 (Chromium 156), packaged with `@electron/packager` 20.3.0. Tauri 2.12.6 with wry 0.57.2 on WebView2 157.0.4341.46, NSIS bundle. Vue 3.5.45 in all three apps, and Vite 8.3.8 for the Electron and Tauri ones.
- **Skathi's build.** The app is built with only the features it uses: `sk publish --payload-only` with `"intl": "invariant"` makes the payload, and a Rust `main` that spawns it and waits holds it, built against `skathi` with `default-features = false` and the features `prod`, `icu-root` and `no-devtools` ([features and size](/embedding/features/)). The exe is built in release mode with the size profile from [Shipping](/embedding/shipping/#binary-size) and linked with `rust-lld`; it is one file. With every default feature on, the same app is a 96.9 MiB exe (42.9 MiB zipped) that draws its first frame in 409 ms and uses 86.6 MiB of private memory, a 149.5 MiB working set and 28.1 MiB of GPU memory (median of 3). The native-call numbers come from a second embedder with the same features and profile.
- **Runs.** Each app started six times, in turn: Skathi, Electron, Tauri, then again. The first round filled the disk cache and each engine's own caches and is not counted; the tables show the median of the other five. Each app opened its window without taking focus and was closed by the script. The native calls ran five times per app, in turn as well.
- **Start.** _First frame with content_ runs from process creation until the app has drawn its UI: each app changes the window title in the second `requestAnimationFrame` after mounting, and a script polls the titles of the started process's windows every millisecond.
- **Memory.** Summed over every process in the app's process tree, for every framework: Electron's helpers and Tauri's six `msedgewebview2.exe` processes are counted in each memory, GPU and CPU row. Samples are taken 5 seconds after start and 3 seconds after the scroll ended. Private memory is the private working set that Task Manager shows; the working set adds shared pages, once per process; GPU memory comes from Windows' GPU Process Memory counters.
- **CPU.** CPU time of the process tree over 10 seconds, idle, after each memory sample.
- **Noise.** The machine was otherwise quiet: 2 to 5 % of total CPU before each start, and up to 9 % before a native-call run. A run would have been repeated above 10 %; none had to be. One app's first frames stayed within 36 ms of each other. Each of Electron's and Tauri's native-call numbers stayed within 9 % across runs; Skathi's, a fraction of a microsecond each, varied by up to a third.

## Rendering and platforms

<div class="wide leaders">

|                                         | Skathi                                                                         | Electron                                                       | Tauri 2                                          |
| --------------------------------------- | ------------------------------------------------------------------------------ | -------------------------------------------------------------- | ------------------------------------------------ |
| What draws the UI                       | Skathi's renderer<sup>1</sup>                                                  | Chromium, a copy in every app                                  | The OS WebView<sup>2</sup>                       |
| JavaScript engine                       | JavaScriptCore, one revision on every OS                                       | V8, in every app                                               | V8 on Windows, JavaScriptCore on macOS and Linux |
| Engine version chosen by                | ✓ The app                                                                      | ✓ The app, a new Chromium major every 8 weeks<sup>3</sup>      | ✗ The OS<sup>4</sup>                             |
| Same result on Windows, macOS and Linux | ✓ The rendering; OS services vary<sup>5</sup>                                  | ✓ One Chromium, at 357 MiB per app                             | ✗ Three engines to test<sup>6</sup>              |
| Web platform                            | Partial<sup>7</sup>                                                            | **✓ All of Chromium, plus Node.js**                            | Differs by OS<sup>8</sup>                        |
| UI code                                 | Vue 3 or Svelte 5, with TypeScript                                             | **Any web framework**                                          | Any web framework, on three engines              |
| Native GPU content inside the UI        | ✓ [`<sk-surface>`](/embedding/native-frames/); CPU frames on Linux<sup>9</sup> | Partial, experimental<sup>10</sup>                             | ✗ Only behind or beside a transparent WebView    |
| More windows                            | **✓ [Pop-outs](/guides/pop-out-windows/) share one realm and its state**       | A renderer process per window, state over IPC                  | A WebView per window, events through Rust        |
| Desktop OSes                            | Windows x64, macOS on Apple silicon, Linux x64                                 | Windows 10 and later, macOS 13 and later, Linux; x64 and arm64 | Windows, macOS, Linux                            |

</div>

<div class="table-notes">

1. A Rust element tree, CSS engine and layout on DirectX, Metal or wgpu (Vulkan).
2. WebView2 (Chromium) on Windows, WKWebView (WebKit) on macOS, WebKitGTK on Linux.
3. Only the latest three majors get fixes.
4. WebView2 updates itself, WebKit comes with macOS and WebKitGTK with the distribution.
5. One renderer, CSS engine, JavaScriptCore revision and sRGB colour pipeline on every OS. OS services vary; each [reference](/web-platform/) page lists its platforms.
6. CSS and API support, GPU paths, colour handling and rendering bugs are each engine's own, and WebKitGTK has documented GPU problems on Linux.
7. Flexbox, grid, Canvas 2D, WebGPU, `fetch`, WebSocket, IndexedDB and more, listed under [web platform](/web-platform/) and [CSS](/css/). No Web Workers, WebGL, WebRTC or Shadow DOM.
8. Each WebView's own: WebGPU, for one, is partial in Safari.
9. D3D11, D3D12 and shared Vulkan textures on Windows, Metal on macOS, CPU frames on Linux; composited like any element, with the UI on top.
10. The experimental `sharedTexture` module (Electron 40 and later) turns a shared texture into a `VideoFrame` for a canvas or WebGPU.

</div>

## JavaScript and native code

The question is whether the UI's code calls native code in its own process or sends a message to another one. The measured cost is in the [native call](#measured) table.

<div class="wide leaders">

|                                  | Skathi                                                             | Electron                                      | Tauri 2                                         |
| -------------------------------- | ------------------------------------------------------------------ | --------------------------------------------- | ----------------------------------------------- |
| Native language                  | Rust, optional<sup>1</sup>                                         | JavaScript in Node.js, C++ in Node-API addons | Rust                                            |
| Calling it from the UI           | **✓ In process**<sup>2</sup>                                       | IPC, a structured clone<sup>3</sup>           | IPC, JSON or raw bytes<sup>4</sup>              |
| Types across the boundary        | **✓ `sk types`** writes the app's `.d.ts` from the Rust signatures | Written by hand for the preload API           | tauri-specta, a release candidate outside Tauri |
| Embedding in an existing program | **✓ A Rust crate**<sup>5</sup>                                     | ✗ Electron is the program                     | Partial<sup>6</sup>                             |

</div>

<div class="table-notes">

1. Through the [`skathi` crate](/embedding/overview/). An app built with `sk` needs none.
2. A [native module](/embedding/native-modules/) function runs inside the JavaScript call, its values read and built straight from the engine's, with no JSON, and bytes read in place; a `#[task]` runs on a worker and returns a promise. A [native API](/embedding/native-api/) passes engine values directly.
3. `ipcRenderer.invoke` sends a structured clone to the main process. In process only through a Node-API addon in a renderer with the sandbox turned off, which Electron's security guide advises against.
4. `invoke` sends JSON, or raw bytes, from the WebView's process to the app's and returns a promise.
5. The `skathi` crate runs the UI on its own thread inside your Rust program; `main` stays yours.
6. Tauri owns the main thread's event loop; wry alone can put a WebView in an existing window.

</div>

## Tooling and licensing

<div class="wide leaders">

|                       | Skathi                                                                   | Electron                                                                 | Tauri 2                                                         |
| --------------------- | ------------------------------------------------------------------------ | ------------------------------------------------------------------------ | --------------------------------------------------------------- |
| Libraries             | Partial<sup>1</sup>                                                      | **✓ npm, the web and Node.js**                                           | npm and the web within each WebView's support, plus Rust crates |
| Hot reload            | ✓ Vue templates keep their state; Svelte components hot-update           | ✓ The renderer; main-process changes restart the app                     | ✓ The frontend; Rust changes rebuild and restart the app        |
| Devtools              | The [inspector](/devtools/overview/), no breakpoint debugger<sup>2</sup> | **✓ Chrome DevTools**                                                    | A different WebView inspector on each OS                        |
| What a build needs    | **Bun**<sup>3</sup>                                                      | **Node.js**                                                              | Rust, MSVC on Windows, and Node.js<sup>4</sup>                  |
| Installer and updates | Yours: `sk publish` makes the exe, or a `.app` on macOS                  | ✓ Electron Forge or electron-builder; `autoUpdater` on macOS and Windows | ✓ MSI, NSIS, DMG, AppImage, deb and rpm; an updater plugin      |
| License               | MIT or Apache-2.0                                                        | MIT                                                                      | MIT or Apache-2.0                                               |
| Releases              | 0.x: APIs can still change between minor releases                        | A major every 8 weeks; the latest three are supported                    | 2.x since October 2024                                          |

</div>

<div class="table-notes">

1. npm, Vue and Svelte libraries and Tailwind CSS work, except code that needs a browser feature the engine lacks; `sk check` flags unsupported CSS and framework features, in the app and its packages.
2. Elements, styles, console, network, storage and a frame profiler.
3. `sk publish` stamps a prebuilt host, so an app needs no compiler.
4. 2 min 43 s for this app's first release build.

</div>

## Sources

Checked in November 2026. Skathi's rows come from this site's reference pages and its source.

**Electron**

- Versions, Chromium and Node.js per release: [releases.electronjs.org](https://releases.electronjs.org/), [release timelines](https://www.electronjs.org/docs/latest/tutorial/electron-timelines)
- Processes: [process model](https://www.electronjs.org/docs/latest/tutorial/process-model)
- IPC and the sandbox: [ipcRenderer](https://www.electronjs.org/docs/latest/api/ipc-renderer), [contextBridge](https://www.electronjs.org/docs/latest/api/context-bridge), [process sandboxing](https://www.electronjs.org/docs/latest/tutorial/sandbox), [security](https://www.electronjs.org/docs/latest/tutorial/security)
- GPU textures: [sharedTexture](https://www.electronjs.org/docs/latest/api/shared-texture), [offscreen rendering](https://www.electronjs.org/docs/latest/tutorial/offscreen-rendering)
- Platforms and updates: [README](https://github.com/electron/electron/blob/main/README.md), [autoUpdater](https://www.electronjs.org/docs/latest/api/auto-updater)

**Tauri**

- WebViews and their versions: [WebView versions](https://v2.tauri.app/reference/webview-versions/), [Windows installer and WebView2](https://v2.tauri.app/distribute/windows-installer/), [Linux graphics](https://v2.tauri.app/develop/debug/linux-graphics/), [WebGPU support](https://caniuse.com/webgpu)
- IPC: [inter-process communication](https://v2.tauri.app/concept/inter-process-communication/), [calling Rust](https://v2.tauri.app/develop/calling-rust/), [tauri-specta](https://github.com/specta-rs/tauri-specta)
- Native content: [wry's wgpu example](https://docs.rs/crate/wry/0.55.1/source/examples/wgpu.rs), [WebviewBuilder](https://docs.rs/tauri/latest/tauri/webview/struct.WebviewBuilder.html)
- Mobile, bundles and updates: [Tauri 2.0](https://v2.tauri.app/blog/tauri-20/), [distribute](https://v2.tauri.app/distribute/), [updater](https://v2.tauri.app/plugin/updater/)
