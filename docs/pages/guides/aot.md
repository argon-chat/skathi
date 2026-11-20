---
title: Ahead-of-time compilation
label: AOT compilation
description: Compile a Windows build's JavaScript to a native image with sk publish --aot. How it works, what it changes, what it cannot run, and how to diagnose it.
section: Guides
order: 10
status: experimental
---

`sk publish --aot` compiles your app's JavaScript to machine code at build time and ships the result, a native image, beside the exe or (since 0.6, with `"publishSingleFile": true`) inside it. At start the host runs the image instead of evaluating the bundle. AOT is experimental, works on Windows x64 only, and today it is slower than the JIT in Skathi's own benchmark, so treat it as something to try and measure, not a default.

If you embed Skathi in your own executable, see [AOT for your own executable](/embedding/shipping/#aot-for-your-own-executable-since-07) (since 0.7).

<div class="tradeoffs">
<div class="tradeoff" data-tone="gain">

**What you get**

- **Machine code at build time.** `sk-aot` compiles the production bundle when you build, and the exe runs that image instead of evaluating `app.prod.js`.
- **One switch.** `--aot` for one build, or `"aot": true` in `sk.config.json`. `sk dev` keeps running the JavaScript with hot reload.
- **A check before the compile.** `sk check` reports the code that breaks or runs slower under AOT.
- **A way back.** `SK_APP_ENGINE=jit` starts the same exe without its image, to compare the two.

</div>
<div class="tradeoff" data-tone="cost">

**What it costs today**

- **Speed.** In the Vue benchmark the first round took 584 ms under AOT and 437 ms under the JIT.
- **Platforms.** Windows x64 only. `sk` refuses `--aot` for macOS and Linux.
- **Read-only built-ins.** Code that replaces a member of `Array.prototype` or a global such as `Map` throws a `TypeError`.
- **No JIT in the process.** `eval`, `new Function` and devtools expressions run in the interpreter.
- **No fallback.** A host that cannot load the image shows why and exits with code 3.
- **Size and build time.** The uncompressed image ships beside the exe or inside it, and each build adds the check and the compile.

</div>
</div>

## How it works

Without AOT, the exe carries the production bundle, `app.prod.js`, and JavaScriptCore runs it: it parses the bundle or reads its bytecode from a per-user cache, interprets it, and JIT-compiles the hot paths while the app runs. With `--aot`, `sk build` and `sk publish` do the compiling up front:

<div class="pipeline">
<p class="pipeline-phase">On your machine, during <code>sk build --aot</code> or <code>sk publish --aot</code></p>

1. **Sources** `src/**/*.vue`, `*.svelte`, `*.ts`

   The app's components and modules.

2. **Bundle** `app.prod.js`

   The production bundle, tree-shaken and minified, built as it is without AOT. Since 0.7, both builds write ASCII source so JavaScriptCore can keep one byte per character. Unicode text, regular expression patterns and tagged template strings retain their values. Set `SK_BUNDLE_UTF8=1` to keep the original source encoding.

3. **Check** `sk check`

   Runs over the app and its dependencies. Errors stop the build; warnings are printed and the build goes on. See [sk check](#before-the-first-aot-build-sk-check).

4. **Compile** `sk-aot`

   `sk-aot` first wraps the bundle so that the host's globals (`document`, `fetch`, timers and the rest) are inputs the compiler makes no assumptions about. Bun's AOT frontend turns it into an intermediate representation, and the JavaScriptCore backend that the Skathi host is built with turns that into machine code.

5. **Native image** `app.image` + `app.native.json`

   The image and its manifest. `sk-aot` is pinned to one Bun revision and one JavaScriptCore revision, and only a host built from the same pair can load its images.

6. **Container** `app.image.dll`

   Since 0.6, the image ships uncompressed in an executable-module container beside the exe: a DLL with no code of its own whose sections hold the image, page-aligned, which you sign like the exe. With [`"publishSingleFile": true`](#one-file-publishsinglefile) the same sections go into the exe instead. The manifest goes into the payload, and `app.prod.js` stays there: the host checks the image against it and runs it on a reload.

<p class="pipeline-phase" data-tone="run">On every machine that runs the app, at each start</p>

7. **Host maps the image where it lies**

   Before the JavaScript engine starts, the host checks the manifest against itself (compiler, engine, builtin table, the host's own startup scripts) and against the payload (the SHA-256 of `app.prod.js` and of the image). Since 0.6 it loads the container like any DLL: Windows maps each part with its own protection and applies code integrity policy and signature checks to the file. The host hashes the image where Windows mapped it and runs the image's entry there instead of evaluating `app.prod.js`. Nothing is expanded or written to disk.

</div>

The host's startup scripts and native globals are not part of the image; they run as they do without AOT.

Since 0.6, workloads carry only the image loader: JavaScriptCore's image compiler is in `sk-aot` alone.

## JIT and AOT compared

|                                              | JIT (default)                                                      | AOT (`--aot`)                                                       |
| -------------------------------------------- | ------------------------------------------------------------------ | ------------------------------------------------------------------- |
| At start                                     | Parses `app.prod.js` or reads its bytecode from the per-user cache | Checks the manifest, loads the native image and runs its entry      |
| While running                                | Interprets the bundle and JIT-compiles the hot paths               | Runs the image; the process has no JIT                              |
| `eval`, `new Function`, devtools expressions | Run like the rest of the code                                      | Run in the interpreter                                              |
| Built-in objects                             | Writable                                                           | Read-only: replacing their members throws a `TypeError`             |
| Per-user cache                               | Bytecode                                                           | None (since 0.6)                                                    |
| In the payload                               | `app.prod.js`                                                      | `app.prod.js` and the image's manifest                              |
| Beside the exe                               | `icu.skpk` and packs                                               | Also `app.image.dll`, unless `publishSingleFile` puts it in the exe |
| Build                                        | Bundle                                                             | Bundle, `sk check`, then `sk-aot`                                   |
| Development                                  | `sk dev`, with hot reload                                          | None: `sk dev` runs the JavaScript. `sk run --aot` tries the image  |
| Platforms                                    | Windows, macOS, Linux                                              | Windows x64                                                         |
| Benchmark, first round                       | **437 ms**                                                         | 584 ms                                                              |
| Benchmark, last round                        | **447 ms**                                                         | 550 ms                                                              |

### Where performance stands

AOT is experimental, and this is its current state: in Skathi's benchmark the AOT build is slower than the JIT, both in the first round after the app starts and in the last round. The compiler is being worked on. For now, AOT is not a guaranteed speedup.

The benchmark lives in `benches/aot` in the Skathi repository: a Vue table that mounts, updates, swaps and clears rows. Each run starts the app and repeats the same round of work, and the table gives the round's total time, median of 3 runs on one Windows x64 machine, with the toolchain current Skathi pins (Bun `237bba51e`, JavaScriptCore `autobuild-3bc94211`). Both columns come from the same exe: the JIT column runs it with `SK_APP_ENGINE=jit`.

Measure your own app the same way: build it once with `--aot`, then start that exe with and without `SK_APP_ENGINE=jit`.

Skathi's sample apps give an idea of the other costs. Their uncompressed images are 2.7 to 3.5 MB, and they compile in 1.3 to 2.1 seconds, with `sk-aot` peaking at 104 to 121 MiB of memory, plus the time of `sk check`. Since 0.6 the image ships uncompressed, so the app on disk grows by the image's full size. Apps built before 0.6 expanded their image into `%LOCALAPPDATA%\skathi\native-aot\v1\`; `sk prune` removes those folders.

## Turn it on

For one build, pass `--aot`:

```sh
bunx sk publish --aot
```

- `sk publish --aot` makes the release build.
- `sk build --aot` makes a debug build with devtools.
- `sk run --aot` builds and starts it.

To compile every build, set `"aot"` in `sk.config.json`:

```json title="sk.config.json"
{
  "name": "notes",
  "aot": true
}
```

`--no-aot` turns it off for one build. `sk dev` ignores the setting: it always runs the JavaScript, with hot reload.

> **Note:** An app with `"aot": true` needs `--no-aot` to build for macOS or Linux.

### Where sk-aot comes from

The first AOT build downloads `sk-aot` from the GitHub release of the `sk` version you run (`sk-aot-windows-x64.exe.gz`), checks its SHA-256 against that release's list, and caches it under `%LOCALAPPDATA%\skathi\hosts\<version>\aot\`. Before compiling, `sk` checks that `sk-aot` targets the same engine as the workload it stamps. `sk prune` removes the `sk-aot` of other versions.

### What the build leaves behind

The build summary has a `native` line with the image size and where it went: `app.image.dll` beside the exe or the exe itself. With `--json`, the result has a `native` object with the same information; its `container` is the path of `app.image.dll`, or `null` for an image in the exe. The intermediate files stay in `dist/<debug|release>/.sk/aot/`:

| File              | What it is                                         |
| ----------------- | -------------------------------------------------- |
| `native-input.js` | The wrapped bundle, exactly what `sk-aot` compiled |
| `app.ir`          | The intermediate representation                    |
| `app.image`       | The native image                                   |
| `app.native.json` | The manifest                                       |

### What you ship (since 0.6)

`sk publish --aot` writes the exe and, beside it:

| File                  | What it is                                                                                             |
| --------------------- | ------------------------------------------------------------------------------------------------------ |
| `app.image.dll`       | The native image, uncompressed, in a DLL with no entry point and no imports. The exe loads it at start |
| `inspector.image.dll` | The devtools inspector's image, when `"publishDevtools": true` and the workload has one                |
| `icu.skpk`            | The ICU data, as without AOT                                                                           |
| `*.skpk`              | The asset packs set in `"packs"`, as without AOT                                                       |

Keep these files together: the exe looks for them in its own folder, so the folder can be moved or installed anywhere as a whole. A missing or changed `app.image.dll` stops the app with a message that names the file. Sign `app.image.dll` with the exe; it is a valid PE file, and signing it does not change the image. The container is rebuilt from the same template on each build, so an unchanged image gives a byte-identical `app.image.dll` and its signature can be reused.

Each part of the image lands in its own section, protected as the engine needs it: `.skapp` (header, read-only), `.skappx` (code, read and execute) and `.skappd` (data, read-only); the inspector's are `.skins`, `.skinsx` and `.skinsd`. `dumpbin /headers app.image.dll` lists them. The container also has an exception table over the code, so debuggers and crash dumps walk the stack through the image; its frames show as `app.image.dll+offset`.

### One file: publishSingleFile (since 0.6)

```json title="sk.config.json"
{
  "name": "notes",
  "aot": true,
  "publishSingleFile": true
}
```

With `"publishSingleFile": true`, `sk publish` writes the exe alone. The image's sections, and the inspector's when there is one, are added to the exe, and the ICU data and the packs go into its payload. Windows maps the image as part of the exe, and the engine runs the code where it lies. One signature then covers everything. `sk build` ignores the option.

`sk publish` also extends the exe's exception table over the image's code: it writes a copy of the table with the image's entries added in a section of its own, `.skpdata`, and points the exe at it. Debuggers and crash dumps walk the stack through the image as they do beside the exe; its frames show as offsets into the exe. The copy adds the size of the host's own table to the exe, about 2 MiB.

Measured on one Windows x64 machine with a small Vue app, `icu.skpk` with every locale, 20 runs per variant in alternating order, after one discarded run each. **Start** is process start to the app's first mount, median. **Cold** means a per-user cache folder that is empty at each start, as on a new machine; **warm** one that earlier starts have filled. **On disk** is everything the app puts on disk: the exe, the files beside it and, before 0.6, the expanded image.

| aot-nojit workload, 2.6 MiB image                  | Start, cold | Start, warm | On disk               |
| -------------------------------------------------- | ----------- | ----------- | --------------------- |
| Before 0.6: image in the payload, expanded to disk | 409 ms      | 390 ms      | 86.8 MiB + 2.6 MiB    |
| Since 0.6: `app.image.dll` beside the exe          | 411 ms      | 421 ms      | 88.4 MiB              |
| Since 0.6: `"publishSingleFile": true`             | 407 ms      | 420 ms      | 90.3 MiB, in one file |

The start times stay within the spread of the runs, whose middle half spans about 50 ms. Checking the image's SHA-256 takes 5.8 ms of each start for this image, as it did before 0.6. The files you ship grow by the difference between the image and its compressed form, 1.6 MiB here, plus the copy of the exception table in one file, and nothing is written at the first start.

## Before the first AOT build: sk check

Run `sk check` on the app before you turn AOT on. An AOT build runs the same check and stops on its errors. Its codes come in five ranges:

| Codes    | Severity                   | Meaning                                                                                                                                              |
| -------- | -------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------- |
| `SK1xxx` | Error: the AOT build stops | Breaks under AOT. See [Built-ins are read-only](#built-ins-are-read-only).                                                                           |
| `SK2xxx` | Warning                    | Works under AOT, but runs slower. See [Code made at run time](#code-made-at-run-time).                                                               |
| `SK3xxx` | Warning                    | Typing quality: values the code passes around without a type.                                                                                        |
| `SK4xxx` | Warning                    | CSS the renderer does not apply, in the app's stylesheets, `<style>` blocks and `style` attributes; one summary per package.                         |
| `SK5xxx` | Error or warning           | Vue and Svelte features that do not work or work differently. See [Vue support](/guides/vue-support/) and [Svelte support](/guides/svelte-support/). |

`sk explain <code>` explains a code with an example and a fix, and `sk explain --list` lists them all. `sk check --deny-warnings` also fails on warnings, for CI.

### Built-ins are read-only

The SK1 codes have one cause: in an image, JavaScript's built-in objects are read-only. Replacing or deleting a member a built-in already has, redefining it through `Object.defineProperty` or `Object.assign`, changing a built-in's prototype, or replacing a global such as `Map` or `JSON` throws a `TypeError` at run time. `Reflect.set`, `Reflect.defineProperty` and `Reflect.setPrototypeOf` do not throw; they return `false` and leave the built-in unchanged.

```ts
// SK1001: throws a TypeError in an AOT build
const push = Array.prototype.push;
Array.prototype.push = function (...items) {
  track(items);
  return push.apply(this, items);
};

// Works with and without AOT
function pushTracked<T>(list: T[], ...items: T[]): number {
  track(items);
  return list.push(...items);
}
```

What still works:

- Adding a member a built-in does not have, except on `Object.prototype`, `Promise.prototype` and `RegExp.prototype`, which take no new members.
- Polyfills guarded by a check for the member, such as `if (!Array.prototype.at) { … }`, and assignments that put back the value read from the same property. `sk check` does not report these.
- Replacing the globals the engine leaves replaceable, among them `Promise`, `Date`, `Error` and `console`.

When the finding is in a dependency, look for a version or an option that does not patch built-ins. If the code that patches is never reached in your app, `sk.config.json` can silence the code for that package:

```json title="sk.config.json"
{
  "check": {
    "allow": { "SK1001": ["some-package"] }
  }
}
```

`"allow"` maps a code to package names or app-relative globs such as `src/legacy/**`.

> **Warning:** An allowed finding only lets the build go on. The code still throws if it ever runs.

### Code made at run time

An image has no JIT, so code created while the app runs is interpreted:

- **SK2001:** `eval`, direct or indirect.
- **SK2002:** `new Function(…)`. Libraries that generate their hot code this way (validators, template engines, serialisers) lose most of their speed. Many have a mode that does not generate code, often called CSP-safe or jitless.
- **SK2004:** `import()` with a computed specifier. A path the build did not compile fails to load at run time. Use string literals, for example a map of `() => import("./pages/home.ts")` loaders.

SK2003 reports a string passed to `setTimeout` or `setInterval`. Skathi's timers never run a string, with or without AOT, so that code is broken on every host.

### Typing warnings

The SK3 warnings measure how much of the code is typed: dependencies without declarations, `any` in what the app imports, `any` and `@ts-ignore` in the app's own code. They do not stop a build. The compiler does not use types: `sk build --aot` compiles the minified JavaScript bundle.

## What still runs without the image

The bundle runs from the image, and nothing falls back to the JIT. Once a process has loaded an image, it has no JIT:

- **Run-time code.** `eval`, `new Function` and expressions evaluated from devtools run in the interpreter.
- **WebAssembly.** Modules compile and run in the WebAssembly interpreter, many times slower than under the JIT. The [`aot-nojit` workload](#ship-without-the-jit) has no WebAssembly at all (since 0.6).
- **Reload.** The image's entry runs once per process. `location.reload()` evaluates `app.prod.js` from the payload instead.
- **A host that cannot load the image.** See the warning below.

> **Warning:** There is no fallback. When the manifest does not match the host, or the image is missing or changed, the app shows the reason in a message box and exits with code 3.

> **Tip:** To run the same exe without its image, set `SK_APP_ENGINE=jit`. The host then evaluates `app.prod.js` under the JIT and logs that it did. This is how you tell whether a bug or a slowdown comes from AOT. An exe on the [`aot-nojit` workload](#ship-without-the-jit) has no JIT and refuses it.

## Debugging

- **Develop with `sk dev`.** It runs the JavaScript with hot reload; there is no AOT in development. Use `sk run --aot` to try the image.
- **Read the host log.** `sk run` prints the app's output in the terminal; for `sk build`, `--console` keeps a console window. A run from the image logs two lines:

  ```text
  [native-aot] selected application image C:\Program Files\Notes\app.image.dll (entry 1, memory Default, sha256 checked in 5.6 ms)
  [native-aot] application native entry completed in … ms
  ```

  A run under `SK_APP_ENGINE=jit` logs `SK_APP_ENGINE=jit: running app.prod.js instead of the native image`.

- **Bisect with the JIT.** If a bug appears only in the AOT build, start the same exe with `SK_APP_ENGINE=jit`. A difference points at a built-in the code changes or code created at run time; run `sk check` again.
- **Look at the input.** `dist/<debug|release>/.sk/aot/native-input.js` is exactly what `sk-aot` compiled. When `sk-aot` fails, its error names this file.

## Troubleshooting

<div class="fixes">

### At build time

- `AOT builds are Windows x64 only for now`

  **Cause:** The build targets macOS or Linux, or `sk` runs on another OS or architecture.

  **Fix:** Build with `--no-aot`.

- `sk check found what a native image cannot run, so the AOT build stops here`

  **Cause:** `sk check` found errors. The report follows the message.

  **Fix:** Fix each error, using `sk explain <code>`, or build with `--no-aot`.

- `… cannot load native images (it was built without the native AOT loader)`

  **Cause:** The workload has no AOT loader.

  **Fix:** Use a release workload of the same version (`sk workload install`), or build yours with the `native-aot` feature.

- `… expects native images made with the … transform and this sk makes …`, or `… compiles for another workload`

  **Cause:** `sk`, `sk-aot` and the workload come from different Skathi versions.

  **Fix:** Use the `sk-aot` of the release that matches the workload: remove `--aot-compiler` and `SK_AOT_COMPILER` and let `sk` fetch it.

- `no sk-aot matches the workload`, with `SK_HOST_OFFLINE=1`

  **Cause:** No cached `sk-aot` matches the workload, and `SK_HOST_OFFLINE=1` forbids downloading one.

  **Fix:** Allow the download once, or pass `--aot-compiler <sk-aot.exe>`.

- `--aot compiles against the workload it stamps, so it cannot be combined with --payload-only`

  **Cause:** An image is tied to one host, so an AOT build must stamp the exe.

  **Fix:** Build without `--payload-only`, or without `--aot`.

- `the aot-nojit workload has no JIT and runs only native images`

  **Cause:** The build names the `aot-nojit` workload but compiles no image.

  **Fix:** Build with `--aot`, or with another workload.

- `this app's native image uses WebAssembly, which the aot-nojit workload leaves out` (since 0.6)

  **Cause:** The app, or a dependency, names `WebAssembly`. The lines after the message give each place, with its line and column in `app.prod.js`. A test such as `typeof WebAssembly` counts too: the image is compiled for an engine that has it, and may have answered the test ahead of time.

  **Fix:** Drop that use, or the dependency. Otherwise publish on the workload with the JIT: remove `"publishJit": false`.

- `the aot-nojit workload leaves out WebAssembly, and the sk-aot that compiled this app does not record what its image uses` (since 0.6)

  **Cause:** `sk-aot` comes from a Skathi version before 0.6, which does not say what an image uses.

  **Fix:** Use the `sk-aot` of the release that matches the workload: remove `--aot-compiler` and `SK_AOT_COMPILER` and let `sk` fetch it.

### At start or while running

- `this app's native image cannot run on this host`

  **Cause:** The image does not match the exe it runs in. The message box gives the reason.

  **Fix:** Rebuild the app with the `sk` that matches the workload.

- `this app cannot start: … app.image.dll is missing` or `… differs from the one this app was built with` (since 0.6)

  **Cause:** `app.image.dll` is not beside the exe, or it, or the image sections of a `publishSingleFile` exe, changed after the build.

  **Fix:** Ship every file `sk publish` wrote, together and unchanged. Signing them is fine: a signature does not touch the image.

- A `TypeError` the JIT build does not throw

  **Cause:** The code replaces a built-in on a path `sk check` did not see.

  **Fix:** Run the app with `SK_APP_ENGINE=jit` to confirm, then find the assignment from the stack.

- `SK_APP_ENGINE=jit asks for the JIT, but this exe is built on the aot-nojit workload`

  **Cause:** The app was published with `"publishJit": false`, so its exe has no JIT to run `app.prod.js` with.

  **Fix:** Unset `SK_APP_ENGINE`. To compare with the JIT, publish a build with `"publishJit"` on.

- `the app has no native image, and this exe is built on the aot-nojit workload`

  **Cause:** The payload was stamped into the `aot-nojit` workload without an image.

  **Fix:** Rebuild with `sk publish --aot`.

</div>

## Platform support

| Platform                               | AOT                                                                        |
| -------------------------------------- | -------------------------------------------------------------------------- |
| Windows x64                            | Build and run. Every Windows workload of a Skathi release can load images. |
| macOS, Linux                           | No. `sk` refuses `--aot` for these targets.                                |
| Building on macOS or Linux for Windows | No. `sk-aot` runs on Windows x64 only.                                     |

A workload you build yourself with `sk workload build` loads images only when it is built with the `native-aot` cargo feature.

## Ship without the JIT (since 0.6)

A process that runs an image already has no JIT, so the JIT compiler in the exe is code the app never runs. Since 0.6, `"publishJit": false` leaves it out: `sk publish --aot` then stamps the app into the `aot-nojit` workload, a Windows x64 release host whose JavaScriptCore is linked without the JIT and without WebAssembly.

```json title="sk.config.json"
{
  "name": "notes",
  "aot": true,
  "publishJit": false
}
```

- **Smaller.** The workload is 73.9 MiB instead of 87.6 MiB, 13.7 MiB less, with the same image and payload: 12.8 MiB of JIT and 0.9 MiB of WebAssembly.
- **No WebAssembly.** `WebAssembly` is `undefined`. `sk publish` refuses an app whose image names it, a `typeof WebAssembly` test included, and lists where; see [Troubleshooting](#at-build-time).
- **Same speed.** Measured below, the app runs as fast as on the normal workload.
- **No way back to the JIT.** `SK_APP_ENGINE=jit` stops the app with exit code 3 and a message that names the workload, instead of running `app.prod.js` in the interpreter. Compare against the JIT with a build that keeps `"publishJit"` on.
- **Images only.** `sk build` and `sk publish` refuse the `aot-nojit` workload without `--aot`, and the host refuses a payload that has no image.
- **Release builds.** `sk build` (debug) keeps the standard workload and the JIT, and so do macOS and Linux builds. `sk publish` fetches the appropriate no-JIT workload as needed, and `sk workload install` downloads it when you name it.

| Windows x64 AOT release workload | `publishJit` | `publishDevtools` |
| -------------------------------- | ------------ | ----------------- |
| `no-devtools`                    | `true`       | `false`           |
| `standard`                       | `true`       | `true`            |
| `aot-nojit`                      | `false`      | `false`           |
| `aot-nojit-devtools` (since 0.7) | `false`      | `true`            |

The `aot-nojit-devtools` workload keeps the interpreter for console expressions and runs the inspector from `inspector.image.dll` beside the exe. It needs both `"publishDevtools": true` in the packaged payload and that DLL. Leave `"publishSingleFile"` off for this combination. With the same Windows x64 release profile, devtools adds 1.82 MiB to the exe before the separate inspector image.

### What it costs (since 0.6)

Measured with `benches/nojit` in the Skathi repository, one Windows x64 machine, release builds of the same app compiled once by `sk-aot`. **aot** is the normal workload running the image, **aot-nojit** the new workload running the same image, and **jit** the normal workload with `SK_APP_ENGINE=jit`, for reference. Each cell is the median of 7 runs; each run is a new process, the three variants take turns in alternating order, and one discarded run per variant comes first. Inside a run, the RegExp and `eval` parts repeat 5 times after a warm-up and report their median.

| Median, ms                                                   | aot       | aot-nojit | jit       |
| ------------------------------------------------------------ | --------- | --------- | --------- |
| Start: process start to first frame                          | 370.3     | 344.4     | 358.8     |
| Scroll, 600 frames of a 100,000-row list: frame p50 / p95    | 6.1 / 6.2 | 6.1 / 6.3 | 6.1 / 6.3 |
| Scroll: Vue update per frame, total                          | 132.8     | 135.9     | 117.1     |
| RegExp: 8 kinds of match, replace and split over 20k lines   | 31.2      | 31.0      | 36.9      |
| `eval` and `new Function`: a loop, a module, 2,000 templates | 34.0      | 37.0      | 73.6      |

The two image columns stay within the spread of the runs. On the normal workload a process that runs an image has no JIT for WebAssembly either: it runs in its interpreter, about 20 times slower than under the JIT, so an app whose hot path is Wasm should not use AOT.

## Environment variables

| Variable                | Used by             | Effect                                                                                                                                                                                                                     |
| ----------------------- | ------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `SK_APP_ENGINE`         | the app             | `auto` (default) runs the image; `jit` runs `app.prod.js`. Since 0.6, the `aot-nojit` workload refuses `jit`.                                                                                                              |
| `SK_AOT_CACHE_DIR`      | the app, `sk prune` | Where images are expanded instead of `%LOCALAPPDATA%\skathi\native-aot\v1`. Must be absolute. Since 0.6 only hosts that embed a compressed inspector image expand one; published apps expand nothing.                      |
| `SK_AOT_MEMORY_PROFILE` | the app             | `default` or `compact`. `compact` caps two engine capacities: the structure heap at 256 MiB and the object address range at 1 GiB. These are capacities, not a limit on the app's memory; a large app can run out of them. |
| `SK_AOT_COMPILER`       | `sk`                | The `sk-aot.exe` to compile with, as `--aot-compiler` does.                                                                                                                                                                |
| `SK_HOST_OFFLINE`       | `sk`                | `1` forbids downloading `sk-aot` and workloads.                                                                                                                                                                            |

## When not to use it

- **Without a measured gain.** AOT is experimental, and in the Vue benchmark above it was slower than the JIT.
- **When the app or a dependency patches built-ins** and cannot stop.
- **When the hot path is WebAssembly.** It runs interpreted; see the [measurements](#what-it-costs).
- **When hot code is generated at run time**, such as a template engine or validator built on `new Function` with no mode that avoids it. That code is interpreted.
- **For builds you iterate on.** Each AOT build adds the check and the compile; use `sk dev` while you work.
