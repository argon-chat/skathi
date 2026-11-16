---
title: Your first native app
label: Quick start
description: Create a Vue application with Bun and run it in the native desktop host.
section: Getting started
order: 1
---

Go from a fresh directory to a native window using Bun and the Skathi CLI. Start with the showcase to explore the SDK, or choose the minimal template for a smaller application.

## Before you begin

Install [Bun](https://bun.sh/) 1.4 or later and use a supported desktop platform: Windows x64, macOS on Apple silicon, or Linux x64.

## Create an application

```bash
bun create skathi my-app
```

It asks for a template, the platforms to package for, and the package manager. `showcase` shows every feature, `minimal` is a single component, and `svelte` starts a Svelte 5 app. To skip the questions, pass them as options:

```bash
bun create skathi my-app --template minimal --yes
```

Then install the dependencies:

```bash
cd my-app
bun install
```

The app depends on `@skathi-native/cli`, so its `bun run` scripts and `bunx sk` use that version of `sk`.

## Open a native window

Start the development host:

```bash
bun run dev
```

Edit `src/App.vue` and save to see the component update. The showcase includes more SDK features; the minimal template starts with a compact Vue component.

## Check compatibility

Run the CLI checker when adding dependencies or porting browser-oriented code:

```bash
bun run check
```

Use the [CSS reference](/css/) and [web platform reference](/web-platform/) to check individual features. A familiar API name does not imply that every browser behavior is available.

## Build and package

Create a debug build with devtools:

```bash
bun run build
```

Produce the release build, one executable to ship:

```bash
bun run publish
```

Run `bunx sk build --help` or `bunx sk publish --help` for packaging options.

## Keep exploring

Read the [runtime model](/concepts/) before adapting an existing web application. It explains the renderer, host services, and why native CSS coverage differs from a browser.
