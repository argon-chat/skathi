---
title: Elements
label: Overview
description: The elements Skathi draws or runs natively, beyond plain layout boxes, and the ones that behave differently from a browser. One line each, with a page per element.
section: Elements
order: 1
---

Most of a Skathi page is boxes: a `<div>`, a `<span>`, a `<p>`, a `<button>` is laid out and painted by Skathi's own engine and behaves as CSS says. The elements in this section are where an app gets its richer content: animation, video, images, drawing, frames from native code, web pages, text input. Each is implemented natively, in Rust, rather than by a script you bundle, and that shapes what it can do:

- **Decoding happens off the UI thread.** Images, Lottie animations and stickers are read and decoded on threads of their own, so a page full of them does not stall scrolling or typing. Decodes are shared: the same avatar in a hundred rows, or the same sticker in fifty messages, is decoded once.
- **Animation frames go straight to the GPU.** A `<lottie>` or `<sk-sticker>` frame is rasterized on a worker and written into the element's GPU tile, and a busy 2D canvas is updated in place, so a new frame costs a repaint, not a rebuild of the page.
- **Media uses the operating system.** `<video>` plays through Media Foundation, AVFoundation or GStreamer, so the app ships no codecs, and on Windows decoded frames stay on the GPU. File and color pickers and the Save As dialog are the system's own.
- **Native frames are part of the page.** `<sk-surface>` shows what your own native code renders, a libmpv player or a game engine's viewport, as an ordinary box that CSS clips and rounds and your UI draws over. There is no second window glued on top.
- **They are still the elements you know.** `src`, `load`, `v-model`, `bind:value`, `object-fit` and `border-radius` work as on the web. Each page says plainly where Skathi differs from a browser.

Each page opens with what you would build with the element and short recipes in Vue and Svelte, and ends with the reference: attributes, events, how it is drawn and what differs from a browser.

## Skathi's own elements

| Element                                 | What it does                                                                                               |
| --------------------------------------- | ---------------------------------------------------------------------------------------------------------- |
| [`<sk-surface>`](/elements/sk-surface/) | Shows frames that native code renders (a video player, a game engine), composited like any other element.  |
| [`<lottie>`](/elements/lottie/)         | Plays a Lottie animation from JSON or `.tgs`, rasterized off the UI thread.                                |
| [`<sk-sticker>`](/elements/sk-sticker/) | Plays many small animated stickers at once from Lottie or WebM, sharing decodes and frames between copies. |
| [`<sk-editor>`](/elements/sk-editor/)   | Rich text editing: named marks, inline elements as atoms, line blocks, transactions, undo and IME.         |

## HTML elements with a native implementation

| Element                                              | What it does                                                                                               |
| ---------------------------------------------------- | ---------------------------------------------------------------------------------------------------------- |
| [`<video>`](/elements/video/)                        | Plays video through the OS player: Media Foundation, AVFoundation or GStreamer. `<audio>` plays nothing.   |
| [`<img>`](/elements/img/)                            | Decodes images off the UI thread, shares decodes between elements and keeps small copies of shrunk images. |
| [`<svg>`](/elements/svg/)                            | Inline SVG is one leaf: its subtree is serialized and drawn by resvg, without per-shape hit testing.       |
| [`<canvas>`](/elements/canvas/)                      | A 2D context rasterized on the CPU, or a WebGPU context on D3D12, Metal or Vulkan.                         |
| [`<iframe>`](/elements/iframe/)                      | A web document in the system webview, composited under the app.                                            |
| [`<input>` and `<textarea>`](/elements/text-fields/) | Native text editing with IME, undo, the clipboard and the caret API; no constraint validation.             |
| [Form controls](/elements/form-controls/)            | Checkboxes, radios, range, color, `<select>` and file inputs, each drawn or picked natively.               |
| [`<a>`](/elements/links/)                            | `<a href>` opens web and mail links in the system, and `<a download>` saves through a Save As dialog.      |

## Components

| Component                          | What it does                                            |
| ---------------------------------- | ------------------------------------------------------- |
| [`SkWindow`](/elements/sk-window/) | Paints part of the component tree in its own OS window. |

`SkWindow` is a component, not a tag: you import it from `@skathi-native/runtime` (or `@skathi-native/runtime/svelte`). There is no new element behind it. It renders an ordinary `div`, and its props and events open, configure and close the window that paints that `div` and its subtree.

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
