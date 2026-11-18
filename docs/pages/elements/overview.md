---
title: Elements
label: Overview
description: The elements Skathi draws or runs natively, beyond plain layout boxes, and the ones that behave differently from a browser. One line each, with a page per element.
section: Elements
order: 1
---

Most of a Skathi page is boxes: a `<div>`, a `<span>`, a `<p>`, a `<button>` is laid out and painted by Skathi's own engine and behaves as CSS says. The elements on these pages are different. Each is implemented natively, in Rust, or adds behavior a web developer would not expect from the browser element of the same name. Their pages say what each one is for, what it reads and fires, how it is drawn, and where it differs from a browser.

## Skathi's own elements

| Element                                 | What it does                                                                                               |
| --------------------------------------- | ---------------------------------------------------------------------------------------------------------- |
| [`<sk-surface>`](/elements/sk-surface/) | Shows frames that native code renders (a video player, a game engine), composited like any other element.  |
| [`<lottie>`](/elements/lottie/)         | Plays a Lottie animation from JSON or `.tgs`, rasterized off the UI thread.                                |
| [`<sk-sticker>`](/elements/sk-sticker/) | Plays many small animated stickers at once from Lottie or WebM, sharing decodes and frames between copies. |
| [`<SkWindow>`](/elements/sk-window/)    | A component that paints part of the component tree in its own OS window.                                   |

## HTML elements with a native implementation

| Element                                              | What it does                                                                                               |
| ---------------------------------------------------- | ---------------------------------------------------------------------------------------------------------- |
| [`<video>`](/elements/video/)                        | Plays video through the OS player: Media Foundation, AVFoundation or GStreamer. `<audio>` plays nothing.   |
| [`<img>`](/elements/img/)                            | Decodes images off the UI thread, shares decodes between elements and keeps small copies of shrunk images. |
| [`<svg>`](/elements/svg/)                            | Inline SVG is one leaf: its subtree is serialized and drawn by resvg, without per-shape hit testing.       |
| [`<canvas>`](/elements/canvas/)                      | A 2D context rasterized on the CPU, or a WebGPU context on D3D12, Metal or Vulkan.                         |
| [`<iframe>`](/elements/iframe/)                      | A web document in WebView2, composited under the app. Windows only.                                        |
| [`<input>` and `<textarea>`](/elements/text-fields/) | Native text editing with IME, undo and the clipboard; no selection API, no constraint validation.          |
| [Form controls](/elements/form-controls/)            | Checkboxes, radios, range, color, `<select>` and file inputs, each drawn or picked natively.               |
| [Links](/elements/links/)                            | `<a href>` opens web and mail links in the system, and `<a download>` saves through a Save As dialog.      |

## What is not here

Some things that look like elements are CSS, and have their pages there:

- **Scrollbars** are drawn by Skathi on every `overflow: auto` or `scroll` box, as an overlay that takes no layout space. [`scrollbar-width`](/css/scrollbar-width/) and [`scrollbar-color`](/css/scrollbar-color/) style them; `::-webkit-scrollbar` is dropped with an advisory.
- **Title-bar drag regions** are the [`app-region`](/css/app-region/) property. [Custom title bar](/guides/custom-title-bar/) covers them.
- **Shader effects** are the [`--shading`](/css/shading/) custom property, not an element.

Some HTML elements have no behavior of their own yet. They lay out as ordinary boxes:

- `<dialog>`: `showModal()`, `show()` and `close()` do not exist, and a `<dialog>` without `open` is still shown. `::backdrop` is dropped.
- `<details>` and `<summary>`: the summary is focusable, but a click does not toggle `open`, and the content of a closed `<details>` is not hidden.
- `<progress>` and `<meter>` draw nothing.
- `<audio>` plays nothing; see [`<video>`](/elements/video/#audio).

## Elements and Cargo features

An embedder can build a host without some elements. `<lottie>` and `<sk-sticker>` need the `lottie` feature, `<video>` the `video` feature and `<iframe>` the `webview` feature, all on by default. Without its feature, an element keeps its CSS box, draws nothing and prints one console warning that names the feature. [Features and size](/embedding/features/) lists them.
