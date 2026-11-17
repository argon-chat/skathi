---
title: Skathi, Electron, Tauri and Qt
label: Comparison
description: Skathi next to Electron, Tauri 2 and Qt 6. Size, memory, processes, start time and the cost of a native call, measured with the same app on one machine, then rendering, platforms, native code, tooling and licensing.
section: Getting started
order: 0.5
---

Electron ships Chromium and Node.js inside every app. Tauri puts a web UI in the WebView the OS provides and gives it a Rust backend in another process. Qt is a C++ toolkit with its own scene graph and the QML language. Skathi runs Vue or Svelte in JavaScriptCore and draws it with its own renderer, in one process with your Rust code.

The first tables were measured: the same small app, built four times, on one machine. The rest comes from each project's documentation, linked under [Sources](#sources). The best value in each row is highlighted; ties are all highlighted.

## When to pick which

- **Skathi** for desktop apps written in Vue or Svelte that look and behave the same on Windows, macOS and Linux. One renderer and one JavaScript engine everywhere, one process per app with every window sharing its state, Rust called in the same process, native GPU content inside the UI, and a Rust program that can embed the whole UI. It is desktop-only and implements a documented subset of the web platform.
- **Electron** when you need all of Chromium and Node.js. You pay with a 357 MiB app folder, four processes for one window and another renderer process per window, and a new Chromium major to ship every 8 weeks.
- **Tauri** when the app must also run on iOS and Android, or a 2 MiB download matters most and your users already have WebView2 (878 MiB, installed with Windows). You pay with three WebView engines to test, whose versions the OS picks, and a process hop on every call into Rust: 1.2 ms per call here.
- **Qt** when the program is C++, runs on embedded devices, or needs vendor long-term support. You pay with C++ and QML instead of web technology, and LGPL obligations or a commercial license.

## Measured

One app, four times: a window with a heading, a counter button and a scrolling list of 1,000 rows. Skathi builds a Vue single-file component with `sk publish`. Electron and Tauri build the same component with Vite. Qt runs a Qt Quick window with a `ListView` (which creates only the visible rows), built in release mode and deployed with `windeployqt`. Twenty seconds after its first frame, each app scrolls its list to the end from a timer; nothing touches the window.

Every size is what the app needs to run.

<div class="wide leaders">

| Size on Windows                       | Skathi                   | Electron               | Tauri 2                                  | Qt 6                                         |
| ------------------------------------- | ------------------------ | ---------------------- | ---------------------------------------- | -------------------------------------------- |
| What runs                             | one exe                  | a folder with 73 files | one exe and the WebView2 runtime         | a folder with 1,378 files from `windeployqt` |
| On disk                               | **107.1 MiB**            | 357.0 MiB              | 882.3 MiB<sup>1</sup>                    | 122.6 MiB<sup>2</sup>                        |
| Download for a machine without it     | **46.4 MiB**<sup>3</sup> | 155.0 MiB<sup>3</sup>  | 206.9 MiB<sup>4</sup>                    | 59.6 MiB<sup>3</sup>                         |
| With the runtime fixed to one version | —                        | —                      | 682.2 MiB on disk, 298.2 MiB to download | —                                            |

</div>

<div class="table-notes">

1. 4.1 MiB of app and 878.2 MiB of WebView2, which is installed system-wide and shared with the other apps that use it.
2. `windeployqt` adds a software OpenGL fallback and the MSVC runtime's installer; without them the folder is 85.2 MiB.
3. The zipped app, standing for the download.
4. Tauri's zipped app, 1.7 MiB, and WebView2's 205.2 MiB standalone installer. Where WebView2 is already installed, as on most Windows 10 and 11 machines, Tauri's download is 1.7 MiB and its installer 1.33 MiB.

</div>

<div class="wide leaders">

| Running, median of 3            | Skathi    | Electron  | Tauri 2               | Qt 6         |
| ------------------------------- | --------- | --------- | --------------------- | ------------ |
| Processes                       | **1**     | 4         | 7 (app + 6 WebView2)  | **1**        |
| First frame with content        | 743 ms    | 1,009 ms  | 597 ms                | **469 ms**   |
| Window on screen                | 743 ms    | 977 ms    | **76 ms**<sup>1</sup> | 281 ms       |
| Private memory, idle            | 85.7 MiB  | 87.2 MiB  | 93.6 MiB              | **28.0 MiB** |
| Private memory, after scrolling | 85.1 MiB  | 86.4 MiB  | 96.7 MiB              | **28.9 MiB** |
| Working set, all processes      | 149.4 MiB | 316.0 MiB | 353.8 MiB             | **81.9 MiB** |
| Dedicated GPU memory            | 28.8 MiB  | 22.7 MiB  | **18.0 MiB**          | 21.9 MiB     |
| CPU when idle, % of one core    | 1.4       | **0.3**   | 0.6                   | 0.5          |
| CPU when idle after scrolling   | 1.1       | 0.8       | **0.2**               | **0.2**      |

</div>

<div class="table-notes">

Each row counts every process the app runs, WebView2's included.

1. Empty until the WebView paints; Tauri's first frame with content comes at 597 ms.

</div>

A native call, measured inside the same apps: 10,000 calls of `add(a, b)` one after another, then 50 sends of 1 MiB of bytes to native code, which returns the length. Electron and Tauri calls are awaited promises; Skathi and Qt calls return directly.

<div class="wide leaders">

| Native call           | Skathi                           | Electron                      | Tauri 2                      | Qt 6                    |
| --------------------- | -------------------------------- | ----------------------------- | ---------------------------- | ----------------------- |
| Where the call runs   | **in the same process**          | in the main process, over IPC | in the app process, over IPC | **in the same process** |
| `add(a, b)`, per call | **0.05 µs**, 0.62 µs<sup>1</sup> | 124 µs                        | 1,183 µs                     | 0.1–0.2 µs<sup>2</sup>  |
| 1 MiB to native code  | **0.002 ms**, 1.0 ms<sup>3</sup> | 1.18 ms<sup>4</sup>           | 9.5 ms<sup>4</sup>           | 0.12 ms<sup>4</sup>     |

</div>

<div class="table-notes">

1. 0.05 µs through a [native API](/embedding/native-api/), 0.62 µs through a [native module](/embedding/native-modules/).
2. QML's clock counts milliseconds.
3. 0.002 ms as a typed array a native API reads in place, 1.0 ms as a string through a native module.
4. Electron sends a structured clone of a `Uint8Array`, Tauri an `ArrayBuffer` as a raw body, and Qt converts an `ArrayBuffer` to a `QByteArray`.

</div>

### How it was measured

- **Machine.** AMD Ryzen 9 9950X (16 cores, 32 threads), 96 GB of RAM, GeForce RTX 5070 and RTX 3070 Ti (driver 32.0.16.1692), a 2560×1440 primary display, Windows 11 Pro Insider build 29683.
- **WebView2.** The installed Evergreen runtime 157.0.4341.46 is 878.2 MiB in 918 files. The standalone x64 installer is 205.2 MiB and the bootstrapper 1.8 MiB. The fixed-version runtime 157.0.4341.60 for x64 is a 296.5 MiB cab that unpacks to 678.1 MiB.
- **Versions.** Skathi 0.3.0 with its release host (no devtools). Electron 45.3.3 (Chromium 156), packaged with `@electron/packager` 20.3.0. Tauri 2.12.6 with wry 0.57.2 on WebView2 157.0.4341.46, NSIS bundle. Qt 6.12.0 for MSVC 2022, built with Visual Studio 2026 and CMake. Vue 3.5.45 and Vite 8.3.8 for the web apps. Skathi's native-call numbers come from an embedder built in release mode against the current `skathi` crate.
- **Runs.** Each app started four times. The first start filled the disk cache and each engine's own caches and is not counted; the tables show the median of the other three. Each app opened its window without taking focus and was closed by the script.
- **Start.** Both times run from process creation. _First frame with content_ ends when the app has drawn its UI: the web apps change the window title in the second `requestAnimationFrame` after mounting, and Qt does it on its first `frameSwapped`. _Window on screen_ ends when a visible window with the app's title exists, drawn or not. A script polls the window titles every millisecond.
- **Memory.** Summed over every process in the app's process tree, for every framework: Electron's helpers and Tauri's six `msedgewebview2.exe` processes are counted in each memory, GPU and CPU row. Samples are taken 5 seconds after start and 3 seconds after the scroll ended. Private memory is the private working set that Task Manager shows; the working set adds shared pages, once per process; GPU memory comes from Windows' GPU Process Memory counters.
- **CPU.** CPU time of the process tree over 10 seconds, idle, after each memory sample.
- **Noise.** Other work ran on the machine, at 10 to 37 % of total CPU before a start. Single starts varied: one of Electron's took 3.7 seconds. The native-call numbers vary by about 20 % between runs.

## Rendering and platforms

<div class="wide leaders">

|                                         | Skathi                                                                              | Electron                                                       | Tauri 2                                          | Qt 6                                                                            |
| --------------------------------------- | ----------------------------------------------------------------------------------- | -------------------------------------------------------------- | ------------------------------------------------ | ------------------------------------------------------------------------------- |
| What draws the UI                       | Skathi's renderer<sup>1</sup>                                                       | Chromium, a copy in every app                                  | The OS WebView<sup>2</sup>                       | Qt Quick's scene graph<sup>3</sup>                                              |
| JavaScript engine                       | JavaScriptCore, one revision on every OS                                            | V8, in every app                                               | V8 on Windows, JavaScriptCore on macOS and Linux | QML's own engine                                                                |
| Engine version chosen by                | ✓ The app                                                                           | ✓ The app, a new Chromium major every 8 weeks<sup>4</sup>      | ✗ The OS<sup>5</sup>                             | ✓ The app                                                                       |
| Same result on Windows, macOS and Linux | ✓ The rendering; OS services vary<sup>6</sup>                                       | ✓ One Chromium, at 357 MiB per app                             | ✗ Three engines to test<sup>7</sup>              | ✓ One scene graph; Qt Quick Controls default to a different style per OS        |
| Web platform                            | Partial<sup>8</sup>                                                                 | **✓ All of Chromium, plus Node.js**                            | Differs by OS<sup>9</sup>                        | ✗ QML, not HTML or CSS<sup>10</sup>                                             |
| UI code                                 | Vue 3 or Svelte 5, with TypeScript                                                  | **Any web framework**                                          | **Any web framework**, on three engines          | QML and C++, or C++ Widgets                                                     |
| Native GPU content inside the UI        | **✓ [`<sk-surface>`](/embedding/native-frames/)**; CPU frames on Linux<sup>11</sup> | Partial, experimental<sup>12</sup>                             | ✗ Only behind or beside a transparent WebView    | **✓ `QQuickRhiItem`** and native textures as `QSGTexture`, in C++               |
| More windows                            | **✓ [Pop-outs](/guides/pop-out-windows/) share one realm and its state**            | A renderer process per window, state over IPC                  | A WebView per window, events through Rust        | **✓ One process, shared C++ and QML objects**                                   |
| Desktop OSes                            | Windows x64, macOS on Apple silicon, Linux x64                                      | Windows 10 and later, macOS 13 and later, Linux; x64 and arm64 | Windows, macOS, Linux                            | **Windows, macOS, Linux, embedded Linux, QNX, VxWorks, INTEGRITY, WebAssembly** |

</div>

<div class="table-notes">

1. A Rust element tree, CSS engine and layout on DirectX, Metal or wgpu (Vulkan).
2. WebView2 (Chromium) on Windows, WKWebView (WebKit) on macOS, WebKitGTK on Linux.
3. On Direct3D 11 or 12, Vulkan, Metal or OpenGL.
4. Only the latest three majors get fixes.
5. WebView2 updates itself, WebKit comes with macOS and WebKitGTK with the distribution.
6. One renderer, CSS engine, JavaScriptCore revision and sRGB colour pipeline on every OS. OS services vary; each [reference](/web-platform/) page lists its platforms.
7. CSS and API support, GPU paths, colour handling and rendering bugs are each engine's own, and WebKitGTK has documented GPU problems on Linux.
8. Flexbox, grid, Canvas 2D, WebGPU, `fetch`, WebSocket, IndexedDB and more, listed under [web platform](/web-platform/) and [CSS](/css/). No Web Workers, WebGL, WebRTC or Shadow DOM.
9. Each WebView's own: WebGPU, for one, is partial in Safari.
10. Web content needs Qt WebEngine, a Chromium of its own.
11. D3D11, D3D12 and shared Vulkan textures on Windows, Metal on macOS, CPU frames on Linux; composited like any element, with the UI on top.
12. The experimental `sharedTexture` module (Electron 40 and later) turns a shared texture into a `VideoFrame` for a canvas or WebGPU.

</div>

## JavaScript and native code

The question is whether the UI's code calls native code in its own process or sends a message to another one. The measured cost is in the [native call](#measured) table.

<div class="wide leaders">

|                                  | Skathi                                                             | Electron                                      | Tauri 2                                         | Qt 6                                                        |
| -------------------------------- | ------------------------------------------------------------------ | --------------------------------------------- | ----------------------------------------------- | ----------------------------------------------------------- |
| Native language                  | Rust, optional<sup>1</sup>                                         | JavaScript in Node.js, C++ in Node-API addons | Rust                                            | C++                                                         |
| Calling it from the UI           | **✓ In process**<sup>2</sup>                                       | IPC, a structured clone<sup>3</sup>           | IPC, JSON or raw bytes<sup>4</sup>              | **✓ In process**<sup>5</sup>                                |
| Types across the boundary        | **✓ `sk types`** writes the app's `.d.ts` from the Rust signatures | Written by hand for the preload API           | tauri-specta, a release candidate outside Tauri | **✓ `QML_ELEMENT`** registers C++ types for the QML tooling |
| Embedding in an existing program | **✓ A Rust crate**<sup>6</sup>                                     | ✗ Electron is the program                     | Partial<sup>7</sup>                             | **✓ A C++ library**<sup>8</sup>                             |

</div>

<div class="table-notes">

1. Through the [`skathi` crate](/embedding/overview/). An app built with `sk` needs none.
2. A [native module](/embedding/native-modules/) function runs inside the JavaScript call, its values passed through JSON and serde; a `#[task]` runs on a worker and returns a promise. A [native API](/embedding/native-api/) passes engine values directly.
3. `ipcRenderer.invoke` sends a structured clone to the main process. In process only through a Node-API addon in a renderer with the sandbox turned off, which Electron's security guide advises against.
4. `invoke` sends JSON, or raw bytes, from the WebView's process to the app's and returns a promise.
5. `Q_INVOKABLE` methods and properties, values converted through `QVariant`.
6. The `skathi` crate runs the UI on its own thread inside your Rust program; `main` stays yours.
7. Tauri owns the main thread's event loop; wry alone can put a WebView in an existing window.
8. A `QQuickView` or `QQuickWidget` goes into an existing program.

</div>

## Tooling and licensing

<div class="wide leaders">

|                       | Skathi                                                                 | Electron                                                                 | Tauri 2                                                         | Qt 6                                                          |
| --------------------- | ---------------------------------------------------------------------- | ------------------------------------------------------------------------ | --------------------------------------------------------------- | ------------------------------------------------------------- |
| Libraries             | Partial<sup>1</sup>                                                    | **✓ npm, the web and Node.js**                                           | npm and the web within each WebView's support, plus Rust crates | Qt's modules and C++ libraries<sup>2</sup>                    |
| Hot reload            | ✓ Vue templates keep their state; Svelte components hot-update         | ✓ The renderer; main-process changes restart the app                     | ✓ The frontend; Rust changes rebuild and restart the app        | ✓ QML since Qt 6.12; C++ changes rebuild                      |
| Devtools              | The [inspector](/guides/devtools/), no breakpoint debugger<sup>3</sup> | **✓ Chrome DevTools**                                                    | A different WebView inspector on each OS                        | **✓ Qt Creator's QML debugger and profiler**                  |
| What a build needs    | **Bun**<sup>4</sup>                                                    | **Node.js**                                                              | Rust, MSVC on Windows, and Node.js<sup>5</sup>                  | A C++ compiler, CMake and the Qt SDK                          |
| Installer and updates | Yours: `sk publish` makes the exe, or a `.app` on macOS                | ✓ Electron Forge or electron-builder; `autoUpdater` on macOS and Windows | ✓ MSI, NSIS, DMG, AppImage, deb and rpm; an updater plugin      | ✓ `windeployqt` and `macdeployqt`; the Qt Installer Framework |
| License               | MIT or Apache-2.0                                                      | MIT                                                                      | MIT or Apache-2.0                                               | LGPLv3 or commercial<sup>6</sup>                              |
| Releases              | 0.x: APIs can still change between minor releases                      | A major every 8 weeks; the latest three are supported                    | 2.x since October 2024                                          | 6.12 is an LTS release, from September 2026                   |

</div>

<div class="table-notes">

1. npm, Vue and Svelte libraries and Tailwind CSS work, except code that needs a browser feature the engine lacks; `sk check` flags unsupported CSS and framework features, in the app and its packages.
2. Third-party QML components are few next to npm.
3. Elements, styles, console, network, storage and a frame profiler.
4. `sk publish` stamps a prebuilt host, so an app needs no compiler.
5. 2 min 43 s for this app's first release build.
6. LGPLv3 or GPLv3, or commercial. Graphs, Quick 3D, Virtual Keyboard and other modules are GPLv3 or commercial only. The LGPL requires that users can relink your app against another Qt. Later LTS patch releases are commercial only. The Small Business license starts at €546 a year.

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

**Qt**

- Release and hot reload: [Qt 6.12 released](https://www.qt.io/blog/qt-6.12-released)
- Rendering: [scene graph renderer](https://doc.qt.io/qt-6/qtquick-visualcanvas-scenegraph-renderer.html), [QQuickRhiItem](https://doc.qt.io/qt-6/qquickrhiitem.html), [QSGD3D11Texture](https://doc.qt.io/qt-6/qnativeinterface-qsgd3d11texture.html), [QQuickWindow](https://doc.qt.io/qt-6/qquickwindow.html)
- QML and C++: [data type conversion](https://doc.qt.io/qt-6/qtqml-cppintegration-data.html)
- Platforms: [supported platforms](https://doc.qt.io/qt-6/supported-platforms.html)
- Licensing: [module licenses](https://doc.qt.io/qt-6/licensing.html), [licensing](https://www.qt.io/licensing), [Qt for Small Business](https://www.qt.io/qt-for-small-business), [open-source licensing FAQ](https://www.qt.io/faq/tag/qt-open-source-licensing)
- Deployment and tools: [Windows deployment](https://doc.qt.io/qt-6/windows-deployment.html), [Qt Installer Framework](https://doc.qt.io/qtinstallerframework/index.html), [QML debugging](https://doc.qt.io/qtcreator/creator-debugging-qml.html)
