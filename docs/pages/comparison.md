---
title: Skathi vs Electron vs Tauri
label: Comparison
description: Skathi vs Electron vs Tauri 2. App size, memory, processes, start time and the cost of a native call, measured with the same app on one machine.
section: Getting started
order: 0.5
since: "0.4"
---

Electron ships Chromium and Node.js inside every app. Tauri puts a web UI in the WebView the OS provides and gives it a Rust backend in another process. Skathi runs Vue or Svelte in JavaScriptCore and draws it with its own renderer, in one process with your Rust code.

The measured tables use the same Vue app on one Windows machine. Bold values mark the lowest result in each numeric row of the three-framework tables, including ties. The platform and tooling tables draw on each project's documentation. [Sources](#sources)

## When to pick which

- **Skathi** for desktop apps written in Vue or Svelte that look and behave the same on Windows, macOS and Linux. One renderer and one JavaScript engine everywhere, one process per app with every window sharing its state, Rust called in the same process, native GPU content inside the UI, and a Rust program that can embed the whole UI. It is desktop-only and implements a documented subset of the web platform.
- **Electron** when you need all of Chromium and Node.js. Each install carries the browser, and each window gets a renderer process.
- **Tauri** when the app must also run on iOS and Android, or a small download matters most and your users already have WebView2. The app uses the WebView available on each OS and crosses a process boundary to call Rust.

## Measured

On November 22, 2026, each framework ran a 760 × 520 window with a heading, a counter button and 1,000 list rows. Twenty seconds after signalling its first frame, each app scrolls to the end on a timer. The main Skathi column uses a release JIT host built with the features this app needs. The other Skathi builds and runtime settings appear below.

<div class="wide leaders">

| Size on Windows                           | Skathi       | Electron               | Tauri 2                                  |
| ----------------------------------------- | ------------ | ---------------------- | ---------------------------------------- |
| What runs                                 | One exe      | A folder with 73 files | One exe and WebView2                     |
| On disk                                   | **73.8 MiB** | 357.0 MiB              | 882.1 MiB                                |
| Download on a machine without the runtime | **29.8 MiB** | 155.0 MiB              | 282.2 MiB                                |
| With the runtime fixed to one version     | —            | —                      | 682.2 MiB on disk, 298.2 MiB to download |

</div>

Sizes use the release files and their ZIPs. Tauri's app is 4.1 MiB (1.7 MiB zipped, 1.32 MiB as an NSIS installer). Evergreen WebView2 157.0.4341.60 adds 878.0 MiB on this machine, shared by all apps that use it. For a machine that needs the runtime, the download row includes Microsoft's 280.5 MiB standalone x64 installer retrieved on November 22. The fixed-version row uses the bundled runtime's extracted files and CAB. [WebView2 distribution options](https://v2.tauri.app/distribute/windows-installer/)

<div class="wide leaders">

| Running, median of 5                 | Skathi        | Electron  | Tauri 2      |
| ------------------------------------ | ------------- | --------- | ------------ |
| Processes                            | **1**         | 4         | 7            |
| First frame with content             | **312 ms**    | 388 ms    | 522 ms       |
| Private working set, idle            | **57.0 MiB**  | 88.8 MiB  | 95.2 MiB     |
| Private working set, after scrolling | **57.1 MiB**  | 89.9 MiB  | 98.2 MiB     |
| Private commit, idle                 | **163.9 MiB** | 167.8 MiB | 180.8 MiB    |
| Working set, all processes           | **116.4 MiB** | 311.6 MiB | 363.8 MiB    |
| Dedicated GPU memory, idle           | 23.0 MiB      | 22.7 MiB  | **19.9 MiB** |
| Idle CPU before scrolling            | 0.31%         | **0.16%** | 0.62%        |
| Idle CPU after scrolling             | **0.16%**     | 0.31%     | 0.31%        |

</div>

Each figure includes the app and its child processes. CPU is percent of one logical core. At this sample length a single Windows accounting tick moves the reading by about 0.16 percentage points.

### Skathi memory and idle modes

<div class="wide">

| Idle profile, median of 3                          | On disk  | First frame | Private working set | Private commit | CPU   |
| -------------------------------------------------- | -------- | ----------- | ------------------- | -------------- | ----- |
| JIT, default memory                                | 73.8 MiB | 312 ms      | 56.7 MiB            | 163.5 MiB      | 0.16% |
| JIT, low memory requested                          | 73.8 MiB | 328 ms      | 57.1 MiB            | 164.0 MiB      | 0.00% |
| JIT, idle frame pacing off                         | 73.8 MiB | 358 ms      | 57.4 MiB            | 163.9 MiB      | 3.75% |
| AOT, engine-default capacities                     | 77.5 MiB | 359 ms      | 51.2 MiB            | 536.1 MiB      | 0.16% |
| AOT, compact capacities                            | 77.5 MiB | 308 ms      | 51.1 MiB            | 203.4 MiB      | 0.31% |
| AOT, compact + low requested                       | 77.5 MiB | 338 ms      | 51.2 MiB            | 204.2 MiB      | 0.47% |
| AOT, compact + low requested, JIT and Wasm removed | 63.7 MiB | 343 ms      | 50.9 MiB            | 203.8 MiB      | 0.00% |
| JIT, full SDK features                             | 99.9 MiB | 481 ms      | 88.8 MiB            | 216.8 MiB      | 0.31% |

</div>

Skathi selects `compact` when `SK_AOT_MEMORY_PROFILE` is unset. On the regular AOT host, this saved 332.7 MiB of private commit compared with engine-default capacities. The median resident private working set was 51.2 MiB with engine-default capacities and 51.1 MiB with compact capacities. Both settings ran the same executable and native image. The trimmed build uses a separately linked engine. Its ZIP is 25.3 MiB including the image DLL. [AOT builds](/embedding/shipping/#leave-out-the-jit-and-webassembly).

**Low memory depends on the entry point in this revision.** The embedded hosts above use `skathi::Builder::spawn()`. Their startup skips the allocator setup for `SK_MEMORY=low`. For a control, the SDK's standalone `sk-app` executable loaded the same production payload with the same minimal features. Its `run_exe()` startup applies the memory setting before loading the renderer. [Memory settings](/guides/memory/).

<div class="wide">

| Standalone JIT, median of 3 | Private working set | Private commit |
| --------------------------- | ------------------- | -------------- |
| Default memory              | 57.4 MiB            | 163.8 MiB      |
| Low memory                  | 56.8 MiB            | 132.2 MiB      |

</div>

### Adaptive effects under power saving

After the list finished scrolling, the full JIT host rendered one animated `noise` effect over the list. Devtools supplied the scheme and pressure inputs and verified the quality tier in a painted frame. The test counted host frames and process CPU time over ten seconds per tier, rotating the order over three rounds. Windows used its Ultimate Performance plan throughout.

<div class="wide">

| Emulated scheme | CPU pressure | Painted tier | Host frames/s | CPU, one core |
| --------------- | ------------ | ------------ | ------------- | ------------- |
| Balanced        | Nominal      | Full         | 78.7          | 100.30%       |
| Saving          | Nominal      | Reduced      | 31.0          | 37.42%        |
| Saving          | Critical     | Minimal      | 18.8          | 21.25%        |

</div>

The static list has no animated shader. With idle frame pacing enabled it used 0.16% of one core, compared with 3.75% with pacing disabled. [Adaptive effects](/guides/adaptive-effects/).

### Native calls

Each sample makes 10,000 serial calls to `add(a, b)`, then sends the length test a 1 MiB buffer 50 times. Skathi calls Rust directly. Electron and Tauri await one IPC reply before sending the next call.

<div class="wide leaders">

| Native call, median of 5 | Skathi                  | Electron                     | Tauri 2                     |
| ------------------------ | ----------------------- | ---------------------------- | --------------------------- |
| Where the call runs      | **In the same process** | In the main process over IPC | In the app process over IPC |
| `add(a, b)`, per call    | **0.054 µs**            | 117.8 µs                     | 995.9 µs                    |
| 1 MiB to native code     | **<0.001 ms**           | 1.18 ms                      | 9.46 ms                     |

</div>

The Skathi column uses a native module with a borrowed `&[u8]` argument. Copying into a `Vec<u8>` took 0.015 ms. With the lower-level native API, the addition took 0.036 µs and reading the byte length took 0.002 ms. Electron sends a structured clone of a `Uint8Array`. Tauri sends a raw binary body. [Native modules](/embedding/native-modules/), [native API](/embedding/native-api/).

### How it was measured

- **Machine.** Windows 11 Pro Insider build 29683, Ryzen 9 9950X (16 cores, 32 threads), 96 GB of RAM, GeForce RTX 5070 and RTX 3070 Ti (driver 32.0.16.1692), 2560×1440 primary display. Windows used the Ultimate Performance power plan throughout.
- **Builds.** Skathi 0.7 development revision `2a884cbe`, Electron 45.3.3 packaged with `@electron/packager` 20.3.0, and Tauri 2.12.6 with wry 0.57.2 and Evergreen WebView2 157.0.4341.60. All three UI fixtures pin Vue 3.5.45. The web frontends use Vite 8.3.8.
- **Skathi.** The production Vue payload uses invariant locale data. A Rust executable embeds it with Cargo default features disabled and `prod`, `icu-root` and `no-devtools` enabled. The build uses the SDK's size-oriented release profile and `rust-lld`. The full host enables the default SDK features with the same UI payload. The AOT builds ship the executable and its image DLL. [Embedding features](/embedding/features/), [shipping](/embedding/shipping/).
- **Runs.** Apps take turns in each round. For window measurements the runner opens and closes each app once to warm its caches, then records five rounds of the three-framework comparison and three rounds each of the Skathi profiles and standalone allocator controls. Native calls run in five rounds. The runner checks the result of every addition and byte-transfer batch before accepting its timing.
- **First frame.** A stopwatch starts immediately before process launch. After mounting, each app sets its window title in its second animation-frame callback. The runner polls visible windows belonging to the launched PID with a requested 1 ms interval.
- **Memory.** At five seconds from launch, the runner sums the process tree's private working set (resident private pages), private commit (private bytes) and total working set. Total working set counts shared pages once per process. Dedicated GPU memory comes from Windows' per-PID GPU counters. The framework comparison repeats the sample three seconds after the app signals the end of scrolling.
- **CPU.** After each memory sample the runner measures the tree's kernel and user CPU time over ten seconds. One 15.6 ms accounting tick is about 0.16% of one logical core. A displayed zero means no CPU tick accrued during that interval. Window and native-call runs start below 10% system-wide CPU. The runner retries if average system CPU reaches 20%. For the 15 accepted framework launches, average system CPU ranged from 6.7% to 15.2%. All 15 windows were in the background at the five-second sample.
- **Reproduction.** The repository contains the apps, lockfiles, build commands, Windows runner, size calculator and per-run JSONL. The November 17 samples used in the earlier 0.4 comparison are archived alongside this run. [Benchmark instructions](https://github.com/argon-chat/skathi/tree/master/benches/comparison), [November 22 samples](https://github.com/argon-chat/skathi/tree/master/benches/comparison/results/2026-11-22).

## Rendering and platforms

<div class="wide leaders">

|                                         | Skathi                                                                         | Electron                                                       | Tauri 2                                          |
| --------------------------------------- | ------------------------------------------------------------------------------ | -------------------------------------------------------------- | ------------------------------------------------ |
| What draws the UI                       | Skathi's renderer<sup>1</sup>                                                  | Chromium, a copy in every app                                  | The OS WebView<sup>2</sup>                       |
| JavaScript engine                       | JavaScriptCore, one revision on every OS                                       | V8, in every app                                               | V8 on Windows, JavaScriptCore on macOS and Linux |
| Engine version chosen by                | ✓ The app                                                                      | ✓ The app, a new Chromium major every 8 weeks<sup>3</sup>      | ✗ The OS<sup>4</sup>                             |
| Same result on Windows, macOS and Linux | ✓ The rendering; OS services vary<sup>5</sup>                                  | ✓ One Chromium runtime per app                                 | ✗ Three engines to test<sup>6</sup>              |
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
7. Flexbox, grid, Canvas 2D, WebGPU, `fetch`, WebSocket and IndexedDB. CSS can select lighter effects from the system power profile or CPU pressure. [Web platform](/web-platform/), [CSS](/css/), [adaptive effects](/guides/adaptive-effects/). No Web Workers, WebGL, WebRTC or Shadow DOM.
8. Each WebView's own: WebGPU, for one, is partial in Safari.
9. D3D11, D3D12 and shared Vulkan textures on Windows, Metal on macOS, CPU frames on Linux; composited like any element, with the UI on top.
10. The experimental `sharedTexture` module (Electron 40 and later) turns a shared texture into a `VideoFrame` for a canvas or WebGPU.

</div>

## JavaScript and native code

The question is whether the UI's code calls native code in its own process or sends a message to another one. The measured cost is in the [native call](#native-calls) table.

<div class="wide leaders">

|                                  | Skathi                                                             | Electron                                      | Tauri 2                                         |
| -------------------------------- | ------------------------------------------------------------------ | --------------------------------------------- | ----------------------------------------------- |
| Native language                  | Rust, optional<sup>1</sup>                                         | JavaScript in Node.js, C++ in Node-API addons | Rust                                            |
| Calling it from the UI           | **✓ In process**<sup>2</sup>                                       | IPC, a structured clone<sup>3</sup>           | IPC, JSON or raw bytes<sup>4</sup>              |
| Types across the boundary        | **✓ `sk types`** writes the app's `.d.ts` from the Rust signatures | Written by hand for the preload API           | tauri-specta, a release candidate outside Tauri |
| Embedding in an existing program | **✓ A Rust crate**<sup>5</sup>                                     | ✗ Electron is the program                     | Partial<sup>6</sup>                             |

</div>

<div class="table-notes">

1. A Rust program can embed Skathi. Apps can also opt into the permission-gated `skathi:ffi` module to call a C library they ship. [Embedding](/embedding/overview/), [standard library](/std/).
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
| What a build needs    | **Bun**<sup>3</sup>                                                      | **Node.js**                                                              | Rust, MSVC on Windows, and Node.js                              |
| Installer and updates | Yours: `sk publish` makes the exe, or a `.app` on macOS                  | ✓ Electron Forge or electron-builder; `autoUpdater` on macOS and Windows | ✓ MSI, NSIS, DMG, AppImage, deb and rpm; an updater plugin      |
| License               | MIT or Apache-2.0                                                        | MIT                                                                      | MIT or Apache-2.0                                               |
| Releases              | 0.x: APIs can still change between minor releases                        | A major every 8 weeks; the latest three are supported                    | 2.x since October 2024                                          |

</div>

<div class="table-notes">

1. npm, Vue and Svelte libraries and Tailwind CSS work, except code that needs a browser feature the engine lacks; `sk check` flags unsupported CSS and framework features, in the app and its packages.
2. Elements, styles, console, network, storage and a frame profiler.
3. `sk publish` stamps a prebuilt host, so an app needs no compiler.

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
