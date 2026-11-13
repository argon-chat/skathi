// The `__vn_*` host protocol. Ambient: engine globals, evaluated as classic scripts.
// Ids are dense u32 (`0` = none). Names cross once, interned via `__vn_intern` as `VnNameId`; a host fn taking a
// `VnNameId` misreads a raw string. Structured answers are strings: comma-joined numbers, `name=value` lines, or JSON.
// A host fn may be absent (storage, permissions, windowing, devtools come from the app host): probe with
// `typeof fn === "function"`.

type VnNodeId = number;
type VnNameId = number;
type VnIdList = string;
/** `"x,y,w,h"` as f32s, `""` when not painted, `"stale"` when the host could not force a layout. A CSS `transform` appends the drawn box `"tx,ty,tw,th"`. */
type VnRect = string;
/** `name=value` lines, `\n`-separated. `""` when there is nothing to report. */
type VnKvLines = string;
/** A JSON document. The host never returns a bare value — always an object. */
type VnJson = string;

/** Interns `name`. Idempotent: the same string always answers the same id. */
declare function __vn_intern(name: string): VnNameId;

declare function __vn_create_element(tag: VnNameId): VnNodeId;
declare function __vn_create_text(): VnNodeId;
declare function __vn_create_anchor(): VnNodeId;
/** Creates THE root node (`:root`, `html`, `body`). Called exactly once, by `createApp().mount()`. */
declare function __vn_create_root(): VnNodeId;

declare function __vn_set_text(id: VnNodeId, text: string): void;
/** Sets an ELEMENT's own text — exclusive with its children, as in the tree. */
declare function __vn_set_element_text(id: VnNodeId, text: string): void;

/** Inserts `child` into `parent` before `anchor` (`0` = append). Also a move: `child` is detached first. */
declare function __vn_insert(child: VnNodeId, parent: VnNodeId, anchor: VnNodeId): void;
/** Destroys `id` AND ITS WHOLE SUBTREE. The ids are recycled afterwards. */
declare function __vn_remove(id: VnNodeId): void;
/** Selects a connected fullscreen element in its owner window; zero restores the document. */
declare function __vn_fullscreen_element(id: VnNodeId): boolean;
/** Queries the owner OS window; an optional boolean queues its fullscreen state. `ownerRoot` is 0 for the primary window or a VnWindow root id. */
declare function __vn_fullscreen_window(ownerRoot: VnNodeId, on?: boolean): boolean;

/** One inline declaration: `""` REMOVES it; a value may end in `!important`; an updated key keeps its place. */
declare function __vn_set_style(id: VnNodeId, key: VnNameId, value: string): void;
/** Does the engine accept `key: value` as one declaration? Absent on an older host. */
declare function __vn_style_accepts(key: VnNameId, value: string): boolean;

/** `el.style` for a mirror element: a CSSStyleDeclaration over its inline declarations, reading `__vnId` on every access. */
declare function __vn_style_object(node: object): import("./runtime/types").MirrorStyle;
/** `getComputedStyle(node)`: read-only, one property per read; anything that is not a mirror node reads the root's. */
declare function __vn_computed_object(node: unknown): import("./runtime/types").MirrorStyle;
/** runtime-dom's `setStyle` for one `:style` entry: the key as written (`backgroundColor`), `""` removes, a trailing `!important` is the priority. */
declare function __vn_style_assign(id: VnNodeId, key: string, value: string): void;
/** One inline declaration's value by CSS name, `""` when there is none. */
declare function __vn_style_get(id: VnNodeId, name: string): string;
/** `style.cssText = text`, or `setAttribute("style", text)` (`asAttribute`, which keeps `text` verbatim). */
declare function __vn_style_replace(id: VnNodeId, text: string, asAttribute: boolean): void;
/** `removeAttribute("style")`. */
declare function __vn_style_clear(id: VnNodeId): void;
/** The `style` attribute: `null` when absent. */
declare function __vn_style_attr(id: VnNodeId): string | null;
/** Called after an inline change that adds or removes the attribute, or every change while observed (`__vn_style_observe`). */
declare function __vn_style_hook(hook: (id: VnNodeId, oldValue: string | null, present: boolean) => void): void;
declare function __vn_style_observe(on: boolean): void;

declare function __vn_set_class(id: VnNodeId, classes: string): void;
/** One attribute-channel prop. `undefined` (or `null`) REMOVES it; anything else must be a string. */
declare function __vn_set_prop(id: VnNodeId, key: VnNameId, value: string | undefined): void;
declare function __vn_add_scope(id: VnNodeId, scope: string): void;
/** Replaces the whole stylesheet; `extra` are the `document.head` `<style>` texts, cascaded after `css` and parsed separately. */
declare function __vn_set_stylesheet(css: string, extra?: string[]): void;

/** Arms or disarms one event bit on one node (`code` < 64). Names can share a code, so callers OR over all of them first. */
declare function __vn_set_listener(id: VnNodeId, code: VnEventCode, on: boolean): void;

/** Installs the one dispatcher the host calls for EVERY input event; returning `false` means a handler called `preventDefault()`. The host drains microtasks after the call. */
declare function __vn_set_dispatcher(fn: VnDispatcher): void;

/**
 * Installs the observer step `layoutStep(op) -> number`, called after layout and before paint. `op` 0 gathers at
 * depth 0 (answers how many observers have entries); 1 calls the next back (answers how many are left); 2 gathers
 * again at the last broadcast depth after a fresh layout; 3 raises the loop error if an observation was skipped,
 * updates intersections, and answers bit 0 = loop error raised, bit 1 = something still observed.
 */
declare function __vn_set_layout_hook(fn: (op: number) => number): void;

/** `true`: the next frame runs the layout step even if nothing lays out. `false`: every observation was released. */
declare function __vn_layout_watch(observing: boolean): void;

/** `(nodeId, code, a, b, c, d, key, data, x, y) -> proceed`. `a`-`d`/`key` are per-event (`VnEventPayload`); `data` is the clipboard DataTransfer for copy/cut/paste; `x`/`y` are window CSS px, only for `wheel`. */
type VnDispatcher = (
  node: VnNodeId,
  code: VnEventCode,
  a: number,
  b: number,
  c: number,
  d: number,
  key: string | undefined,
  data: unknown,
  x?: number,
  y?: number,
) => boolean;

/**
 * What `a`/`b`/`c`/`d`/`key` carry, per event family.
 *
 * | family                    | a            | b           | c           | d          | key             |
 * |---------------------------|--------------|-------------|-------------|------------|-----------------|
 * | mouse (click/down/up/…)   | clientX      | clientY     | button      | mods \| detail<<4 | —        |
 * | contextmenu               | clientX      | clientY     | 2 (mouse) / 0 (keys) | mods | —              |
 * | pointercancel             | clientX      | clientY     | -1          | —          | —               |
 * | `<img>` load              | naturalWidth | naturalHeight | —         | —          | the src         |
 * | `<img>` error             | —            | —           | —           | —          | the src         |
 * | wheel                     | deltaX       | deltaY      | —           | mods       | —               |
 * | keydown/keyup             | —            | —           | —           | mods       | `KeyboardEvent.key` |
 * | input                     | —            | —           | —           | —          | the new value   |
 * | scroll                    | scrollLeft   | scrollTop   | —           | —          | —               |
 * | focus / blur              | the move's other node id (`relatedTarget`; 0 none) | — | — | — | —   |
 * | media                     | currentTime  | duration    | readyState  | playbackRate | —             |
 * | media `error`             | MediaError.code | currentTime | —        | —          | advisory text   |
 * | lottie `enterFrame`       | currentTime  | totalTime   | elapsed     | progress   | —               |
 * | lottie `loopComplete`     | currentTime  | totalTime   | currentLoop | —          | —               |
 * | lottie `complete`         | currentTime  | totalTime   | loops       | —          | —               |
 * | transitionend             | elapsedTime  | —           | —           | —          | property name   |
 * | animation{start,end,cancel} | elapsedTime | —          | —           | —          | @keyframes name |
 * | copy / cut / paste        | —            | —           | —           | —          | —  (`data` is the DataTransfer) |
 *
 * `mods` is a bitfield: 1 ctrl, 2 shift, 4 alt, 8 meta. `mousedown` / `mouseup` /
 * `click` carry the click count (`MouseEvent.detail`) above it. A keyboard `click`
 * (detail 0) carries `"Enter"` or `" "` in `key`; a mouse click leaves it empty.
 */
interface VnEventPayload {
  a: number;
  b: number;
  c: number;
  d: number;
  key: string | undefined;
}

/** Every event code the host can deliver, in lockstep with the host's `EV_*` table (`EV_CODES` in `runtime/event-names.ts`; `test/protocol-codes.test.ts` checks it against the Rust). Not a `const enum`: bun downgrades it and `isolatedModules` forbids ambient ones. */
type VnEventCode =
  | 1 // click
  | 2 // mousedown
  | 3 // mouseup
  | 4 // wheel
  | 5 // keydown
  | 6 // keyup
  | 7 // focus
  | 8 // blur
  | 9 // input
  | 10 // scroll
  | 11 // complete            (lottie)
  | 12 // loopComplete        (lottie)
  | 13 // enterFrame          (lottie)
  | 14 // mousemove
  | 15 // mouseenter
  | 16 // mouseleave
  | 17 // loadstart           (media)
  | 18 // loadedmetadata
  | 19 // loadeddata
  | 20 // canplay
  | 21 // canplaythrough
  | 22 // play
  | 23 // playing
  | 24 // pause
  | 25 // seeking
  | 26 // seeked
  | 27 // timeupdate
  | 28 // durationchange
  | 29 // ratechange
  | 30 // volumechange
  | 31 // waiting
  | 32 // ended
  | 33 // error
  | 34 // emptied
  | 35 // transitionend
  | 36 // animationstart
  | 37 // animationend
  | 38 // animationcancel
  | 39 // paste
  | 40 // copy
  | 41 // cut
  | 42 // contextmenu         (right-button release, or Menu key / Shift+F10)
  | 43 // load                (<img>)
  | 44 // pointercancel      (the window lost activation mid-press)
  | 45 // selectstart        (cancelable)
  | 46 // selectionchange   (at the root, for `document`)
  | 47 // auxclick          (middle or right button, after its mouseup)
  | 48 // beforeinput       (cancelable, but not during IME composition)
  | 49 // compositionstart
  | 50 // compositionupdate
  | 51 // compositionend
  | 52; // message            (an <iframe>'s document to its parent)

/** The document selection: `"anchor,anchorOffset,anchorOwn,focus,focusOffset,focusOwn"` (UTF-16 offsets), `""` for none. `own` 1 is an offset into an element's OWN text, which the runtime turns into a Text child. */
declare function __vn_selection_get(): string;
/** Sets anchor and focus; anchor 0 empties it. Answers whether both named live, attached nodes. */
declare function __vn_selection_set(
  anchor: VnNodeId,
  anchorOffset: number,
  focus: VnNodeId,
  focusOffset: number,
): boolean;
declare function __vn_selection_text(): string;
/** `element`'s own text became the Text child `text`: a boundary in it moves with it. */
declare function __vn_selection_rebase(element: VnNodeId, text: VnNodeId): void;

/** `elementFromPoint` (`all` 0) / `elementsFromPoint` (`all` 1): comma-separated ids under viewport point `(x, y)`, topmost first; skips `pointer-events: none` and `visibility: hidden`. */
declare function __vn_hit_at(x: number, y: number, all: 0 | 1): string;
declare function __vn_pointer_capture(nodeId: number, pointerId: number, capture: boolean): boolean;

declare var __vn_waapi_core: import("./waapi").NativeWaapiCore | undefined;

/** `__vn_query`'s mode: 0 descendants, 1 self (`matches`), 2 `closest`, 3 document subtree (root included). */
type VnQueryMode = 0 | 1 | 2 | 3;

/** Selector matching through the cascade's own matcher: comma-joined ids, `""` for none, `"!<advisory>"` when the engine cannot run the selector. */
declare function __vn_query(selector: string, root: VnNodeId, mode: VnQueryMode): VnIdList;

/** The node's rectangle at the LAST PAINTED FRAME, in client-area space. Lays out first if the tree changed, except inside an input listener, where it answers `"stale"`. */
declare function __vn_bounds(id: VnNodeId): VnRect;

type VnScrollOp = 0 | 1 | 2;

/** `"left,top,scrollWidth,scrollHeight,stale"` in CSS px. `stale` 1: the content half (and the clamp) is a frame old because layout could not be forced. `""` for a node that is not a scroll container. */
type VnScrollPos = string;

/** Reads or moves a scroll container; answers the position after the clamp. `x`/`y` are CSS px; op 1 leaves a `NaN` axis where it is, op 2 makes it 0. The `scroll` event fires next frame. */
declare function __vn_scroll(id: VnNodeId, op: VnScrollOp, x: number, y: number): VnScrollPos;

type VnScrollAlign = 0 | 1 | 2 | 3;

/** `scrollIntoView({ block, inline })` through every scrollable ancestor: `""` ran, `"stale"` ran on a frame-old layout, `"!<advisory>"` could not run. */
declare function __vn_scroll_into_view(id: VnNodeId, block: VnScrollAlign, inline: VnScrollAlign): string;

/**
 * `el.focus()` (op 0) / `el.blur()` (op 1). Focus answers `"moved:<old id>"` (the runtime fires blur/focusout on the
 * old node, 0 = none, then focus/focusin), `"already"`, `"pending"` (queued until the node is built; the host fires
 * the events) or `"refused"`. Blur answers `"blurred"` or `""`.
 */
declare function __vn_focus(id: VnNodeId, op: 0 | 1): string;
declare function __vn_input_select(id: VnNodeId): boolean;

declare function __vn_reload(): void;

declare function __vn_media_state(id: VnNodeId): VnKvLines;

type VnMediaProp =
  | 0 // currentTime
  | 1 // duration
  | 2 // paused        (0 | 1)
  | 3 // ended         (0 | 1)
  | 4 // seeking       (0 | 1)
  | 5 // readyState
  | 6 // networkState
  | 7 // volume
  | 8 // muted         (0 | 1)
  | 9 // playbackRate
  | 10 // loop         (0 | 1)
  | 11 // videoWidth
  | 12; // videoHeight

/** ONE scalar media property (booleans 0/1), `undefined` for a node that is not a live media element. Cheaper than `__vn_media_state`. */
declare function __vn_media_prop(id: VnNodeId, prop: VnMediaProp): number | undefined;

type VnMediaVerb =
  | 0 // play
  | 1 // pause
  | 2 // currentTime =
  | 3 // volume =
  | 4 // muted =        (arg: 0 | 1)
  | 5 // playbackRate =
  | 6; // loop =        (arg: 0 | 1)

/** Runs one verb. Returns whether the node was a live media element at all. */
declare function __vn_media_call(id: VnNodeId, verb: VnMediaVerb, arg: number): boolean;

/** `canPlayType(type)`, probed on THIS machine. */
declare function __vn_media_can_play(id: VnNodeId, mime: string): "" | "maybe" | "probably";

/** `<iframe>` postMessage: `wire` is the JSON payload; `targetOrigin` is `"*"`, `"/"` (the app's own origin) or an origin. Dropped when there is no document or the origin does not match. */
declare function __vn_iframe_post(id: VnNodeId, wire: string, targetOrigin: string): void;
declare function __vn_iframe_supported(): boolean;
declare var vnFeatures: { readonly iframe: boolean };

/** Drains the node's recent media events, comma-joined (tests only). */
declare function __vn_media_events(id: VnNodeId): string;

// Multi-window: these move a subtree's PAINT into a second OS window while the node stays in this tree; `nodeId` is
// `mirrorNode.id`. `<VnWindow>` is the only intended caller. Probe with `typeof __vn_window_open === "function"`.

/**
 * Moves the node's PAINT into a new OS window. `options` is JSON: `<VnWindow>`'s props, verbatim. `onClose` fires
 * however the window closed; `onEvent` gets `("open" | "focus" | "blur" | "resize" | "show" | "hide", detailJson)`,
 * e.g. `("open", '{"windowId":N}')`, `("resize", '{"width":W,"height":H,"scale":S}')`.
 * Returns `undefined`: the open is DEFERRED by one effect cycle (it cannot nest in the primary window's update).
 */
declare function __vn_window_open(
  nodeId: VnNodeId,
  options: VnJson,
  onClose?: () => void,
  onEvent?: (kind: string, detailJson: VnJson) => void,
): void;

/** The live half of `<VnWindow>`'s props (`title`, `alwaysOnTop`, `visible`, `fullResolution`, `ignoreMouseEvents`, `contentProtection`, `monitor`); other changes are fixed at creation: advised, ignored. */
declare function __vn_window_set_options(nodeId: VnNodeId, options: VnJson): void;

/** Where `nodeId`, as the PRIMARY window painted it, falls in the pop-out `popoutNodeId`: `[x, y, width, height]` JSON in the pop-out's CSS px; `undefined` when either was not painted. */
declare function __vn_window_map_rect(popoutNodeId: VnNodeId, nodeId: VnNodeId): VnJson | undefined;

declare function __vn_window_close(nodeId: VnNodeId): void;

declare function __vn_window_popouts(): VnJson;

/** TEST SURFACE. Dispatches a real platform click at the window's own client coordinates; `true` if it landed. */
declare function __vn_window_click(nodeId: VnNodeId, x: number, y: number): boolean;

/** A line into the host log. Always registered by the app host (callers do not probe); a bare embedder must provide it. */
declare function __vn_log(message: string): void;

/** Optional recording controls, absent without devtools. A nonzero token must be ended in finally. */
declare var __vn_trace_callback_begin: ((callback: unknown) => number) | undefined;
declare var __vn_trace_callback_end: ((token: number) => void) | undefined;

/** Polls background discovery: `{ pending: true }` until a scan completes, then every attachable instance as JSON (`{ preferred, self, targets }`). Absent with `--features no-devtools`. */
declare function __vn_devtools_targets(): VnJson;

/** `{ enabled, locked, port, pid, key, inspectors, slot }`. `enabled` off means NO listener; `locked` is `VN_DEVTOOLS=1` (refuses `disable()`); `key` is the `VN_DEVTOOLS_KEY` gate (declines F12 only). */
declare function __vn_devtools_state(): VnJson;

/** Turns the devtools on or off; answers the settled state or `{ error }` when refused. On binds a fresh port and token; off closes the listener and sockets, blanks the token and deletes the handshake. */
declare function __vn_devtools_set_enabled(on: boolean): VnJson;

declare function __vn_devtools_open(): VnJson;

/** How THIS process was started as an inspector (`vn-app --devtools`); published by another process, so readers guard. */
declare var __VN_DEVTOOLS_ARGS__:
  | {
      target: string | null;
      window: number | null;
      parent: number | null;
      embedded: boolean;
    }
  | undefined;

type VnPermissionState = "granted" | "denied" | "prompt" | "unsupported";
type VnPermissionName =
  "microphone" | "camera" | "display-capture" | "notifications" | "clipboard-read" | "clipboard-write" | "geolocation";

// appHost: registered ONLY when another program embeds vue-native; otherwise `appHost.available` is false.

/** The one callback calls settle and events arrive through: `(token, ok, text)`. Token `0` is an event (`text` is `{"event", "data"}`); else `ok` true means `text` is the JSON result, false the host's message. */
declare function __vn_host_set_dispatcher(fn: (token: number, ok: boolean, text: string) => void): void;
declare function __vn_host_call(name: string, argsJson: string): number;
declare function __vn_host_functions(): string;
declare const __VN_HOST__: import("./apphost").AppHost;

/** vn-desktop's half of `<input type=file|color>` and `<a download>`. The pickers resolve `null` when dismissed (or the same files were picked again) and `undefined` when nothing could be shown. */
declare var __VN_FS__:
  | {
      pickForInput(accept: string, multiple: boolean, directory: boolean, current: unknown): Promise<unknown>;
      pickColor(value: string): Promise<string | null | undefined>;
      download(url: string, downloadAttribute: string): void;
    }
  | undefined;

/** Transient user activation, read by vn-desktop's pickers and permission requests. */
declare var __VN_ACTIVATION__: { isActive(): boolean; consume(): void } | undefined;

/** Every SFC's `<style>` block, collected at module-eval time and joined into the first stylesheet by `mount()`. */
declare var __VN_CSS__: string[] | undefined;

/** Installed by `mount()`; the host calls it before re-evaluating a bundle. */
declare var __VN_TEARDOWN__: (() => void) | undefined;

/** The windowing host. Absent in the test harness and in embedders without the prelude (`runtime.ts` then exports an inert `appWindow`). */
declare var __VN_WINDOW__: VnWindowHost | undefined;

interface VnWindowHost {
  appWindow: VnAppWindow;
  escapeFullscreen?: () => boolean;
  defineWindow: (options: Record<string, unknown>) => void;
}

interface VnWindowRect {
  x: number;
  y: number;
  width: number;
  height: number;
}

/** `appWindow.chrome`: `client` is the client area; `contentInset` is how far host-drawn chrome insets the viewport in it (`inner* === client* - inset`; zero in custom-chrome mode). */
interface VnWindowChrome {
  titleBarStyle: string;
  displayMode: string;
  titleBarHeight: number;
  fallbackTitleBar: boolean;
  overlayVisible: boolean;
  hostControls: boolean;
  client: { width: number; height: number };
  contentInset: { left: number; top: number; right: number; bottom: number };
}

/** The window facade: an `EventTarget` (`resize`/`move`/`focus`) with Tauri/Electron-shaped verbs. Every verb is SYNCHRONOUS (JS and the UI thread are the same thread) and returns `undefined`. */
interface VnAppWindow extends VnEventTarget {
  minimize(): void;
  maximize(): void;
  unmaximize(): void;
  toggleMaximize(): void;
  restore(): void;
  close(): void;
  center(): void;
  /** Shows and raises the window, including one that started hidden (a tray click). */
  show(): void;
  startDragging(): void;
  startResizeDragging(): void;
  setFullscreen(on: boolean): void;
  isMaximized(): boolean;
  isMinimized(): boolean;
  isFullscreen(): boolean;
  isFocused(): boolean;
  getBounds(): VnWindowRect;
  getContentBounds(): VnWindowRect;
  setBounds(rect: Partial<VnWindowRect>): void;
  setContentBounds(rect: Partial<VnWindowRect>): void;
  setTitleBarOverlay(overlay: boolean | null | Record<string, unknown>): void;
  title: string;
  backdrop: string;
  /** What the window draws of `backdrop`: the same value, or `"none"` while a macOS native-fullscreen window draws flat. `backdropchange` fires when either moves. */
  readonly effectiveBackdrop: string;
  theme: string;
  alwaysOnTop: boolean;
  readonly scaleFactor: number;
  readonly chrome: VnWindowChrome;
  setProgressBar(...args: unknown[]): void;
  setOverlayIcon(...args: unknown[]): void;
  setBadgeCount(...args: unknown[]): void;
  setSkipTaskbar(...args: unknown[]): void;
  setIcon(...args: unknown[]): void;
}

/** Sticky activation: the host's dispatcher calls this on `keydown`/`mousedown` (gpui events never reach the prelude's EventTarget); without it `beforeunload` could never veto a close. */
declare var __vnStickyActivation: (() => void) | undefined;

declare var __VUE_HMR_RUNTIME__: VnVueHmrRuntime | undefined;

interface VnVueHmrRuntime {
  createRecord(id: string, def: unknown): boolean;
  rerender(id: string, render: unknown): void;
  reload(id: string, def: unknown): void;
}
