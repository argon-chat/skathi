---
title: "<iframe>"
label: "<iframe>"
metaTitle: "<iframe>: a web document in WebView2"
description: What <iframe> is in Skathi, a web document in the system's WebView2 composited under the app on Windows. What it can load, messaging with postMessage, origins, input, sizing, the limits, and macOS and Linux.
section: Elements
order: 10
---

An `<iframe>` shows a web document: an HTML page from the app's files, from `srcdoc` or from the web. Skathi has no browser engine of its own, so the document runs in the system's webview, WebView2, in a separate browser process, and is composited into the element's box under the app's own drawing. It is the one place in a Skathi app where HTML and its JavaScript run in a browser.

It works on **Windows only**. On macOS and Linux an `<iframe>` is an empty box that fires `error`; check `skFeatures.iframe` before relying on it.

The document is not a Skathi app or component. It cannot import your Vue or Svelte code or reach your state; the two sides talk through `postMessage`.

## A minimal example

```vue
<script setup lang="ts">
import { onMounted, onUnmounted, ref } from "vue";

const frame = ref<HTMLIFrameElement | null>(null);
const reply = ref("");
const webView = skFeatures.iframe;

function onMessage(e: MessageEvent) {
  if (e.source === frame.value?.contentWindow) reply.value = String(e.data);
}
onMounted(() => window.addEventListener("message", onMessage));
onUnmounted(() => window.removeEventListener("message", onMessage));

function ping() {
  frame.value?.contentWindow?.postMessage({ type: "ping" });
}
</script>

<template>
  <iframe v-if="webView" ref="frame" class="doc" src="help/index.html" />
  <p v-else>Help needs Windows.</p>
  <button @click="ping">Ping</button>
  <p>{{ reply }}</p>
</template>

<style scoped>
.doc {
  width: 100%;
  height: 480px;
  border: 0;
}
</style>
```

Inside `help/index.html`, the document answers with `window.parent.postMessage("pong", "*")`.

## What it can load

| `src`                                             | Loads                                                                                                                    |
| ------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------ |
| A relative or root-relative path, `app://redjs/…` | The app's own file, served by Skathi from the package, or in development from the app's root or its `public/` directory. |
| `https:`, `data:`, `//host/…`                     | Loaded by WebView2 from the network, with its own network stack, cache and cookies.                                      |
| `srcdoc`                                          | That HTML. It wins over `src`.                                                                                           |

Inside the webview, the app's files are served from the origin `https://app.vn.invalid`, a name that never resolves, so a request Skathi does not answer fails instead of going to the network. A document from the app's files sees that as its `location.origin` and as the origin of messages from the app. The app sees its own origin in messages from such a document.

## Attributes

`src`, `srcdoc`, `sandbox`, `allow`, `referrerpolicy`, `name` and `allowfullscreen` are passed to the document as a browser would apply them. `sandbox` is the browser's real sandbox, and without it the document is not sandboxed. Changing any of them navigates again: the document is rebuilt from scratch, and messages queued for the old one are dropped. `width` and `height` are size hints in CSS pixels, unitless; CSS wins.

`loading` is reflected but ignored: the document is created as soon as the element is in the document, on screen or not. `allow` grants nothing in practice, because Skathi denies every permission request from the webview: no camera, microphone, location or notifications.

## Messaging

**From the app.** `iframe.contentWindow` is an object with only `postMessage` (and `toString`), the same object each time, and `null` while the element is not in the document. `contentDocument` is always `null`: the document is cross-origin.

`postMessage(message, targetOrigin)` sends to the document. The target origin defaults to `"/"`, the app's own origin, which reaches a document from the app's files. Pass the document's origin, or `"*"`, for one from the web, and `"*"` for a document sandboxed without `allow-same-origin`, whose origin is opaque. A message is encoded in the sender, so a value that cannot cross throws there: JSON values, `undefined`, `NaN`, the infinities, `-0`, BigInt, `Date`, `ArrayBuffer`, typed arrays, `DataView`, `Map`, `Set` and plain objects can; functions, symbols, elements, cyclic objects and transfer lists cannot. A message sent before the document is ready waits for it; one sent to an element without a document is dropped.

**From the document.** The document calls `window.parent.postMessage(…)`. The app gets a `message` event on `window`, not on the element, with `source === iframe.contentWindow`, `origin` and `data`. `ports` is always empty. Only the iframe's own document reaches the app; frames nested in it and popups it opens do not.

## Events

`load` and `error` fire once per navigation of the element: when it is created, and after each change of `src` or the other attributes above. Links followed inside the document fire nothing. Unlike a browser, which never fires `error` on an iframe, Skathi fires `error` when the page fails to load, when there is no webview, when the browser process exits, or when too many documents are open. The reason goes to the console.

## Input and focus

An `<iframe>` is focusable by default. While it has focus, the keyboard goes to the document, except F5, F11, F12, combinations with Alt, and Escape while the app is full screen, which stay with the app.

Mouse buttons (left, middle, right) and the wheel go to the document where the app's hit test lands on the iframe's box, so an element positioned over the iframe gets the clicks there. Shift and Ctrl are passed with them; Alt and the Windows key are not. One wheel notch scrolls 100 pixels. Touch and pen input is not forwarded. The cursor follows the document's.

## Size and CSS

The box is 300 × 150 unless CSS sizes it, as in browsers. The webview is placed at the box, clipped to it and to clipping ancestors, and scaled with the window. `border-radius` rounds the visible corners.

The document is drawn under the app: Skathi cuts the iframe's box out of its own drawing, so the document shows through. The element's own `background` therefore does not show under the document, and an element positioned over the iframe draws over the document. `opacity` on the iframe is not applied to the document, and the effect of `transform` on it is not defined.

When the iframe is not painted (`display: none`, scrolled out of view), its document is hidden but keeps running.

## Lifecycle and limits

- A document exists while the element is in the document and has a `src` or `srcdoc`. Moving the element keeps its document; removing it closes the document.
- At most 8 documents are open at once, across the app. The ninth `<iframe>` fires `error`.
- The webview's own top-level navigation, new windows and popups are blocked. `alert()` and `confirm()` show no dialog.
- Zooming, swipe navigation, autofill and password saving are off. The webview's devtools and context menu are on in development builds only.
- Documents close when their window closes and when the app quits.
- The webview's storage lives in the app's data directory, under `webview`.

## Platforms

| Platform | `<iframe>`                                                                                         |
| -------- | -------------------------------------------------------------------------------------------------- |
| Windows  | WebView2, when the WebView2 runtime is installed. It ships with Windows 11 and current Windows 10. |
| macOS    | An empty box that fires `error`.                                                                   |
| Linux    | An empty box that fires `error`.                                                                   |

`skFeatures.iframe` is `false` wherever there is no webview: on macOS and Linux, on Windows without the WebView2 runtime, and in a host built without the `webview` Cargo feature. Without the feature the element also prints a warning naming it; see [Features and size](/embedding/features/).

## See also

- [Features and size](/embedding/features/): the `webview` feature.
- [Links](/elements/links/): opening a URL in the system browser instead.
