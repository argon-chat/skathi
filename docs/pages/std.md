---
title: Skathi modules
label: skathi:* modules
description: The modules every Skathi host provides under the skathi: prefix — ending and restarting the app, its version, the Skathi it runs on, and the operating system.
section: Reference
order: 6
since: "0.7"
---

# Skathi modules

`skathi:*` is Skathi's own library: modules the host provides, imported by name the way Node has
`node:` modules. There is nothing to install. Every builder (`sk dev`, `sk build`, the Vite plugin)
resolves the prefix, and the types come with `@skathi-native/runtime`.

```ts
import { app } from "skathi:app";
import { os } from "skathi:os";
import { Runtime, runtime } from "skathi:runtime";
```

| Module                             | What it gives                                                             |
| ---------------------------------- | ------------------------------------------------------------------------- |
| [`skathi:app`](#skathiapp)         | `app.quit()`, `app.restart()`, `app.getVersion()`, and the `app*` objects |
| [`skathi:runtime`](#skathiruntime) | The Skathi version, the host's features, AOT and single-file              |
| [`skathi:os`](#skathios)           | Platform, architecture, OS release, host name, home and temp              |

A name Skathi has no module for is a build error that lists the ones there are. The modules are
part of the host, so they work the same under `sk dev` and in a built app. A module the embedding
program registers is a different thing: that is [`native:<name>`](/embedding/native-modules/).

## skathi:app

```ts
import { app } from "skathi:app";

interface App {
  quit(options?: { exitCode?: number }): void;
  restart(options?: { args?: string[] }): void;
  getVersion(): string;
}
```

The module also exports every [App API](/app-api/) object (`appTray`, `appShell`, `appWindow` and
the rest), the same objects `@skathi-native/runtime` exports.

### app.quit

Ends the app the way [`appWindow.close()`](/window-api/) does: `pagehide` fires once,
`close-requested` and `beforeunload` are not asked, every window hides at once and the teardown
runs behind them, under the same deadline. See [Closing, hiding and quitting](/guides/closing-and-background/).

`exitCode` is the process's exit code, an integer from 0 to 255; the default is 0. Anything else
throws a `TypeError` and the app keeps running. Calling `quit()` again while the app is closing
does nothing.

```ts
import { app } from "skathi:app";

if (!(await migrate())) app.quit({ exitCode: 3 });
```

### app.restart

Ends the app as `quit()` does and starts it again, with the arguments this process was started
with or with `args`. The new process is started before the teardown begins. A single-instance app
restarts correctly: the new process waits for this one to end.

```ts
app.restart();
app.restart({ args: ["--safe-mode"] });
```

`args` must be an array of strings, or the call throws a `TypeError`.

### app.getVersion

The app's version: `"version"` in `sk.config.json`, else the one in `package.json`, else
`"0.0.0"`. It is the string `sk build` stamps into the executable's version resource and the macOS
bundle. The version of Skathi itself is `runtime.version`.

### Platforms

| Method         | Windows      | macOS                        | Linux                                           | Embedded                                                              |
| -------------- | ------------ | ---------------------------- | ----------------------------------------------- | --------------------------------------------------------------------- |
| `quit()`       | Yes          | Yes                          | Yes                                             | Tells the embedder the app closed; `Exit::exit_code` carries the code |
| `restart()`    | The same exe | The executable in the bundle | The AppImage (`$APPIMAGE`), else the executable | Throws `NotSupportedError`: the process is the embedder's             |
| `getVersion()` | Yes          | Yes                          | Yes                                             | The version in the embedder's payload                                 |

## skathi:runtime

```ts
import { Runtime, runtime } from "skathi:runtime";

interface Runtime {
  readonly version: string;
  readonly features: ReadonlySet<string>;
}
declare const Runtime: {
  isAot(): boolean;
  isSingleFile(): boolean;
};
```

- **`runtime.version`** is the version of Skathi the host was built from, for example `"0.7.0"`.
- **`runtime.features`** names the optional parts this host has: `audio`, `spatial-audio`,
  `storages`, `clipboard`, `indexeddb`, `webgpu`, `lottie`, `video`, `hotkeys`, `webview`, `battery`,
  `devtools`, `native-aot`, `no-jit`, `no-wasm`. An embedder chooses them
  ([Features](/embedding/features/)); the hosts `sk build` uses have the default set. Use it for
  diagnostics and bug reports. To decide whether to call an API, test the API itself
  (`typeof localStorage`, `appHotkeys.available`), as on the web.
- **`Runtime.isAot()`** is `true` while the app's code runs from its [native image](/guides/aot/).
  It is `false` under `sk dev`, in a build without `--aot`, with `SK_APP_ENGINE=jit`, and after a
  reload, which runs the bundle.
- **`Runtime.isSingleFile()`** is `true` when the app needs no file beside its executable: the
  payload is inside it and there are no asset packs, no `icu.skpk` and no native image file to load.
  A macOS `.app` bundle counts as one file. It is `false` under `sk dev` and for a payload read
  from a file.

```ts
import { app } from "skathi:app";
import { os } from "skathi:os";
import { Runtime, runtime } from "skathi:runtime";

const report = {
  app: app.getVersion(),
  skathi: runtime.version,
  features: [...runtime.features],
  aot: Runtime.isAot(),
  os: `${os.platform} ${os.release} ${os.arch}`,
};
```

## skathi:os

```ts
import { os } from "skathi:os";

interface OS {
  readonly platform: "win32" | "darwin" | "linux";
  readonly arch: "x64" | "arm64";
  readonly machineArch: "x64" | "arm64";
  readonly release: string;
  readonly distro: { name: string; version: string; id: string } | null;
  readonly hostname: string;
  readonly homedir: string;
  readonly tmpdir: string;
}
```

The values are synchronous and use Node's names, so code and packages written against
`process.platform` and `os.arch()` read the same strings. `navigator.userAgentData` stays for code
written for a browser; its `getHighEntropyValues()` is asynchronous and reports Chromium's
`platformVersion`, not the OS release.

| Property      | Windows                             | macOS                         | Linux                                                                       |
| ------------- | ----------------------------------- | ----------------------------- | --------------------------------------------------------------------------- |
| `platform`    | `"win32"`                           | `"darwin"`                    | `"linux"`                                                                   |
| `arch`        | What the host was built for         | The same                      | The same                                                                    |
| `machineArch` | The CPU, also for an x64 app on Arm | The CPU, also under Rosetta   | The kernel's machine                                                        |
| `release`     | `"10.0.26100"`: major, minor, build | `"15.1"`: the product version | The kernel release, `"6.8.0-45-generic"`                                    |
| `distro`      | `null`                              | `null`                        | `NAME`, `VERSION_ID` and `ID` of `/etc/os-release`; `null` without the file |
| `hostname`    | The DNS host name                   | `gethostname`                 | `gethostname`                                                               |
| `homedir`     | The user's profile folder           | `$HOME`                       | `$HOME`                                                                     |
| `tmpdir`      | The user's temp folder              | `$TMPDIR`                     | `$TMPDIR`, else `/tmp`                                                      |

`arch` and `machineArch` differ when the app runs translated: an x64 build on Windows on Arm or
under Rosetta reads `arch: "x64"`, `machineArch: "arm64"`. `tmpdir` has no trailing separator. The
app's own folders are in [`appStorage`](/app-api/app-storage/).
