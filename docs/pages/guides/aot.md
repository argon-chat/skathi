---
title: Ahead-of-time compilation
label: AOT compilation
description: Compile JavaScript to a native image with sk publish --aot on Windows x64 or Apple Silicon. How it works, what it changes, and how to diagnose it.
section: Guides
order: 10
status: experimental
---

`sk publish --aot` compiles your app's JavaScript to machine code at build time. The host runs the resulting native image at startup instead of evaluating the bundle. AOT is experimental and supports Windows x64 and macOS arm64. In Skathi's Windows benchmark, the JIT is faster.

If you embed Skathi in your own executable, see [AOT for your own executable](/embedding/shipping/#aot-for-your-own-executable).

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
- **Platforms.** Build Windows x64 images on Windows x64 and macOS arm64 images on Apple Silicon.
- **Read-only built-ins.** Code that replaces a member of `Array.prototype` or a global such as `Map` throws a `TypeError`.
- **No JIT in the process.** `eval`, `new Function` and devtools expressions run in the interpreter.
- **No fallback.** A host that cannot load the image shows why and exits with code 3.
- **Size and build time.** The uncompressed image ships with the app, and each build adds the check and the compile.

</div>
</div>

## How it works

Without AOT, the exe carries the production bundle, `app.prod.js`, and JavaScriptCore runs it: it parses the bundle or reads its bytecode from a per-user cache, interprets it, and JIT-compiles the hot paths while the app runs. With `--aot`, `sk build` and `sk publish` do the compiling up front:

<div class="pipeline">
<p class="pipeline-phase">On your machine, during <code>sk build --aot</code> or <code>sk publish --aot</code></p>

1. **Sources** `src/**/*.vue`, `*.svelte`, `*.ts`

   The app's components and modules.

2. **Bundle** `app.prod.js`

   The production bundle, tree-shaken and minified, built as it is without AOT. Both builds write ASCII source so JavaScriptCore can keep one byte per character. Unicode text, regular expression patterns and tagged template strings retain their values. Set `SK_BUNDLE_UTF8=1` to keep the original source encoding.

3. **Check** `sk check`

   Runs over the app and its dependencies. Errors stop the build; warnings are printed and the build goes on. See [sk check](#before-the-first-aot-build-sk-check).

4. **Compile** `sk-aot`

   `sk-aot` first wraps the bundle so that the host's globals (`document`, `fetch`, timers and the rest) are inputs the compiler makes no assumptions about. Bun's AOT frontend turns it into an intermediate representation, and the JavaScriptCore backend that the Skathi host is built with turns that into machine code.

5. **Native image** `app.image` + `app.native.json`

   The image and its manifest. `sk-aot` is pinned to one Bun revision and one JavaScriptCore revision, and only a host built from the same pair can load its images.

6. **Container** `app.image.dll` or `app.image.dylib`

   The image ships uncompressed in page-aligned sections. On Windows, the container is a DLL beside the exe. With [`"publishSingleFile": true`](#one-file-publishsinglefile) the sections go into the exe. On macOS, the container is a Mach-O dylib in `Contents/Frameworks`. The payload holds the image manifest and `app.prod.js`. The host checks the image against the bundle and uses the bundle on reload.

<p class="pipeline-phase" data-tone="run">On every machine that runs the app, at each start</p>

7. **Host maps the image where it lies**

   Before the JavaScript engine starts, the host checks the compiler, engine, builtin table and startup scripts in the manifest, along with the SHA-256 of `app.prod.js` and of the image. The operating system loads the container as a shared library and applies its code-signing policy. The host hashes the mapped image and runs its entry there, without expanding it or creating a disk cache for it.

</div>

The host's startup scripts and native globals are not part of the image; they run as they do without AOT.

Workloads carry only the image loader: JavaScriptCore's image compiler is in `sk-aot` alone.

## JIT and AOT compared

|                                              | JIT (default)                                                      | AOT (`--aot`)                                                      |
| -------------------------------------------- | ------------------------------------------------------------------ | ------------------------------------------------------------------ |
| At start                                     | Parses `app.prod.js` or reads its bytecode from the per-user cache | Checks the manifest, loads the native image and runs its entry     |
| While running                                | Interprets the bundle and JIT-compiles the hot paths               | Runs the image; the process has no JIT                             |
| `eval`, `new Function`, devtools expressions | Run like the rest of the code                                      | Run in the interpreter                                             |
| Built-in objects                             | Writable                                                           | Read-only: replacing their members throws a `TypeError`            |
| Per-user cache                               | Bytecode                                                           | None                                                               |
| In the payload                               | `app.prod.js`                                                      | `app.prod.js` and the image's manifest                             |
| Image in the distribution                    |                                                                    | Windows DLL, macOS dylib, or sections in a Windows single-file exe |
| Build                                        | Bundle                                                             | Bundle, `sk check`, then `sk-aot`                                  |
| Development                                  | `sk dev`, with hot reload                                          | None: `sk dev` runs the JavaScript. `sk run --aot` tries the image |
| Platforms                                    | Windows, macOS, Linux                                              | Windows x64 and macOS arm64                                        |
| Benchmark, first round                       | **437 ms**                                                         | 584 ms                                                             |
| Benchmark, last round                        | **447 ms**                                                         | 550 ms                                                             |

### Where performance stands

AOT is experimental, and this is its current state: in Skathi's benchmark the AOT build is slower than the JIT, both in the first round after the app starts and in the last round. The compiler is being worked on. For now, AOT is not a guaranteed speedup.

The benchmark lives in `benches/aot` in the Skathi repository: a Vue table that mounts, updates, swaps and clears rows. Each run starts the app and repeats the same round of work, and the table gives the round's total time, median of 3 runs on one Windows x64 machine, with the toolchain current Skathi pins (Bun `237bba51e`, JavaScriptCore `autobuild-3bc94211`). Both columns come from the same exe: the JIT column runs it with `SK_APP_ENGINE=jit`.

Measure your own app the same way: build it once with `--aot`, then start that exe with and without `SK_APP_ENGINE=jit`.

Skathi's sample apps give an idea of the other costs. Their uncompressed images are 2.7 to 3.5 MB, and they compile in 1.3 to 2.1 seconds, with `sk-aot` peaking at 104 to 121 MiB of memory, plus the time of `sk check`. The image ships uncompressed, so the app on disk grows by the image's full size. Apps built before 0.6 expanded their image into `%LOCALAPPDATA%\skathi\native-aot\v1\`; `sk prune` removes those folders.

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

Use `--no-aot` to build for Intel Macs, Linux, or from a machine that cannot run the target's `sk-aot`.

### Where sk-aot comes from

The first AOT build downloads `sk-aot` from the GitHub release of the `sk` version you run and checks its SHA-256 against the release manifest. Windows uses `sk-aot-windows-x64.exe.gz` and caches it under `%LOCALAPPDATA%\skathi\hosts\<version>\aot\`. macOS uses `sk-aot-macos-arm64.gz` and caches it under `~/Library/Caches/skathi/hosts/<version>/aot/`. Before compiling, `sk` checks that the compiler matches the workload. `sk prune` removes compilers for other versions.

### What the build leaves behind

The build summary's `native` line reports the image size and location. With `--json`, `native.container` is the path of the DLL or dylib, or `null` for an image in a Windows single-file exe. The intermediate files stay in `dist/<debug|release>/.sk/aot/`.

| File              | What it is                                         |
| ----------------- | -------------------------------------------------- |
| `native-input.js` | The wrapped bundle, exactly what `sk-aot` compiled |
| `app.ir`          | The intermediate representation                    |
| `app.image`       | The native image                                   |
| `app.native.json` | The manifest                                       |

### What you ship

On Windows, `sk publish --aot` writes the exe and its companion files.

| File                  | What it is                                                                                             |
| --------------------- | ------------------------------------------------------------------------------------------------------ |
| `app.image.dll`       | The native image, uncompressed, in a DLL with no entry point and no imports. The exe loads it at start |
| `inspector.image.dll` | The devtools inspector's image, when `"publishDevtools": true` and the workload has one                |
| `icu.skpk`            | The ICU data, as without AOT                                                                           |
| `*.skpk`              | The asset packs set in `"packs"`, as without AOT                                                       |

Keep these files together: the exe looks for them in its own folder, so the folder can be moved or installed anywhere as a whole. A missing or changed `app.image.dll` stops the app with a message that names the file. Sign `app.image.dll` with the exe; it is a valid PE file, and signing it does not change the image. The container is rebuilt from the same template on each build, so an unchanged image gives a byte-identical `app.image.dll` and its signature can be reused.

Each part of the image lands in its own section, protected as the engine needs it: `.skapp` (header, read-only), `.skappx` (code, read and execute) and `.skappd` (data, read-only); the inspector's are `.skins`, `.skinsx` and `.skinsd`. `dumpbin /headers app.image.dll` lists them. The container also has an exception table over the code, so debuggers and crash dumps walk the stack through the image; its frames show as `app.image.dll+offset`.

#### macOS bundle

Ship the entire `.app` from `sk publish --aot`.

| Bundle path                                 | Contents                                                |
| ------------------------------------------- | ------------------------------------------------------- |
| `Contents/MacOS/<name>`                     | The host executable                                     |
| `Contents/Resources/app.skpk`               | The JavaScript bundle, image manifest and app resources |
| `Contents/Resources/*.skpk`                 | Asset packs, when configured                            |
| `Contents/Frameworks/app.image.dylib`       | The application image                                   |
| `Contents/Frameworks/inspector.image.dylib` | The image for the inspector, when shipped               |

The Mach-O container maps the application image on 16 KiB boundaries. Its code pages have read and execute permissions and its data pages are read-only. The compact unwind records cover the compiled functions so a stack walker can step through image frames.

On Apple Silicon, `sk-aot` uses wide references for JavaScript objects. This lets the same image run on the full and no-JIT workloads even when JavaScriptCore cannot reserve its aligned heap range.

On a Mac, `sk publish` signs the image libraries before signing the bundle. For local use, keep the default ad hoc signature or pass `--sign -`. Rebuilding in the same output directory with the same image and signing identity lets `sk` reuse the existing library signature. Use a signing identity with a Team ID for hardened runtime. On a physical Mac running macOS 26.4, library validation rejected an ad hoc hardened app loading its ad hoc dylib because neither had a Team ID. The default ad hoc build runs with ordinary code signing. A no-JIT build needs no JIT or unsigned executable memory entitlement.

For distribution outside the Mac App Store, pass your `Developer ID Application` identity with `--sign` or `macos.sign` in `sk.config.json`. `sk` enables hardened runtime and requests a secure timestamp for that signature. Verify the signed bundle with `codesign --verify --deep --strict dist/release/notes.app`. Submit it to Apple with `notarytool`, staple the accepted ticket, then assess the result with `spctl --assess --type execute -vv dist/release/notes.app`. [Apple’s notarization documentation](https://developer.apple.com/documentation/security/notarizing-macos-software-before-distribution)

### One file: publishSingleFile

```json title="sk.config.json"
{
  "name": "notes",
  "aot": true,
  "publishSingleFile": true
}
```

With `"publishSingleFile": true`, `sk publish` writes the exe alone. The image's sections, and the inspector's when there is one, are added to the exe, and the ICU data and the packs go into its payload. Windows maps the image as part of the exe, and the engine runs the code where it lies. One signature then covers everything. `sk build` ignores the option.

Remove `"publishSingleFile": true` when publishing for macOS. `sk publish` requires the `.app` layout there so `codesign` can sign the image libraries as nested code and seal the resources with the bundle.

`sk publish` also extends the exe's exception table over the image's code: it writes a copy of the table with the image's entries added in a section of its own, `.skpdata`, and points the exe at it. Debuggers and crash dumps walk the stack through the image as they do beside the exe; its frames show as offsets into the exe. The copy adds the size of the host's own table to the exe, about 2 MiB.

Measured on one Windows x64 machine with a small Vue app, `icu.skpk` with every locale, 20 runs per variant in alternating order, after one discarded run each. **Start** is process start to the app's first mount, median. **Cold** means a per-user cache folder that is empty at each start, as on a new machine; **warm** one that earlier starts have filled. **On disk** is everything the app puts on disk: the exe, the files beside it and, before 0.6, the expanded image.

| aot-nojit workload, 2.6 MiB image                   | Start, cold | Start, warm | On disk               |
| --------------------------------------------------- | ----------- | ----------- | --------------------- |
| Image in the payload, expanded to disk (before 0.6) | 409 ms      | 390 ms      | 86.8 MiB + 2.6 MiB    |
| `app.image.dll` beside the exe                      | 411 ms      | 421 ms      | 88.4 MiB              |
| `"publishSingleFile": true`                         | 407 ms      | 420 ms      | 90.3 MiB, in one file |

The start times stay within the spread of the runs, whose middle half spans about 50 ms. Checking the image's SHA-256 takes 5.8 ms of each start for this image, as it did before 0.6. The files you ship grow by the difference between the image and its compressed form, 1.6 MiB here, plus the copy of the exception table in one file, and nothing is written at the first start.

## Before the first AOT build: sk check

Run `sk check` on the app before you turn AOT on. An AOT build runs the same check and stops on its errors. Its codes come in six ranges:

| Codes    | Severity                   | Meaning                                                                                                                                                                     |
| -------- | -------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `SK1xxx` | Error: the AOT build stops | Breaks under AOT. See [Built-ins are read-only](#built-ins-are-read-only).                                                                                                  |
| `SK2xxx` | Warning                    | Works under AOT, but runs slower. See [Code made at run time](#code-made-at-run-time) and [Tight loops](#tight-loops).                                                      |
| `SK3xxx` | Warning                    | Typing quality: values the code passes around without a type.                                                                                                               |
| `SK4xxx` | Warning                    | CSS the renderer does not apply, in the app's stylesheets, `<style>` blocks and `style` attributes; one summary per package.                                                |
| `SK5xxx` | Error or warning           | Vue and Svelte features that do not work or work differently. See [Vue support](/guides/vue-support/) and [Svelte support](/guides/svelte-support/).                        |
| `SK6xxx` | Error or warning           | Permissions: a [`dlopen()`](/std/dlopen/) of a library that sk.config.json does not allow, or a dependency that imports `skathi:ffi`. `check.allow` cannot hide the errors. |

Every code has a page in the [`sk check` reference](/sk-check/). `sk explain <code>` prints the same in the terminal, and `sk explain --list` lists them all. `sk check --deny-warnings` also fails on warnings, for CI.

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
- Replacing the globals the engine leaves replaceable, among them `Promise`, `Date` and `Error`.

The methods of `console` are read-only in an image as well (`sk check` reports it as SK1001). A logger that wraps `console.error` to keep the last lines for a bug report throws at that assignment, and since it usually runs before the app is mounted, the window stays empty. Wrap only when the app does not run from an image, and call your own function where the app logs:

```ts
import { Runtime } from "skathi:runtime";

if (!Runtime.isAot()) {
  const original = console.error.bind(console);
  console.error = (...args: unknown[]) => {
    record(args);
    original(...args);
  };
}
```

`sk check` does not report an assignment under `if (!Runtime.isAot())`. It does check a computed key that is one of a few literals, such as `console[level] = …` in a loop over `["error", "warn"]`.

When the entry of a built app throws before it has mounted anything, with or without an image, the host opens its error dialog with the message instead of leaving an empty window. The line is in the log as `[crash] the app's entry threw and mounted nothing`.

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

### Tight loops

A JIT compiles a loop after watching it run, so it knows which numbers were always integers and which arrays stayed dense. An image is compiled before the app starts and assumes less. In most code that does not show. It shows in a loop whose body is only arithmetic and element access, such as an emulator's or a codec's inner loop, where two warnings point at the shapes that cost the most:

- **SK2005:** a `number` parameter bounds the loop. Take an integer copy first, `const end = n | 0`, and compare with that. The copy is only right for a bound that is always an integer below 2^31.
- **SK2006:** the loop writes into a `number[]` through a computed index. Keep those numbers in an `Int32Array`, `Uint8Array` or `Float64Array`.

Both are reported in TypeScript only, and only for a loop with no calls in its body other than `Math`'s. A loop that runs a few times is not worth changing: list the file under the code in `check.allow`.

The warnings come from a measurement of a RISC-V interpreter written in TypeScript, on Windows x64 hosts of Skathi 0.6.1. Its instruction loop, in millions of emulated instructions a second, by what ran it:

| Runs the loop      | MIPS |
| ------------------ | ---- |
| Interpreter        | 11   |
| JIT, baseline tier | 55   |
| JIT, middle tier   | 249  |
| JIT, top tier      | 311  |
| AOT image          | 331  |

So an image runs such a loop at the speed of the JIT's top tier, from the first pass, and code the image does not hold runs 30 times slower. Four shapes were slower in the image than in the top tier, in nanoseconds per pass:

| Shape                                                            | JIT, top tier | AOT image | AOT image, rewritten                        |
| ---------------------------------------------------------------- | ------------- | --------- | ------------------------------------------- |
| `for (let i = 0; i < n; i++)` with `n: number`                   | 0.18          | 0.61      | 0.18 with `const end = n \| 0`              |
| The same loop computing `(i * 7) >>> 3`                          | 0.21          | 0.66      | 0.21 with `Math.imul` and the integer bound |
| Interpreter registers in a `number[]`, read and written by index | 1.1           | 2.93      | 1.25 in an `Int32Array`                     |
| A call to a small function                                       | 0.20          | 0.46      |                                             |

`sk check` reports the first three. A small function called from a hot loop costs about twice what it does under the JIT; when a profile shows such a call, write its body in the loop.

One more result has no explanation yet and no warning: in a small interpreter loop, declaring the countdown as `let left = n | 0` instead of `let left = n` made the image 2.3 times slower (2.94 against 1.25 ns), while the same change did nothing in the full emulator. Measure before and after when you rewrite a loop for the image.

### Typing warnings

The SK3 warnings measure how much of the code is typed: dependencies without declarations, `any` in what the app imports, `any` and `@ts-ignore` in the app's own code. They do not stop a build. The compiler does not use types: `sk build --aot` compiles the minified JavaScript bundle.

## What still runs without the image

The bundle runs from the image, and nothing falls back to the JIT. Once a process has loaded an image, it has no JIT:

- **Run-time code.** `eval`, `new Function` and expressions evaluated from devtools run in the interpreter.
- **WebAssembly.** Modules compile and run in the WebAssembly interpreter, many times slower than under the JIT. The [`aot-nojit` workload](#ship-without-the-jit) has no WebAssembly at all.
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

- `AOT builds require Windows x64 or macOS arm64` or `The macos AOT compiler runs on macOS on Apple Silicon`

  **Cause:** The build targets Linux, or `sk` runs on an OS or architecture that cannot run the target's compiler.

  **Fix:** Compile on the target's supported platform, or build with `--no-aot`.

- `"publishSingleFile": true has no macOS form`

  **Fix:** Drop the setting for macOS and distribute the resulting `.app` intact.

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

  **Fix:** Allow the download once, or pass `--aot-compiler <executable>`.

- `--aot --payload-only needs --workload <exe> to compile against your embedder's native AOT loader`

  **Fix:** Pass the embedder executable with `--workload`, or remove `--payload-only` to build a complete app.

- `the aot-nojit workload has no JIT and runs only native images`

  **Cause:** The build names the `aot-nojit` workload but compiles no image.

  **Fix:** Build with `--aot`, or with another workload.

- `this app's native image uses WebAssembly, which the aot-nojit workload leaves out`

  **Cause:** The app, or a dependency, names `WebAssembly`. The lines after the message give each place, with its line and column in `app.prod.js`. A test such as `typeof WebAssembly` counts too: the image is compiled for an engine that has it, and may have answered the test ahead of time.

  **Fix:** Drop that use, or the dependency. Otherwise publish on the workload with the JIT: remove `"publishJit": false`.

- `the aot-nojit workload leaves out WebAssembly, and the sk-aot that compiled this app does not record what its image uses`

  **Cause:** `sk-aot` comes from a Skathi version before 0.6, which does not say what an image uses.

  **Fix:** Use the `sk-aot` of the release that matches the workload: remove `--aot-compiler` and `SK_AOT_COMPILER` and let `sk` fetch it.

### At start or while running

- `this app's native image cannot run on this host`

  **Cause:** The image does not match the exe it runs in. The message box gives the reason.

  **Fix:** Rebuild the app with the `sk` that matches the workload.

- `this app cannot start: … app.image.dll is missing`, `… app.image.dylib is missing`, or `… differs from the one this app was built with`

  **Cause:** The container is missing, invalid, or from a different build. On Windows, the single-file exe's image sections may have changed.

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

| Target              | Build and run                                             |
| ------------------- | --------------------------------------------------------- |
| Windows x64         | On Windows x64. Release workloads can load native images. |
| macOS arm64         | On macOS with Apple Silicon and an arm64 `sk-aot`.        |
| macOS x64 and Linux | Use `--no-aot`.                                           |

A workload you build yourself with `sk workload build` loads images only when it is built with the `native-aot` cargo feature.

## Ship without the JIT

A process that runs an image has the JIT disabled. To also leave it out of the executable, set `"publishJit": false` for `sk publish --aot`. The `aot-nojit` workload links JavaScriptCore without the JIT and WebAssembly. It is available on Windows x64 and macOS arm64.

```json title="sk.config.json"
{
  "name": "notes",
  "aot": true,
  "publishJit": false
}
```

- **Smaller.** On Windows x64 the workload is 73.9 MiB instead of 87.6 MiB with the same image and payload. The reduction is 12.8 MiB of JIT and 0.9 MiB of WebAssembly.
- **No WebAssembly.** `WebAssembly` is `undefined`. `sk publish` refuses an app whose image names it, a `typeof WebAssembly` test included, and lists where; see [Troubleshooting](#at-build-time).
- **Interpreter.** `eval` and `new Function` execute in the interpreter on either AOT workload. The Windows measurements below compare the two workloads.
- **No way back to the JIT.** `SK_APP_ENGINE=jit` stops the app with exit code 3 and a message that names the workload, instead of running `app.prod.js` in the interpreter. Compare against the JIT with a build that keeps `"publishJit"` on.
- **Images only.** `sk build` and `sk publish` refuse the `aot-nojit` workload without `--aot`, and the host refuses a payload that has no image.
- **Release builds.** `sk build` (debug) uses the standard workload by default. `sk publish` fetches the no-JIT workload as needed. Run `sk workload install aot-nojit aot-nojit-devtools` to install both variants ahead of time.

| AOT release workload | `publishJit` | `publishDevtools` |
| -------------------- | ------------ | ----------------- |
| `no-devtools`        | `true`       | `false`           |
| `standard`           | `true`       | `true`            |
| `aot-nojit`          | `false`      | `false`           |
| `aot-nojit-devtools` | `false`      | `true`            |

The `aot-nojit-devtools` workload keeps the interpreter for console expressions. Set `"publishDevtools": true` to ship and enable its inspector. On Windows it runs from `inspector.image.dll` beside the exe, so leave `"publishSingleFile"` off. On macOS it runs from `Contents/Frameworks/inspector.image.dylib`. With the same Windows x64 release profile, devtools adds 1.82 MiB to the exe before the separate inspector image.

### What it costs

Measured with `benches/nojit` in the Skathi repository, one Windows x64 machine, release builds of the same app compiled once by `sk-aot`. **aot** is the normal workload running the image, **aot-nojit** the new workload running the same image, and **jit** the normal workload with `SK_APP_ENGINE=jit`, for reference. Each cell is the median of 7 runs; each run is a new process, the three variants take turns in alternating order, and one discarded run per variant comes first. Inside a run, the RegExp and `eval` parts repeat 5 times after a warm-up and report their median.

| Median, ms                                                   | aot       | aot-nojit | jit       |
| ------------------------------------------------------------ | --------- | --------- | --------- |
| Start: process start to first frame                          | 370.3     | 344.4     | 358.8     |
| Scroll, 600 frames of a 100,000-row list: frame p50 / p95    | 6.1 / 6.2 | 6.1 / 6.3 | 6.1 / 6.3 |
| Scroll: Vue update per frame, total                          | 132.8     | 135.9     | 117.1     |
| RegExp: 8 kinds of match, replace and split over 20k lines   | 31.2      | 31.0      | 36.9      |
| `eval` and `new Function`: a loop, a module, 2,000 templates | 34.0      | 37.0      | 73.6      |

The two image columns stay within the spread of the runs. On the normal workload a process that runs an image has no JIT for WebAssembly either: it runs in its interpreter, about 20 times slower than under the JIT, so an app whose hot path is Wasm should not use AOT.

### macOS arm64 measurements

Skathi `86194799` ran on a GitHub-hosted Apple M1 virtual machine (VirtualMac2,1, 7.0 GiB RAM) with macOS 15.8.2. The empty app appends a single `<div>`. Showcase opens its home screen. For each app, JIT and regular AOT used the same release host. Both AOT variants used the compact memory profile. The no-JIT workload omits the JIT and WebAssembly.

Startup is the time from process launch until Skathi starts processing the first frame. The first launch uses a fresh app data directory. Packaging smoke tests had already opened the apps. macOS kept its filesystem cache between launches. The warm startup and memory values are medians of seven new processes per variant, reusing that variant’s app data. Each run rotates which variant starts first. Memory is process RSS and physical footprint, sampled after the first frame marker and again five seconds after it. Showcase continues its home-screen animation during the five seconds without input.

| App      | Engine          | Fresh profile (ms) | Warm (ms) | After first frame, RSS / footprint (MiB) | At 5 s, RSS / footprint (MiB) | Bundle, logical / allocated (MiB) |
| -------- | --------------- | -----------------: | --------: | ---------------------------------------: | ----------------------------: | --------------------------------: |
| Empty    | JIT             |              413.9 |     520.6 |                              77.8 / 26.1 |                   82.2 / 25.8 |                       94.5 / 94.5 |
| Empty    | AOT             |              334.4 |     479.6 |                              77.0 / 26.5 |                   82.1 / 26.6 |                       95.5 / 95.5 |
| Empty    | AOT without JIT |              303.5 |     434.0 |                              76.6 / 26.5 |                   81.7 / 26.5 |                       77.3 / 77.3 |
| Showcase | JIT             |              691.2 |     483.5 |                             115.4 / 70.3 |                  129.5 / 80.0 |                       96.3 / 96.3 |
| Showcase | AOT             |             1145.2 |     358.5 |                             101.5 / 61.4 |                  120.5 / 96.8 |                     100.9 / 100.9 |
| Showcase | AOT without JIT |              895.5 |     291.2 |                             101.1 / 57.9 |                  119.4 / 96.0 |                       82.7 / 82.7 |

Showcase scrolls its main content for 600 animation frames per run. Each column is the median of seven runs. The intervals measure the pacing of animation callbacks. The last column counts intervals over 33.33 ms in a 600-frame run.

| Engine          | Frame p50 (ms) | Frame p95 (ms) | Frames over 33.33 ms |
| --------------- | -------------: | -------------: | -------------------: |
| JIT             |           16.6 |           20.3 |                    0 |
| AOT             |           16.6 |           20.2 |                    0 |
| AOT without JIT |           16.6 |           20.2 |                    0 |

The no-JIT Showcase bundle used 13.6 MiB less disk space than JIT. At five seconds the median RSS fell from 129.5 to 119.4 MiB, while the median physical footprint rose from 80.0 to 96.0 MiB. Across runs the footprint ranged from 79.2 to 110.4 MiB under JIT and from 70.1 to 101.6 MiB under AOT without JIT while Showcase animated. The p95 scroll intervals differed by 0.1 ms on this virtual display.

[Measured SDK revision](https://github.com/argon-chat/skathi/commit/8619479954e87acbe378915815dc1d61b88cee28) · [GitHub Actions run](https://github.com/argon-chat/skathi/actions/runs/44914425380) · [Raw samples and published apps](https://github.com/argon-chat/skathi/actions/runs/44914425380/artifacts/14712114085)

## Environment variables

| Variable                | Used by             | Effect                                                                                                                                                                                                                                                                                                                                                                                                             |
| ----------------------- | ------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| `SK_APP_ENGINE`         | the app             | `auto` (default) runs the image; `jit` runs `app.prod.js`. The `aot-nojit` workload refuses `jit`.                                                                                                                                                                                                                                                                                                                 |
| `SK_AOT_CACHE_DIR`      | the app, `sk prune` | Where images are expanded instead of `%LOCALAPPDATA%\skathi\native-aot\v1`. Must be absolute. Only hosts that embed a compressed inspector image expand one; published apps expand nothing.                                                                                                                                                                                                                        |
| `SK_AOT_MEMORY_PROFILE` | the app             | `compact` (default) or `default`. `compact` sets the structure heap capacity to 256 MiB and requests a 1 GiB range for JavaScript objects. `default` keeps the engine's own sizes, which commit about 325 MB more on Windows without touching the pages; choose it when an app runs out of either capacity. Windows images need a range for objects of either size. macOS arm64 images can use objects outside it. |
| `SK_AOT_COMPILER`       | `sk`                | The `sk-aot.exe` to compile with, as `--aot-compiler` does.                                                                                                                                                                                                                                                                                                                                                        |
| `SK_HOST_OFFLINE`       | `sk`                | `1` forbids downloading `sk-aot` and workloads.                                                                                                                                                                                                                                                                                                                                                                    |

### Commit charge on Windows

A process running an image commits more private bytes than the same app under the JIT, in pages it mostly never touches, so its working set is not higher. With the `compact` profile the difference is about 35 to 50 MB. Most of it is one block of about 58 MB for each JavaScript VM, a worker's included: the engine's property-cache tables for compiled code, sized for their limit. Windows charges commit for the whole mapping at once, where macOS and Linux charge a page when it is first written. No option changes its size.

## When not to use it

- **Without a measured gain.** AOT is experimental, and in the Vue benchmark above it was slower than the JIT.
- **When the app or a dependency patches built-ins** and cannot stop.
- **When the hot path is WebAssembly.** It runs interpreted; see the [measurements](#what-it-costs).
- **When hot code is generated at run time**, such as a template engine or validator built on `new Function` with no mode that avoids it. That code is interpreted.
- **For builds you iterate on.** Each AOT build adds the check and the compile; use `sk dev` while you work.
