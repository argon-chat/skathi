---
title: Vue, naturally native.
label: Introduction
description: Build native desktop apps with Vue or Svelte and TypeScript, without Electron or a webview. A GPU renderer, JavaScriptCore, and the components you already know.
metaTitle: What is Skathi? Native desktop apps in Vue and Svelte
section: Getting started
order: 0
---

Skathi brings Vue to the desktop, with a native GPU renderer and JavaScriptCore running your code. Write Vue single-file components and TypeScript. Ship an application that runs without a browser engine.

Your components, reactivity, and familiar development workflow come with you. The pixels are native.

```bash
bun create skathi my-app
```

## A familiar place to start

A component is still a component. Declare your state, describe your interface, and let Vue connect the two.

```vue
<script setup lang="ts">
import { ref } from "vue";

const count = ref(0);
</script>

<template>
  <main class="window">
    <h1>A little more native.</h1>
    <button @click="count++">Clicked {{ count }} times</button>
  </main>
</template>

<style scoped>
.window {
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: center;
  gap: 16px;
  height: 100%;
}
</style>
```

The native entry point creates the application through the SDK runtime:

```ts
import { createApp } from "@skathi-native/runtime";
import App from "./App.vue";

createApp(App).mount();
```

## What happens under the hood

Vue’s renderer connects your components to a native element tree. Rust handles layout, drawing, windows, and platform services. JavaScriptCore runs the application’s JavaScript.

There is no WebView in this rendering path. CSS and web-compatible APIs are implemented by the SDK, which means their support has explicit boundaries. Check [CSS coverage](/css/) and [web platform coverage](/web-platform/) when bringing an existing component over.

## Bring what you know

- **Vue 3 single-file components.** Use the Composition API, reactive state, props, and events. Svelte 5 works too: pick the `svelte` template. [Vue support](/guides/vue-support/) and [Svelte support](/guides/svelte-support/) list the few features that do not carry over.
- **TypeScript.** Keep types alongside your application code.
- **Native rendering.** Layout and drawing belong to the host, rather than a browser.
- **Desktop platforms.** Windows is the primary development target; macOS and Linux hosts also exist. Individual capabilities vary by platform.
- **Bun tooling.** Create, develop, check, and package your application with the `sk` CLI.

## Pick your next step

Start with the [quick start](/quick-start/) to create and run an app. Read the [runtime model](/concepts/) to understand where Vue ends and the native host begins. Choosing between Skathi, Electron and Tauri? The [comparison](/comparison/) has measured size, memory and call cost side by side.

Already migrating an application? Keep the [CSS reference](/css/) and [platform API reference](/web-platform/) nearby. They describe supported features and the limitations that matter in practice.

## Read with your agent

Every page is available as Markdown through **Copy page** or the page menu. Point a coding agent at [/llms.txt](/llms.txt) for the documentation index, or [/llms-full.txt](/llms-full.txt) for all pages in one text file.

The HTML is rendered at build time, so readers and crawlers can access the content without running JavaScript.
