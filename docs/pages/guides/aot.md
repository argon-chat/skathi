---
title: Ahead-of-time compilation
label: AOT compilation
description: Compile a Windows build's JavaScript to a native image with sk publish --aot. How it works, what it changes, what it cannot run, and how to diagnose it.
section: Guides
order: 10
status: experimental
---

`sk publish --aot` compiles your app's JavaScript to machine code at build time and packs the result, a native image, into the exe. At start the host runs the image instead of evaluating the bundle. AOT is experimental, works on Windows x64 only, and today it is slower than the JIT in Skathi's own benchmark, so treat it as something to try and measure, not a default.

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
- **Size and build time.** The image is added beside `app.prod.js`, and each build adds the check and the compile.

</div>
</div>

## How it works

Without AOT, the exe carries the production bundle, `app.prod.js`, and JavaScriptCore runs it: it parses the bundle or reads its bytecode from a per-user cache, interprets it, and JIT-compiles the hot paths while the app runs. With `--aot`, `sk build` and `sk publish` do the compiling up front:

<div class="pipeline">
<p class="pipeline-phase">On your machine, during <code>sk build --aot</code> or <code>sk publish --aot</code></p>

1. **Sources** `src/**/*.vue`, `*.svelte`, `*.ts`

   The app's components and modules.

2. **Bundle** `app.prod.js`

   The production bundle, tree-shaken and minified, built as it is without AOT.

3. **Check** `sk check`

   Runs over the app and its dependencies. Errors stop the build; warnings are printed and the build goes on. See [sk check](#before-the-first-aot-build-sk-check).

4. **Compile** `sk-aot`

   `sk-aot` first wraps the bundle so that the host's globals (`document`, `fetch`, timers and the rest) are inputs the compiler makes no assumptions about. Bun's AOT frontend turns it into an intermediate representation, and the JavaScriptCore backend that the Skathi host is built with turns that into machine code.

5. **Native image** `app.image` + `app.native.json`

   The image and its manifest. `sk-aot` is pinned to one Bun revision and one JavaScriptCore revision, and only a host built from the same pair can load its images.

6. **Packed exe**

   The image, deflate-compressed, and the manifest are added to the app's payload. `app.prod.js` stays in the payload: the host checks the image against it and runs it on a reload.

<p class="pipeline-phase" data-tone="run">On every machine that runs the app, at each start</p>

7. **Host loads the image** `%LOCALAPPDATA%\skathi\native-aot\v1\<sha256>\`

   Before the JavaScript engine starts, the host checks the manifest against itself (compiler, engine, builtin table, the host's own startup scripts) and against the payload (the SHA-256 of `app.prod.js` and of the image). It expands the image once into a per-user cache keyed by its digest, hashes the cached file again on later starts, loads it, and runs the image's entry instead of evaluating `app.prod.js`.

</div>

The host's startup scripts and native globals are not part of the image; they run as they do without AOT.

## JIT and AOT compared

|                                              | JIT (default)                                                      | AOT (`--aot`)                                                                |
| -------------------------------------------- | ------------------------------------------------------------------ | ---------------------------------------------------------------------------- |
| At start                                     | Parses `app.prod.js` or reads its bytecode from the per-user cache | Checks the manifest, loads the native image and runs its entry               |
| While running                                | Interprets the bundle and JIT-compiles the hot paths               | Runs the image; the process has no JIT                                       |
| `eval`, `new Function`, devtools expressions | Run like the rest of the code                                      | Run in the interpreter                                                       |
| Built-in objects                             | Writable                                                           | Read-only: replacing their members throws a `TypeError`                      |
| Per-user cache                               | Bytecode                                                           | Expanded images in `%LOCALAPPDATA%\skathi\native-aot\v1\`; no bytecode cache |
| In the payload                               | `app.prod.js`                                                      | `app.prod.js`, the deflate-compressed image and its manifest                 |
| Build                                        | Bundle                                                             | Bundle, `sk check`, then `sk-aot`                                            |
| Development                                  | `sk dev`, with hot reload                                          | None: `sk dev` runs the JavaScript. `sk run --aot` tries the image           |
| Platforms                                    | Windows, macOS, Linux                                              | Windows x64                                                                  |
| Benchmark, first round                       | **437 ms**                                                         | 584 ms                                                                       |
| Benchmark, last round                        | **447 ms**                                                         | 550 ms                                                                       |

### Where performance stands

AOT is experimental, and this is its current state: in Skathi's benchmark the AOT build is slower than the JIT, both in the first round after the app starts and in the last round. The compiler is being worked on. For now, AOT is not a guaranteed speedup.

The benchmark lives in `benches/aot` in the Skathi repository: a Vue table that mounts, updates, swaps and clears rows. Each run starts the app and repeats the same round of work, and the table gives the round's total time, median of 3 runs on one Windows x64 machine, with the toolchain current Skathi pins (Bun `237bba51e`, JavaScriptCore `autobuild-3bc94211`). Both columns come from the same exe: the JIT column runs it with `SK_APP_ENGINE=jit`.

Measure your own app the same way: build it once with `--aot`, then start that exe with and without `SK_APP_ENGINE=jit`.

Skathi's sample apps give an idea of the other costs. Their uncompressed images are 2.7 to 3.5 MB, and they compile in 1.3 to 2.1 seconds, with `sk-aot` peaking at 104 to 121 MiB of memory, plus the time of `sk check`. `sk prune` removes the expanded images; apps expand them again at their next start.

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

The build summary has a `native` line with the image size and its size in the payload. With `--json`, the result has a `native` object with the same information. The intermediate files stay in `dist/<debug|release>/.sk/aot/`:

| File              | What it is                                         |
| ----------------- | -------------------------------------------------- |
| `native-input.js` | The wrapped bundle, exactly what `sk-aot` compiled |
| `app.ir`          | The intermediate representation                    |
| `app.image`       | The native image                                   |
| `app.native.json` | The manifest                                       |

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
- **Reload.** The image's entry runs once per process. `location.reload()` evaluates `app.prod.js` from the payload instead.
- **A host that cannot load the image.** See the warning below.

> **Warning:** There is no fallback. When the manifest does not match the host, or the cache cannot be written, the app shows the reason in a message box and exits with code 3.

> **Tip:** To run the same exe without its image, set `SK_APP_ENGINE=jit`. The host then evaluates `app.prod.js` under the JIT and logs that it did. This is how you tell whether a bug or a slowdown comes from AOT.

## Debugging

- **Develop with `sk dev`.** It runs the JavaScript with hot reload; there is no AOT in development. Use `sk run --aot` to try the image.
- **Read the host log.** `sk run` prints the app's output in the terminal; for `sk build`, `--console` keeps a console window. A run from the image logs two lines:

  ```text
  [native-aot] selected application image sha256:… (entry 1, memory Default)
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

### At start or while running

- `this app's native image cannot run on this host`

  **Cause:** The image does not match the exe it runs in, or the image cache cannot be created. The message box gives the reason.

  **Fix:** Rebuild the app with the `sk` that matches the workload. If the reason names `LOCALAPPDATA` or the cache, set `SK_AOT_CACHE_DIR` to an absolute folder the user can write.

- A `TypeError` the JIT build does not throw

  **Cause:** The code replaces a built-in on a path `sk check` did not see.

  **Fix:** Run the app with `SK_APP_ENGINE=jit` to confirm, then find the assignment from the stack.

</div>

## Platform support

| Platform                               | AOT                                                                        |
| -------------------------------------- | -------------------------------------------------------------------------- |
| Windows x64                            | Build and run. Every Windows workload of a Skathi release can load images. |
| macOS, Linux                           | No. `sk` refuses `--aot` for these targets.                                |
| Building on macOS or Linux for Windows | No. `sk-aot` runs on Windows x64 only.                                     |

A workload you build yourself with `sk workload build` loads images only when it is built with the `native-aot` cargo feature.

## Environment variables

| Variable                | Used by             | Effect                                                                                                                                                                                                                     |
| ----------------------- | ------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `SK_APP_ENGINE`         | the app             | `auto` (default) runs the image; `jit` runs `app.prod.js`.                                                                                                                                                                 |
| `SK_AOT_CACHE_DIR`      | the app, `sk prune` | Where images are expanded instead of `%LOCALAPPDATA%\skathi\native-aot\v1`. Must be absolute.                                                                                                                              |
| `SK_AOT_MEMORY_PROFILE` | the app             | `default` or `compact`. `compact` caps two engine capacities: the structure heap at 256 MiB and the object address range at 1 GiB. These are capacities, not a limit on the app's memory; a large app can run out of them. |
| `SK_AOT_COMPILER`       | `sk`                | The `sk-aot.exe` to compile with, as `--aot-compiler` does.                                                                                                                                                                |
| `SK_HOST_OFFLINE`       | `sk`                | `1` forbids downloading `sk-aot` and workloads.                                                                                                                                                                            |

## When not to use it

- **Without a measured gain.** AOT is experimental, and in the Vue benchmark above it was slower than the JIT.
- **When the app or a dependency patches built-ins** and cannot stop.
- **When hot code is generated at run time**, such as a template engine or validator built on `new Function` with no mode that avoids it. That code is interpreted.
- **For builds you iterate on.** Each AOT build adds the check and the compile; use `sk dev` while you work.
