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

`sk publish` without `"publishDevtools": true` uses a host built without devtools. The inspector, F12 and the local port are not in the executable, and nothing can turn them on: `SK_DEVTOOLS=1` and `appDevtools.enable()` have no effect.

`appDevtools` still exists in that build, so code that calls it does not need a check. `appDevtools.available` is `false`, `isEnabled()` returns `false`, and `open()` returns `false` and prints an advisory.

An app [embedded in your own executable](/embedding/dev-loop/#devtools) leaves the inspector out with the `no-devtools` feature of the `skathi` crate.

## Why to keep it off for users

The inspector's connection can evaluate JavaScript in your app. It listens on `127.0.0.1` only and requires a token the app writes into a handshake file, but a published build with devtools still lets a program on the same machine that can read that file drive the app. Keep `publishDevtools` off for apps you ship to users.

To inspect a release-like build yourself, use `sk build` (a debug build of the same app, with devtools) or a separate `sk publish` with `publishDevtools` turned on that you do not distribute.
