---
title: Ahead-of-time compilation
label: AOT compilation
description: Compile a Windows build's JavaScript to a native image with sk publish --aot. How it works, what it changes, what it cannot run, and how to diagnose it. Experimental.
section: Guides
order: 10
---

Without AOT, the exe carries the production bundle, `app.prod.js`, and JavaScriptCore runs it: it parses the bundle or reads its bytecode from a per-user cache, interprets it, and JIT-compiles the hot paths while the app runs. With `--aot`, `sk build` and `sk publish` compile the bundle to machine code at build time. The result, a native image, is packed into the exe, and the exe runs the image instead of the bundle. AOT is experimental and works on Windows x64 only.

## How it works

```text
src/**/*.vue, *.svelte, *.ts
   │  bundle: tree-shaken, minified (as without AOT)
   ▼
app.prod.js
   │  sk check: errors stop the build
   │  sk-aot: Bun's AOT frontend → intermediate representation → machine code
   ▼
app.image + app.native.json    packed into the exe, beside app.prod.js
   │  first start: expanded to %LOCALAPPDATA%\skathi\native-aot\v1\<sha256>\
   ▼
the host checks the manifest, loads the image, runs its entry
```

1. **Bundle.** The production bundle is built as it is without AOT.
2. **Check.** [`sk check`](#before-the-first-aot-build-sk-check) runs over the app and its dependencies. Errors stop the build; warnings are printed and the build goes on.
3. **Compile.** `sk-aot` compiles the bundle. It first wraps the bundle so that the host's globals (`document`, `fetch`, timers and the rest) are inputs the compiler makes no assumptions about. Bun's AOT frontend then turns it into an intermediate representation, and the JavaScriptCore backend that the Skathi host is built with turns that into machine code. `sk-aot` is pinned to one Bun revision and one JavaScriptCore revision, and only a host built from the same pair can load its images.
4. **Package.** The image, deflate-compressed, and a manifest are added to the app's payload. `app.prod.js` stays in the payload: the host checks the image against it and runs it on a reload (see [What still runs without the image](#what-still-runs-without-the-image)).
5. **Start.** Before the JavaScript engine starts, the host checks the manifest against itself (compiler, engine, builtin table, the host's own startup scripts) and against the payload (the SHA-256 of `app.prod.js` and of the image). It expands the image once into a per-user cache keyed by its digest, hashes the cached file again on later starts, loads it, and runs the image's entry instead of evaluating `app.prod.js`. The host's startup scripts and native globals are not part of the image; they run as they do without AOT.

## Turn it on

For one build, pass `--aot`:

```sh
bunx sk publish --aot
```

`sk build --aot` makes a debug build with devtools, and `sk run --aot` builds and starts it. To compile every build, set `"aot"` in `sk.config.json`:

```json title="sk.config.json"
{
  "name": "notes",
  "aot": true
}
```

`--no-aot` turns it off for one build. `sk dev` ignores the setting: it always runs the JavaScript, with hot reload. An app with `"aot": true` needs `--no-aot` to build for macOS or Linux.

The first AOT build downloads `sk-aot` from the GitHub release of the `sk` version you run (`sk-aot-windows-x64.exe.gz`), checks its SHA-256 against that release's list, and caches it under `%LOCALAPPDATA%\skathi\hosts\<version>\aot\`. Before compiling, `sk` checks that `sk-aot` targets the same engine as the workload it stamps. `sk prune` removes the `sk-aot` of other versions.

The build summary has a `native` line with the image size and its size in the payload. The intermediate files stay in `dist/<debug|release>/.sk/aot/`: `native-input.js` (the wrapped bundle sk-aot compiled), `app.ir`, `app.image` and the manifest, `app.native.json`. With `--json`, the result has a `native` object with the same information.

## Before the first AOT build: sk check

Run `sk check` on the app before you turn AOT on. An AOT build runs the same check and stops on its errors. Its codes come in four ranges:

| Codes  | Severity | Meaning                                                                                                                      |
| ------ | -------- | ---------------------------------------------------------------------------------------------------------------------------- |
| SK1xxx | error    | Breaks under AOT. The AOT build stops.                                                                                       |
| SK2xxx | warning  | Works under AOT, but runs slower.                                                                                            |
| SK3xxx | warning  | Typing quality: values the code passes around without a type.                                                                |
| SK4xxx | warning  | CSS the renderer does not apply, in the app's stylesheets, `<style>` blocks and `style` attributes; one summary per package. |

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

When the finding is in a dependency, look for a version or an option that does not patch built-ins. If the code that patches is never reached in your app, `sk.config.json` can silence the code for that package. The build then goes on, and the code still throws if it ever runs:

```json title="sk.config.json"
{
  "check": {
    "allow": { "SK1001": ["some-package"] }
  }
}
```

`"allow"` maps a code to package names or app-relative globs such as `src/legacy/**`.

### Code made at run time

An image has no JIT, so code created while the app runs is interpreted:

- **SK2001:** `eval`, direct or indirect.
- **SK2002:** `new Function(…)`. Libraries that generate their hot code this way (validators, template engines, serialisers) lose most of their speed. Many have a mode that does not generate code, often called CSP-safe or jitless.
- **SK2004:** `import()` with a computed specifier. A path the build did not compile fails to load at run time. Use string literals, for example a map of `() => import("./pages/home.ts")` loaders.

SK2003 reports a string passed to `setTimeout` or `setInterval`. Skathi's timers never run a string, with or without AOT, so that code is broken on every host.

The SK3 warnings measure how much of the code is typed: dependencies without declarations, `any` in what the app imports, `any` and `@ts-ignore` in the app's own code. They do not stop a build. `sk build --aot` compiles the minified JavaScript bundle.

## What still runs without the image

The bundle runs from the image, and nothing falls back to the JIT. Once a process has loaded an image, it has no JIT:

- **Run-time code.** `eval`, `new Function` and expressions evaluated from devtools run in the interpreter.
- **Reload.** The image's entry runs once per process. `location.reload()` evaluates `app.prod.js` from the payload instead.
- **A host that cannot load the image.** There is no fallback. When the manifest does not match the host, or the cache cannot be written, the app shows the reason in a message box and exits with code 3.

To run the same exe without its image, set `SK_APP_ENGINE=jit`. The host then evaluates `app.prod.js` under the JIT and logs that it did. This is how you tell whether a bug or a slowdown comes from AOT.

## Performance

AOT is not a guaranteed speedup. Measure your app both ways before you ship it.

The Skathi repository has a benchmark in `benches/aot`: a Vue table that mounts, updates, swaps and clears rows. With the toolchain current Skathi pins (Bun `237bba51e`, JavaScriptCore `autobuild-3bc94211`), the AOT build was slower than the JIT in it. Each run starts the app and repeats the same round of work; the table gives the round's total time, median of 3 runs on one Windows x64 machine:

| Round                            | JIT    | AOT    |
| -------------------------------- | ------ | ------ |
| First round after the app starts | 437 ms | 584 ms |
| Last round                       | 447 ms | 550 ms |

What else changes:

- **Exe size.** The image is added to the payload, deflate-compressed, and `app.prod.js` stays. The uncompressed images of Skathi's sample apps are 2.7 to 3.5 MB.
- **Disk.** The first start expands the image into `%LOCALAPPDATA%\skathi\native-aot\v1\`, one folder per image. `sk prune` removes the expanded images; apps expand them again at their next start.
- **Build time.** The sample apps compile in 1.3 to 2.1 seconds, with `sk-aot` peaking at 104 to 121 MiB of memory, plus the time of `sk check`.
- **Bytecode cache.** An AOT app does not read or write the per-user bytecode cache that a JIT app uses.

To compare your own app, build it once with `--aot` and start the same exe with and without `SK_APP_ENGINE=jit`.

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

## Troubleshooting

**`AOT builds are Windows x64 only for now`.** The build targets macOS or Linux, or `sk` runs on another OS or architecture. Build with `--no-aot`.

**`sk check found what a native image cannot run, so the AOT build stops here`.** The report follows. Fix each error, using `sk explain <code>`, or build with `--no-aot`.

**`… cannot load native images (it was built without the native AOT loader)`.** The workload has no AOT loader. Use a release workload of the same version (`sk workload install`), or build yours with the `native-aot` feature.

**`… expects native images made with the … transform and this sk makes …`** or **`… compiles for another workload`.** `sk`, `sk-aot` and the workload come from different Skathi versions. Use the `sk-aot` of the release that matches the workload: remove `--aot-compiler` and `SK_AOT_COMPILER` and let `sk` fetch it.

**`no sk-aot matches the workload` with `SK_HOST_OFFLINE=1`.** Allow the download once, or pass `--aot-compiler <sk-aot.exe>`.

**`--aot compiles against the workload it stamps, so it cannot be combined with --payload-only`.** An image is tied to one host, so an AOT build must stamp the exe.

**At start: `this app's native image cannot run on this host`.** The image does not match the exe it runs in, or the image cache cannot be created. The message box gives the reason. Rebuild the app with the `sk` that matches the workload. If the reason names `LOCALAPPDATA` or the cache, set `SK_AOT_CACHE_DIR` to an absolute folder the user can write.

**A `TypeError` the JIT build does not throw.** The code replaces a built-in on a path `sk check` did not see. Run the app with `SK_APP_ENGINE=jit` to confirm, then find the assignment from the stack.

## When not to use it

- **Without a measured gain.** AOT is experimental, and in the Vue benchmark above it was slower than the JIT.
- **When the app or a dependency patches built-ins** and cannot stop.
- **When hot code is generated at run time**, such as a template engine or validator built on `new Function` with no mode that avoids it. That code is interpreted.
- **For builds you iterate on.** Each AOT build adds the check and the compile; use `sk dev` while you work.
