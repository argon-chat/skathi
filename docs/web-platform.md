# Web platform coverage

What a Skathi app can use from the web platform, what is coming, and what is not. CSS has its own
page: [CSS coverage](css.md).

| Mark | Meaning                                                     |
| ---- | ----------------------------------------------------------- |
| ✅   | Supported on Windows, macOS and Linux                       |
| 🪟   | Supported on Windows; macOS and Linux are planned           |
| 🍎   | Supported on Windows and macOS; Linux is planned            |
| 🟡   | Supported with a limitation (see the note)                  |
| 🗓️   | Planned                                                     |
| ⏸️   | On hold: waiting on something outside Skathi                |
| ⛔   | Not planned for now                                         |
| 🗑️   | Deprecated in Skathi: will not be added, or will be removed |

## Language and scheduling

| API                                                   | Status | Notes                                                |
| ----------------------------------------------------- | ------ | ---------------------------------------------------- |
| `setTimeout`, `setInterval`, `queueMicrotask`         | ✅     |                                                      |
| `requestAnimationFrame`, `requestIdleCallback`        | ✅     | Driven by the window's frame loop                    |
| `structuredClone`                                     | ✅     | Including `FileSystemHandle`                         |
| `reportError`, `ErrorEvent`, `onerror`                | ✅     | Uncanceled errors reach the host's error report      |
| `performance.now()`, `performance.timeOrigin`         | ✅     |                                                      |
| `performance.mark` / `measure`, `PerformanceObserver` | ⛔     |                                                      |
| Web Workers, `SharedWorker`, `MessageChannel`         | ⏸️     | Waiting for multithreading in Bun and JavaScriptCore |
| `BroadcastChannel`                                    | ✅     | Within the app                                       |

## DOM and events

| API                                                                                                   | Status | Notes                                                                      |
| ----------------------------------------------------------------------------------------------------- | ------ | -------------------------------------------------------------------------- |
| Elements, attributes, `getComputedStyle`, `elementFromPoint`, `scrollIntoView`                        | ✅     | The DOM is Skathi's element tree, rendered natively                        |
| `Event`, `CustomEvent`, `EventTarget`, UI, mouse, pointer, keyboard, wheel, focus, input, drag events | ✅     |                                                                            |
| `TouchEvent`                                                                                          | 🟡     | Constructible; the host does not deliver touch input yet                   |
| `ResizeObserver`, `IntersectionObserver`                                                              | ✅     | Run after layout, before paint, as in a browser                            |
| `MutationObserver`                                                                                    | ⛔     |                                                                            |
| `Selection`, `Range`                                                                                  | ✅     |                                                                            |
| `matchMedia`                                                                                          | ✅     |                                                                            |
| Fullscreen API                                                                                        | ✅     |                                                                            |
| Web Animations (`Animation`, `KeyframeEffect`, `element.animate`)                                     | ✅     |                                                                            |
| `FontFace`, `document.fonts`                                                                          | ✅     |                                                                            |
| `XMLSerializer`                                                                                       | ✅     |                                                                            |
| `DOMParser`                                                                                           | ⛔     |                                                                            |
| Custom elements, shadow DOM                                                                           | ⛔     | Use Vue components                                                         |
| `alert`, `confirm`, `prompt`                                                                          | ✅     | Native dialogs, in the user's language                                     |
| `window.open`                                                                                         | 🟡     | Opens external URLs in the system browser; a second window is `<SkWindow>` |

## Networking

| API                                       | Status | Notes                                                                                          |
| ----------------------------------------- | ------ | ---------------------------------------------------------------------------------------------- |
| `fetch`, `Request`, `Response`, `Headers` | ✅     | HTTP/1.1 and HTTP/2, HTTP cache, cookie jar. Response bodies are buffered                      |
| `XMLHttpRequest`                          | ✅     | With upload and download progress                                                              |
| `WebSocket`, `WebSocketStream`            | ✅     |                                                                                                |
| `EventSource`                             | ✅     | Reconnects with `Last-Event-ID` and the server's `retry`; the stream is requested uncompressed |
| `URL`, `URLSearchParams`                  | ✅     |                                                                                                |
| `URLPattern`                              | 🟡     | Custom groups use Rust `regex` syntax: lookaround and backreferences are a `TypeError`         |
| `Accept-Language`                         | ✅     | From `navigator.languages`, weighted as Chrome does                                            |
| `navigator.sendBeacon`                    | ⛔     |                                                                                                |
| Service workers, Cache API                | ⛔     |                                                                                                |

## Streams, binary data and encoding

| API                                                                       | Status | Notes                                                 |
| ------------------------------------------------------------------------- | ------ | ----------------------------------------------------- |
| `ReadableStream`, `WritableStream`, `TransformStream`, queuing strategies | ✅     | Including `ReadableStream.from`                       |
| `CompressionStream`, `DecompressionStream`                                | ✅     |                                                       |
| `TextEncoder`, `TextDecoder`, `atob`, `btoa`                              | ✅     |                                                       |
| `Blob`, `File`, `FileList`, `FileReader`, `FormData`                      | ✅     |                                                       |
| `AbortController`, `AbortSignal`                                          | ✅     | Including `AbortSignal.timeout` and `AbortSignal.any` |
| `crypto.getRandomValues`, `crypto.randomUUID`, `SubtleCrypto`             | ✅     |                                                       |

## Storage and files

| API                                                                                                               | Status | Notes                                                              |
| ----------------------------------------------------------------------------------------------------------------- | ------ | ------------------------------------------------------------------ |
| `localStorage`, `sessionStorage`                                                                                  | ✅     | `localStorage` can be encrypted (`appStorage`)                     |
| IndexedDB                                                                                                         | ✅     |                                                                    |
| `navigator.storage` (`StorageManager`)                                                                            | ✅     |                                                                    |
| File System Access (`showOpenFilePicker`, `showSaveFilePicker`, `showDirectoryPicker`, handles, writable streams) | ✅     | Restored handles are granted on a user activation without a prompt |
| `<input type=file>`, `<a download>`                                                                               | ✅     | Native pickers                                                     |
| `navigator.locks`                                                                                                 | ⛔     |                                                                    |

## Graphics and media

| API                                                      | Status | Notes                                                                                |
| -------------------------------------------------------- | ------ | ------------------------------------------------------------------------------------ |
| Canvas 2D, `OffscreenCanvas`, `Path2D`                   | ✅     |                                                                                      |
| `ImageData`, `ImageBitmap`, `createImageBitmap`          | ✅     |                                                                                      |
| `DOMMatrix`, `DOMPoint`                                  | ✅     |                                                                                      |
| WebGPU                                                   | ✅     |                                                                                      |
| WebGL, WebGL 2                                           | 🗑️     | `getContext('webgl')` returns `null`; use WebGPU                                     |
| Web Audio (`AudioContext` and its nodes, audio worklets) | ✅     |                                                                                      |
| `getUserMedia`, `MediaDevices`, `MediaStream`            | 🟡     | Microphone, behind the app's permission prompt; no camera yet                        |
| `<video>`                                                | 🟡     | Plays; `drawImage(video)` on a canvas draws nothing yet                              |
| `navigator.mediaSession`, `MediaMetadata`                | 🍎     | System media controls and media keys (Windows: the media flyout; macOS: Now Playing) |
| `MediaRecorder`, WebCodecs, WebRTC                       | ⛔     |                                                                                      |

## System integration

| API                                                                | Status | Notes                                                                                                 |
| ------------------------------------------------------------------ | ------ | ----------------------------------------------------------------------------------------------------- |
| `navigator.language`, `navigator.languages`                        | ✅     | The system's language preferences; an app can override them                                           |
| `navigator.userAgentData`                                          | ✅     | Including `getHighEntropyValues`                                                                      |
| `navigator.permissions.query`                                      | ✅     |                                                                                                       |
| Async Clipboard (`navigator.clipboard`, `ClipboardItem`)           | ✅     |                                                                                                       |
| `navigator.getBattery`                                             | ✅     |                                                                                                       |
| `navigator.userActivation`                                         | ✅     |                                                                                                       |
| `navigator.windowControlsOverlay`                                  | ✅     |                                                                                                       |
| `launchQueue` (`LaunchParams`)                                     | ✅     | Files and URLs the app was opened with                                                                |
| `navigator.setAppBadge`, `clearAppBadge`                           | 🍎     | Windows: a badge on the taskbar button; macOS: the Dock icon's badge                                  |
| `navigator.wakeLock` (`screen`)                                    | 🍎     | Released when the window is minimized or closed                                                       |
| `navigator.share`, `navigator.canShare`                            | 🍎     | The system share sheet; text, URLs and files                                                          |
| Passkeys: `navigator.credentials`, `PublicKeyCredential`           | 🍎     | Windows Hello, iCloud Keychain, security keys, phones. See below. No conditional mediation (autofill) |
| `Notification`                                                     | 🗑️     |                                                                                                       |
| Gamepad (`navigator.getGamepads`)                                  | ⛔     |                                                                                                       |
| Geolocation                                                        | ⛔     |                                                                                                       |
| Vibration, Pointer Lock, Web Bluetooth, WebUSB, WebHID, Web Serial | ⛔     |                                                                                                       |

### Passkeys in an app

A web page's relying party ID defaults to its own domain. An app has no web origin, so it names the
relying party itself: `rp.id` in `create()`, `rpId` in `get()`. The client data's `origin` is then
`https://<rpId>`, which is what the relying party's server should expect. A page served over https
from the relying party's domain keeps its own origin and needs neither.

On macOS the system serves a relying party only to an app associated with its domain: the app must be
signed with a provisioning profile that grants the Associated Domains entitlement
`webcredentials:<rpId>`, and `https://<rpId>/.well-known/apple-app-site-association` must list the app
under `webcredentials`. Without that, `create()` and `get()` reject with `NotAllowedError` and
`isUserVerifyingPlatformAuthenticatorAvailable()` resolves `false`. The client data is the system's
own, with the origin `https://<rpId>`.
