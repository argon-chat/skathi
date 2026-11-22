---
title: Porting from Electron or Tauri
label: Porting from Electron or Tauri
description: "What to expect when an Electron or Tauri app moves to Skathi: what replaces the main process and IPC, where each Electron and Tauri API goes, what has no counterpart yet, and an order to do the work in."
section: Guides
order: 18
---

# Porting from Electron or Tauri

An Electron or Tauri app is a web page in a browser engine, with a second process beside it for
everything the page may not do. A Skathi app is one process with no browser engine: your components
run in JavaScriptCore, and a renderer written for them draws the result. [Architecture](/architecture/)
has the picture, and [Comparison](/comparison/) has the numbers.

Two things follow for a port. The second process goes away, and most of what crossed to it becomes
an ordinary function call. And the page loses the browser: the app keeps what Skathi implements of
the web platform, and has to replace the rest.

## Check that the app fits

Before any work, three questions decide whether the port is worth starting:

- **The UI is Vue 3 or Svelte 5.** Skathi renders these two. An app in React, Angular or Solid is a
  rewrite of its UI, not a port.
- **The app does not need a browser.** There is no HTML parser, no `innerHTML`, and no inline text
  layout that mixes boxes inside a line the way a document does. A rich-text editor built on
  `contenteditable`, or a page that renders HTML it downloads, has to live in an
  [`<iframe>`](/elements/iframe/), which is the system webview.
- **Its libraries do not need one either.** A library that measures and draws with the DOM it is
  given usually works. One that needs Web Workers, WebRTC, service workers or WebGL does not.

`sk check` answers the third question for you, and much of the second. Add Skathi to the app as it
is and run it:

```sh
bun add -d @skathi-native/cli @skathi-native/runtime
bunx sk check
```

It reads the app and every package the bundle includes and [reports](/sk-check/) the CSS the
renderer does not apply, the Vue and Svelte features it lacks, and code that will not run. A long
report from one package is a reason to look for another package. [Web platform](/web-platform/) and
[CSS](/css/) list everything by name.

## What replaces what

### From Electron

| In Electron                                | In Skathi                                                                                                             |
| ------------------------------------------ | --------------------------------------------------------------------------------------------------------------------- |
| The main process                           | Nothing: the app's script is the app. Native code, when you need it, is a [native module](/embedding/native-modules/) |
| The preload script and `contextBridge`     | Nothing. There is no page to isolate the native side from                                                             |
| `ipcRenderer.invoke` and `ipcMain.handle`  | A function of a native module, called in process. `#[task]` makes it a promise on a worker thread                     |
| `webContents.send` and `ipcRenderer.on`    | An event of a native module, or plain JavaScript when both ends were JavaScript                                       |
| `new BrowserWindow(…)` for the main window | [`window` in sk.config.json](/sk-config/#window), and [`appWindow`](/window-api/) at run time                         |
| `new BrowserWindow(…)` for more windows    | [`<SkWindow>`](/guides/pop-out-windows/): a part of the same component tree, in a window of its own                   |
| `frame: false`, `titleBarStyle`            | [`titleBarStyle`](/guides/custom-title-bar/) and `app-region: drag` in CSS                                            |
| `app.quit()`, `app.relaunch()`             | `app.quit()` and `app.restart()` from [`skathi:app`](/std/)                                                           |
| `app.requestSingleInstanceLock()`          | [`"singleInstance": true`](/sk-config/) and the `launch` event of [`appLaunch`](/app-api/app-launch/)                 |
| `app.setAsDefaultProtocolClient()`         | [`appLaunch.registerProtocol()`](/app-api/launch-register-protocol/), on Windows                                      |
| `Tray` and its `Menu`                      | [`appTray`](/app-api/app-tray/), on Windows and macOS                                                                 |
| `globalShortcut`                           | [`appHotkeys`](/app-api/app-hotkeys/), on Windows and macOS                                                           |
| `shell.openExternal`, `showItemInFolder`   | [`appShell`](/app-api/app-shell/), on Windows and Linux                                                               |
| `dialog.showOpenDialog`, `showSaveDialog`  | [`showOpenFilePicker()` and its siblings](/web-platform/file-system-access/), or `<input type=file>`                  |
| `dialog.showMessageBox`                    | A dialog component of your own. `alert()` and `confirm()` show nothing                                                |
| `Menu` for a context menu                  | A menu component of your own, opened from a `contextmenu` handler                                                     |
| `clipboard`                                | [`navigator.clipboard`](/web-platform/async-clipboard/)                                                               |
| `safeStorage`, `electron-store`            | `localStorage` and IndexedDB, [encrypted at rest](/app-api/app-storage/) when the app asks                            |
| `desktopCapturer`                          | [`appCapture`](/app-api/app-capture/) with `getDisplayMedia()`, on Windows and macOS                                  |
| `systemPreferences.askForMediaAccess`      | [`appPermissions`](/app-api/app-permissions/)                                                                         |
| `app.setBadgeCount()`                      | `navigator.setAppBadge()`, on Windows and macOS                                                                       |
| `<webview>` and `BrowserView`              | [`<iframe>`](/elements/iframe/), the system webview composited under the app                                          |
| `fs`, `path`, `child_process`, `net`       | A native module in Rust, or [`skathi:ffi`](/std/dlopen/) for a C library. There are no Node modules                   |
| A Node-API addon                           | A native module, or `skathi:ffi` when the addon wraps a C library                                                     |
| `Notification`                             | None: `new Notification()` shows nothing, and no API for it is planned. A native module of yours can show one         |
| `autoUpdater`, electron-builder            | Yours. [`sk publish`](/quick-start/) makes the exe, or a `.app` on macOS; the installer and updates are the app's     |

### From Tauri

| In Tauri                                    | In Skathi                                                                                                            |
| ------------------------------------------- | -------------------------------------------------------------------------------------------------------------------- |
| `#[tauri::command]` and `invoke()`          | A `pub fn` of a `#[skathi::module]`, imported as `native:<name>` and called like any function                        |
| An `async` command                          | A `#[task]` method: it runs on a worker thread and the app gets a promise                                            |
| `emit` and `listen`                         | An event of the module, declared in `#[skathi::module(events(…))]`                                                   |
| Managed state (`State<T>`)                  | The module's own struct: its methods take `&self`                                                                    |
| Hand-written types, or tauri-specta         | `sk types` writes the `.d.ts` from the Rust signatures                                                               |
| Capabilities and permissions files          | [`permissions` in sk.config.json](/sk-config/#permissions) for what the app's script may reach, such as `skathi:ffi` |
| `tauri.conf.json` → `app.windows`           | [`window` in sk.config.json](/sk-config/#window)                                                                     |
| `WebviewWindow` for more windows            | [`<SkWindow>`](/guides/pop-out-windows/)                                                                             |
| `data-tauri-drag-region`                    | `app-region: drag` in CSS                                                                                            |
| The tray, global-shortcut and shell plugins | [`appTray`](/app-api/app-tray/), [`appHotkeys`](/app-api/app-hotkeys/), [`appShell`](/app-api/app-shell/)            |
| The dialog and fs plugins                   | [File System Access](/web-platform/file-system-access/) pickers and handles, or a native module                      |
| The store plugin                            | `localStorage` and IndexedDB, with [`appStorage`](/app-api/app-storage/)                                             |
| The single-instance and deep-link plugins   | [`appLaunch`](/app-api/app-launch/)                                                                                  |
| The updater plugin and the bundler          | Yours, as above                                                                                                      |
| `tauri::Builder` in `main`                  | Optional. Without Rust of your own there is no `main` to write: `sk` uses a prebuilt host                            |

A Tauri app keeps its Rust. The functions behind its commands move into modules almost as they are,
and [Embedding](/embedding/overview/) is the arrangement where your program stays the program and
Skathi is a crate inside it.

## IPC becomes a call

This is where most of the port's lines change, and where the app gets simpler. An Electron handler
and its caller:

```ts title="main.ts"
ipcMain.handle("folder-size", async (_event, path: string) => measure(path));
```

```ts title="renderer.ts"
const size = await window.api.folderSize(path);
```

with a preload script between them that exposes `api.folderSize`. In Skathi the three files are one
Rust method, and the app imports it:

```rust
#[skathi::module]
impl Files {
    #[task]
    pub fn folder_size(&self, path: String) -> Result<FolderSize, String> {
        measure(&path)
    }
}
```

```ts
import { folderSize } from "native:files";

const size = await folderSize(path);
```

There is no channel name to keep in step and no serialization: the values are read from the
engine's own. A method without `#[task]` runs inside the call and returns a value, not a promise,
which suits anything quick. [Native modules](/embedding/native-modules/) is the full walkthrough.

Not every handler needs Rust. Code that lived in the main process only because the page could not
do something, and that Skathi's own APIs now cover (the tray, a global shortcut, opening a link,
storage), moves into the app's script and the handler is deleted.

## Windows share one app

In Electron and Tauri each window is a page of its own, and windows talk through the main process.
A Skathi app is one JavaScript realm and one component tree. A second window is
[`<SkWindow>`](/guides/pop-out-windows/) around a part of that tree:

```vue framework="vue"
<SkWindow v-if="settingsOpen" title="Settings" :width="520" :height="640" @close="settingsOpen = false">
  <SettingsPanel />
</SkWindow>
```

The panel reads the same stores as the main window, so the code that synchronized state between
windows is deleted too. One thing to keep in mind: global APIs such as `window.innerWidth` and
`appWindow` describe the main window even inside a pop-out, while CSS is evaluated against the
window that paints the element. `window.open()` does not open a window.

## What the page loses

Things a web page takes for granted that a Skathi app does not have. `sk check` and the
[web platform reference](/web-platform/) are the full list; these are the ones ports meet first:

- **HTML strings.** There is no HTML parser, so `innerHTML`, `v-html` and `{@html}` insert nothing.
  Render data with components.
- **Inline formatting.** Text wraps inside its own box. Boxes do not flow inside a line of text, and
  there is no `float`.
- **Web Workers.** Long work goes to a `#[task]` of a native module, or a `nonblocking` call of
  `skathi:ffi`. An [audio worklet](/guides/audio-worklets/) is the one script that runs on another
  thread.
- **WebRTC and WebGL.** Neither exists. WebGPU and Canvas 2D do.
- **`alert()`, `confirm()`, `prompt()`, context menus.** Draw them in the app.
- **CSS.** Flexbox, grid, transitions, animations and most of what a modern stylesheet uses work.
  `filter` beyond the supported functions, `float` and right-to-left layout do not. [CSS](/css/) has
  every property.
- **A breakpoint debugger.** The [inspector](/devtools/overview/) has elements, styles, the console,
  network, storage and a frame profiler, and no stepping through code.

## Keep the web build

A port does not have to be a fork. `sk` reads the app's own `vite.config.ts`, so the same sources can
build for the browser with Vite and for the desktop with `sk build`, and the web build can stay the
app's Electron renderer while the port is in progress. Branch where the two differ:

```ts
const native = import.meta.env.SK;
```

and give sk.config.json an [`aliases`](/sk-config/) entry for a module that needs a different
implementation on each side. A `native:` import needs a stand-in in the web build:
[the same code in a web build](/embedding/native-modules/types-and-testing/#the-same-code-in-a-web-build).

## An order to work in

1. **Run `sk check`** on the app as it is, and settle the libraries it names.
2. **Get the window up.** Write sk.config.json, start `bunx sk dev`, and stub what the preload
   script exposed so the app mounts.
3. **Fix what looks wrong.** The check's CSS findings and the inspector's rules panel say why an
   element is not styled the way the browser styled it.
4. **Replace the stubs,** one feature at a time: Skathi's own API where there is one, a native
   module where there is not.
5. **Move the other windows** into `<SkWindow>`.
6. **Package it.** `bunx sk publish`, then the installer and updates the app already had.

[AOT](/guides/aot/) is not a step of a port: it is experimental, and a ported app runs without it.

## See also

- [Architecture](/architecture/): what runs where.
- [Comparison](/comparison/): measurements against Electron and Tauri.
- [Best practices](/guides/best-practices/): what to avoid on the UI thread.
- [Testing](/guides/testing/): driving the ported app in tests.
