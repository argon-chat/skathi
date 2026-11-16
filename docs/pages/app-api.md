---
title: App APIs
description: The desktop APIs a browser does not have — tray, global hotkeys, storage folder, shell, permissions, screen capture, locale, launch, embedding and devtools — with platform support and examples.
section: Reference
order: 5
---

# App APIs

The objects an app imports from `@skathi-native/runtime` for what the web platform cannot do: a tray
icon, system-wide hotkeys, where its data lives, opening files and links, permissions, screen capture
sources, its language, how it was launched, the program that embeds it, and its devtools. The main
window is in the [Window API](/window-api/); web standards are in [Web platform coverage](/web-platform/).

Every entry has its own page: the signature, the support on Windows, macOS and Linux, the release it
arrived in, examples, and what throws or prints an advisory.

## Import

A Vue app imports them from `@skathi-native/runtime`; a Svelte app from `@skathi-native/runtime/svelte`,
which re-exports the same objects:

```ts
import { appLaunch, appTray } from "@skathi-native/runtime"; // Vue
import { appLaunch, appTray } from "@skathi-native/runtime/svelte"; // Svelte
```

Unlike `appWindow`, none of them is reactive in Vue or Svelte. Keep what a template shows in a `ref`
or `$state`, and update it from the API's event or after the call. Register listeners in `onMounted` and
remove them in `onUnmounted` in Vue; in Svelte, return the unsubscribe function from `$effect`.

## Errors and advisories

- A wrong argument throws a `TypeError` that says what is wrong. A method that returns a promise rejects
  with it instead of throwing.
- A feature the platform lacks throws or rejects with an `Error` named `NotSupportedError`, or reads as
  `available: false` or `"unsupported"`, as each entry says.
- A host built without the native part of an object (an embedder can leave features out) still has the
  object, so importing it never fails: it reads as empty or `unsupported`, and a call that cannot work
  throws or rejects with the reason.

## Platforms at a glance

| Object                                        | Windows       | macOS                                | Linux                          |
| --------------------------------------------- | ------------- | ------------------------------------ | ------------------------------ |
| [`appTray`](/app-api/app-tray/)               | Yes           | Yes                                  | No                             |
| [`appHotkeys`](/app-api/app-hotkeys/)         | Yes           | Yes, with Accessibility access       | No                             |
| [`appStorage`](/app-api/app-storage/)         | Yes           | Yes                                  | Yes                            |
| [`appShell`](/app-api/app-shell/)             | Yes           | Not yet                              | Yes                            |
| [`appPermissions`](/app-api/app-permissions/) | Yes           | Microphone, camera, screen recording | Microphone                     |
| [`appCapture`](/app-api/app-capture/)         | Yes           | Yes                                  | No                             |
| [`appLocale`](/app-api/app-locale/)           | Yes           | Yes                                  | Yes                            |
| [`appLaunch`](/app-api/app-launch/)           | Yes           | Yes, no custom URL schemes yet       | Yes, no custom URL schemes yet |
| [`appHost`](/app-api/app-host/)               | When embedded | When embedded                        | When embedded                  |
| [`appDevtools`](/app-api/app-devtools/)       | Yes           | Yes                                  | Yes                            |
