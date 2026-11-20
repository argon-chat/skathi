---
title: Turning devtools off and release builds
label: Release builds
description: Turn the devtools listener off in a build that has it, at startup or at runtime, and what a published build without devtools leaves out.
section: Devtools
order: 7
---

A build that carries devtools listens on a local port for the inspector and for [scripts](/devtools/scripting/). That is what you want while you work on the app, and not what you want in the copy your users run.

## Turn devtools off

Three settings control whether a build that carries devtools listens for the inspector:

- `"devtools": false` in `sk.config.json`, at the top level or in the `window` block, starts the app with devtools off: no F12 and no local port. See [`window.devtools`](/window-api/config-window-devtools/).
- `appDevtools.disable()` and `appDevtools.enable()` switch them at runtime. `appDevtools.onChange()` reports the change, and `appDevtools.isEnabled()` reads it.
- The environment variable `SK_DEVTOOLS=1` turns them on and refuses `disable()`, which is useful to inspect an app that turns them off. `SK_DEVTOOLS=0` starts with them off.

`SK_DEVTOOLS_KEY=off` disables only the F12 key; `appDevtools.open()` and the local port still work.

F12 in an app with devtools off prints why nothing opened, once, to the terminal and the console.

## Release builds

Two switches decide whether a shipped app has devtools: `"publishDevtools"` in `sk.config.json`, which `sk publish` writes into the app's payload, and the host the payload runs on, which either has the devtools compiled in or does not.

`sk publish` without `"publishDevtools": true` marks the payload as published without devtools and stamps it into a host built without them. The inspector, F12 and the local port are not in the executable, and nothing can turn them on: `SK_DEVTOOLS=1` and `appDevtools.enable()` have no effect.

`appDevtools` still exists in that build, so code that calls it does not need a check. `appDevtools.available` is `false`, `isEnabled()` returns `false`, and `open()` returns `false` and prints an advisory.

On Windows x64, `sk publish --aot` with `"publishJit": false` and `"publishDevtools": true` uses the `aot-nojit-devtools` workload (since 0.7). It enables devtools only when the payload authorises them and `inspector.image.dll` is present beside the exe. The interpreter runs console expressions. [Ship without the JIT](/guides/aot/#ship-without-the-jit).

### Embedded hosts (since 0.5)

An app [embedded in your own executable](/embedding/dev-loop/#devtools) runs on whatever host you build. Its devtools are compiled in unless you turn on the `no-devtools` feature of the `skathi` crate. Such a host still honours a payload published without devtools: they start off, as with `"devtools": false`, and no port is opened. The code is still in the executable, though, so `SK_DEVTOOLS=1` or a call to `appDevtools.enable()` turns them on. Build the release exe with `no-devtools` to leave them out entirely.

## Why to keep it off for users

The inspector's connection can evaluate JavaScript in your app. It listens on `127.0.0.1` only and requires a token the app writes into a handshake file, but a published build with devtools still lets a program on the same machine that can read that file drive the app. Keep `publishDevtools` off for apps you ship to users.

To inspect a release-like build yourself, use `sk build` (a debug build of the same app, with devtools) or a separate `sk publish` with `publishDevtools` turned on that you do not distribute.
