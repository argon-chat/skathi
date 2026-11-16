---
title: The runtime model
label: Core concepts
description: How Vue components, the native element tree, JavaScriptCore, and Rust platform services fit together.
section: Getting started
order: 2
---

Skathi keeps Vue’s component model and changes what renders it. Understanding that boundary helps you choose libraries and reason about platform behavior.

## Vue and the element tree

The Vue renderer translates component updates into operations on a native element tree. The native host lays out that tree and paints the interface through its GPU renderer.

Application code uses Vue reactivity and component composition. The runtime provides the objects and renderer glue needed to connect those components to the host.

```ts
import { createApp } from "@skathi-native/runtime";
import { createPinia } from "pinia";
import App from "./App.vue";

createApp(App).use(createPinia()).mount();
```

## JavaScriptCore runs the script

The application’s JavaScript executes in JavaScriptCore through the native runtime. Browser globals and APIs are available only where the SDK implements them.

A package that works with Vue is not automatically compatible if it assumes a complete browser DOM. Check what it accesses and run `sk check` when evaluating it.

## Rust owns platform services

Windows, storage, clipboard access, permissions, and other platform services are implemented in Rust. The JavaScript surface exposes those native capabilities to application code.

For embedding the SDK into another native application, the host provides `sk_app::embed` and `HostContext` hooks. Keep application-specific behavior in the embedder.

## CSS has a native implementation

Stylesheets are compiled with Lightning CSS. Layout uses Taffy, and drawing is native. There is no browser layout engine underneath.

Some CSS properties are supported, some have limitations, and others are planned. The [CSS coverage table](/css/) is the source to consult before relying on a property.

## Develop with the host

The `sk` CLI compiles components, starts the native development host, and handles the development workflow. Release packaging uses the same SDK boundary.

The [quick start](/quick-start/) walks through the commands. The [web platform coverage table](/web-platform/) lists the APIs you can use inside the application.
