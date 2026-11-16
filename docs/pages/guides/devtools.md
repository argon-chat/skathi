---
title: Devtools
label: Devtools
description: Open the inspector with F12, read the element tree, styles, console, network and storage, record frames in the Performance tab, and control a running app from a script.
section: Guides
order: 6
---

Every development build of a Skathi app carries an inspector. It opens in a window of its own, beside the app, and shows the element tree with its matched CSS rules and computed styles, the console with a JavaScript prompt, network requests, storage, shader effects and a frame profiler. It runs in a process of its own and reads the app over a local connection, so an app that hangs or crashes does not take the inspector down with it.

## Open the inspector

Run the app with `bun run dev` (`sk dev`) and press **F12** in its window. The inspector opens without taking focus from the app. Press F12 again and the inspector already open is brought to the front instead of a second one. Pressed in a [pop-out window](/guides/pop-out-windows/), F12 opens an inspector on that window.

F12 is caught before your app's key handlers, so an app cannot swallow it by accident. Your app still receives the `keydown`, as in a browser. F12 with Ctrl, Alt, Shift or Cmd held does not open the inspector.

The inspector closes when the app quits.

### Which builds have devtools

| Command                          | Devtools                                                |
| -------------------------------- | ------------------------------------------------------- |
| `bun run dev` (`sk dev`)         | yes                                                     |
| `bun run build` (`sk build`)     | yes: a debug build in `dist/debug`                      |
| `bunx sk run`                    | yes: `sk build`, then starts the app                    |
| `bun run publish` (`sk publish`) | only with `"publishDevtools": true` in `sk.config.json` |

`bun create skathi` asks whether published builds keep devtools and writes the answer to `publishDevtools`. Without it, `sk publish` packages a host built without devtools, described in [Release builds](#release-builds).

### Open it from code

`appDevtools` opens the inspector as F12 does, for example from a Help menu:

```ts
import { appDevtools } from "@skathi-native/runtime";

export function openDevtools(): void {
  if (appDevtools.available) appDevtools.open();
}
```

A Svelte app imports it from `@skathi-native/runtime/svelte`.

## Find your way around

The inspector attaches on its own to the app that opened it. The title bar shows the app's slot (the directory that identifies a running app), its process id and the window it inspects, and on the right **live** while updates are pushed or **polling** while it falls back to asking once a second.

The toolbar holds **⌖ inspect**, the tabs **Elements**, **Network**, **Performance**, **Storage**, **Targets** and **Effects**, and a **console** button. When the app has pop-out windows, a row of chips selects which window's tree the inspector shows.

| Key              | Does                                          |
| ---------------- | --------------------------------------------- |
| Ctrl+Shift+C     | pick an element in the app                    |
| Ctrl+F           | search the element tree                       |
| Ctrl+1 to Ctrl+6 | switch tabs, in toolbar order                 |
| Esc              | open or close the console drawer              |
| Ctrl+`           | open the console and focus its prompt         |
| Ctrl+C           | copy the selected console line or network URL |

On macOS, Cmd works wherever Ctrl is listed.

## Elements and styles

The **Elements** tab shows the element tree on the left and the styles of the selected element on the right.

- **Select an element.** Click a row, or press **⌖ inspect** (Ctrl+Shift+C) and click the element in the app. The click selects it instead of reaching your app; Esc cancels. The selected element is outlined in the app.
- **Search.** **find** (Ctrl+F) searches the whole tree by tag, `.class`, `#id` or text, and expands the parents of each hit. Enter and Shift+Enter step through the hits.
- **Vue fragment anchors.** The `<!---->` toggle shows the empty comment nodes Vue leaves for `v-if` and `v-for`. They paint nothing, so they are hidden by default.
- **Breadcrumbs.** The bar under the tree lists the selected element's ancestors; click one to select it.

The styles pane has two views:

- **rules** lists the CSS rules that matched the element, the winning one first, with their selectors, `@media` conditions, specificity and the state they apply in, such as `hover`. Overridden declarations are struck through. A declaration the engine does not support carries its advisory under the rule.
- **computed** lists the resolved values, grouped by state: **live** is what is in effect this frame, and **base**, **hover**, **active**, **focus**, **focus-visible** and **focus-within** are what each state resolves to.

The tree and the styles are read-only: the inspector does not edit attributes, text or CSS. Each rule and value has a copy button.

## Console

The console is a drawer under every tab. Open it with **console**, Esc, or Ctrl+` to go straight to the prompt.

It shows `console` calls, uncaught errors, CSS and runtime advisories, the `[skathi perf]` [performance advisories](/guides/performance-advisories/) and the host's own `[window]` and `[storage]` lines. `%c` styling is rendered.

- **Filter** by level with **error**, **warn**, **info** and **debug**, each with its count, or by text with the filter box.
- **follow** keeps the newest line in view. **preserve** keeps the lines when the inspector reattaches. **clear** (Ctrl+L) empties it.
- **Evaluate** JavaScript in the app at the prompt. Up and Down walk the history, and Tab completes global names. The result is shown with its type; an error with its stack.
- Right-click a line to copy it, the selected lines or every visible line.

The prompt has no `$0` or other console helpers.

## Network

The **Network** tab lists every `fetch`, `XMLHttpRequest` and `WebSocket` the app made since it started, with method, URL, protocol, connection, status and time. A `(disk cache)` protocol means the response came from the HTTP cache. Filter by URL, or right-click a row to copy its URL, a cURL command, a `fetch()` call, the response body or the response headers.

Click a row to open its details:

- **Headers**: the general facts, the response headers and the request headers. Cookie and authorization values are masked until you press **show**.
- **Payload**: query string parameters, form data and the request body. The bytes of uploaded files are not kept.
- **Response**: the body, pretty-printed or raw for JSON, as hex for binary.
- **Timing**: when the request started, when the response arrived and when the body was read. There is no DNS, connect or TLS breakdown.

A WebSocket row has **Socket** and **Timing** instead: the URL, subprotocol, state, close code and reason, and the number of messages and bytes sent and received. The messages themselves are not logged.

Bodies are kept for recent requests only, 256 KB each. There is no request throttling or blocking.

## Performance

The **Performance** tab has three views.

### Overview

A live chart of the last few hundred frames, always running: the work each frame took, the time it waited for the next frame, and the display's frame budget. Work past 60% of the budget is amber, past the budget red. Above the chart are the median, 95th percentile and worst frame work. Waiting never counts against the budget.

Under it, a memory breakdown by category, with **Refresh memory**, **Measure RAM** and **Force GC**. Force GC changes the app's state: it runs two full garbage collections. The memory breakdown is available on Windows and macOS.

Start the app with `SK_PERF=1` in its environment to also print a per-phase breakdown of each frame to the terminal.

### Frames

Press **Record interaction**, use the app, and press **Stop recording**. The timeline shows one bar per frame, its work stacked by category: JavaScript, microtasks, DOM, style, layout, paint, render and native work. The frame with the most work is selected; click another bar, or use the Left and Right keys.

A selected frame shows its calls in order, as a tree with self and total time, which you can zoom (+ and -) and pan (Shift+Left and Shift+Right). Select a call to see its source location and counters such as layout reads, style reads and DOM writes.

Under the calls, the frame lists its hints:

- **Long callback**: a JavaScript callback held the main thread for more than 8 ms.
- **Forced synchronous layout**: a geometry read, such as `getBoundingClientRect()`, made the engine lay out in the middle of the frame. Batch DOM writes before reads.
- **Read-write ping-pong**: DOM writes after a forced layout caused another one.
- **DOM node churn**: more than 128 nodes were created and more than 128 removed in the same frame. Check list keys and components that remount.
- Under `sk dev`, the [performance advisories](/guides/performance-advisories/) for that frame, with a count when one repeated.

Times are main-thread CPU time. GPU execution is not timed. On Windows the recording also includes the Direct3D 12 draw, submit and present steps.

### Allocations

**Start tracking** records allocations made by the native host, grouped by category and site. **Snapshot** reads the live totals, and **Set baseline** shows the change from a snapshot to the next. JavaScript objects are not part of it: this view is for memory held by the native side.

## Storage

The **Storage** tab shows `localStorage`, `sessionStorage` and IndexedDB.

- In `localStorage` and `sessionStorage` you can edit a value in place, delete a key, add one, or **clear** the area.
- IndexedDB is read-only: pick a database, an object store and optionally an index, then page or search through the records, 50 at a time.

## Effects

The **Effects** tab lists the shader effects the app can use, built-in or its own, and the elements that paint one through `--shading`. Select an element to tune each parameter of its effect with sliders and colour pickers; the change applies live. The **CSS** box shows the resulting `--shading` declaration to copy into your stylesheet, and **Reset** drops the change.

## Targets

The **Targets** tab lists the running Skathi apps the inspector can find, with their process ids and ports, and the windows of the one attached. Apps built with `sk build` are found wherever they run; development hosts are found beside the one the inspector was opened from. Click a row to attach to another app, or to inspect another window. **capture** takes a screenshot of the selected window without raising it.

## Turn devtools off

Three settings control whether a build that carries devtools listens for the inspector:

- `"devtools": false` in `sk.config.json`, at the top level or in the `window` block, starts the app with devtools off: no F12 and no local port. See [`window.devtools`](/window-api/config-window-devtools/).
- `appDevtools.disable()` and `appDevtools.enable()` switch them at runtime. `appDevtools.onChange()` reports the change.
- The environment variable `SK_DEVTOOLS=1` turns them on and refuses `disable()`, which is useful to inspect an app that turns them off. `SK_DEVTOOLS=0` starts with them off.

`SK_DEVTOOLS_KEY=off` disables only the F12 key; `appDevtools.open()` and the local port still work.

## Release builds

`sk publish` without `"publishDevtools": true` uses a host built without devtools. The inspector, F12 and the local port are not in the executable, and nothing can turn them on: `SK_DEVTOOLS=1` and `appDevtools.enable()` have no effect.

`appDevtools` still exists in that build, so code that calls it does not need a check. `appDevtools.available` is `false`, `isEnabled()` returns `false`, and `open()` returns `false` and prints an advisory.

The inspector's connection can evaluate JavaScript in your app. It listens on `127.0.0.1` only and requires a token the app writes into a handshake file, but a published build with devtools still lets a program on the same machine that can read that file drive the app. Keep `publishDevtools` off for apps you ship to users.

## Control a running app from a script

The CLI package includes the script the inspector's protocol is built on. It sends one command to a running app and prints the reply as JSON, which is useful in scripts, tests and coding agents:

```bash
bun node_modules/@skathi-native/cli/devtools/ctl.ts --dist dist/.sk-dev tree --depth 3
bun node_modules/@skathi-native/cli/devtools/ctl.ts --dist dist/.sk-dev click "button.save"
bun node_modules/@skathi-native/cli/devtools/ctl.ts --dist dist/.sk-dev eval "document.title"
```

`--dist dist/.sk-dev` addresses the app `sk dev` is running from the current directory. For an app built with `sk build`, pass `--app <name>` with the `name` from `sk.config.json`; it attaches to the newest running copy. `--window <id>` addresses a pop-out window; the `windows` command lists them, and `<SkWindow>`'s `open` event gives the id.

The commands cover what the inspector shows and more: `tree`, `query`, `text`, `styles`, `rules`, `highlight`, `pick`, `logs`, `net`, `storage`, `indexeddb`, `perf`, `trace`, `mem`, `screenshot`, `effects`, and input that goes through the real hit test, `click`, `focus`, `type`, `key`, `mouse`, `scroll` and `wheel`. Run the script with `--help` for the full list and their options.

## See also

- [Performance advisories](/guides/performance-advisories/): the `[skathi perf]` warnings and how to fix each one.
- [Pop-out windows](/guides/pop-out-windows/): the windows the inspector can switch between.
- [`window.devtools`](/window-api/config-window-devtools/) in `sk.config.json`.
