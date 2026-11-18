// The `__sk_*` host protocol. Ambient: engine globals, evaluated as classic scripts.
// Ids are dense u32 (`0` = none). Names cross once, interned via `__sk_intern` as `SkNameId`; a host fn taking a
// `SkNameId` misreads a raw string. Structured answers are strings: comma-joined numbers, `name=value` lines, or JSON.
// A host fn may be absent (storage, permissions, windowing, devtools come from the app host): probe with
// `typeof fn === "function"`.

type SkNodeId = number;
type SkNameId = number;
type SkIdList = string;
/** `"x,y,w,h"` as f32s, `""` when not painted, `"stale"` when the host could not force a layout. A CSS `transform` appends the drawn box `"tx,ty,tw,th"`. */
type SkRect = string;
/** `name=value` lines, `\n`-separated. `""` when there is nothing to report. */
type SkKvLines = string;
/** A JSON document. The host never returns a bare value — always an object. */
type SkJson = string;

/** Interns `name`. Idempotent: the same string always answers the same id. */
declare function __sk_intern(name: string): SkNameId;

declare function __sk_create_element(tag: SkNameId): SkNodeId;
declare function __sk_create_text(): SkNodeId;
declare function __sk_create_anchor(): SkNodeId;
/** Creates THE root node (`:root`, `html`, `body`). Called exactly once, by `createApp().mount()`. */
declare function __sk_create_root(): SkNodeId;

declare function __sk_set_text(id: SkNodeId, text: string): void;
/** Sets an ELEMENT's own text — exclusive with its children, as in the tree. */
declare function __sk_set_element_text(id: SkNodeId, text: string): void;

/** Inserts `child` into `parent` before `anchor` (`0` = append). Also a move: `child` is detached first. */
declare function __sk_insert(child: SkNodeId, parent: SkNodeId, anchor: SkNodeId): void;
/** Destroys `id` AND ITS WHOLE SUBTREE. The ids are recycled afterwards. */
declare function __sk_remove(id: SkNodeId): void;
/** Selects a connected fullscreen element in its owner window; zero restores the document. */
declare function __sk_fullscreen_element(id: SkNodeId): boolean;
/** Queries the owner OS window; an optional boolean queues its fullscreen state. `ownerRoot` is 0 for the primary window or a SkWindow root id. */
declare function __sk_fullscreen_window(ownerRoot: SkNodeId, on?: boolean): boolean;
/** Hands the host the registry the native DOM members answer nodes from. */
declare function __sk_set_node_lookup(lookup: (id: SkNodeId) => object | undefined): void;

/** One inline declaration: `""` REMOVES it; a value may end in `!important`; an updated key keeps its place. */
declare function __sk_set_style(id: SkNodeId, key: SkNameId, value: string): void;
/** Does the engine accept `key: value` as one declaration? Absent on an older host. */
declare function __sk_style_accepts(key: SkNameId, value: string): boolean;

/** `el.style` for a mirror element: a CSSStyleDeclaration over its inline declarations, reading `__skId` on every access. */
declare function __sk_style_object(node: object): import("./runtime/types").MirrorStyle;
/** `getComputedStyle(node)`: read-only, one property per read; anything that is not a mirror node reads the root's. */
declare function __sk_computed_object(node: unknown): import("./runtime/types").MirrorStyle;
/** runtime-dom's `setStyle` for one `:style` entry: the key as written (`backgroundColor`), `""` removes, a trailing `!important` is the priority. */
declare function __sk_style_assign(id: SkNodeId, key: string, value: string): void;
/** `__sk_style_assign` for a whole record `:style` as one change: `key\0value\0` per entry (`""` removes; `\x02` then `\x01value` per entry is an array, whose last accepted entry wins). Only declarations that differ from the inline ones are written. */
declare function __sk_style_patch(id: SkNodeId, entries: string): void;
/** One inline declaration's value by CSS name, `""` when there is none. */
declare function __sk_style_get(id: SkNodeId, name: string): string;
/** `style.cssText = text`, or `setAttribute("style", text)` (`asAttribute`, which keeps `text` verbatim). */
declare function __sk_style_replace(id: SkNodeId, text: string, asAttribute: boolean): void;
/** `removeAttribute("style")`. */
declare function __sk_style_clear(id: SkNodeId): void;
/** The `style` attribute: `null` when absent. */
declare function __sk_style_attr(id: SkNodeId): string | null;
/** Called after an inline change that adds or removes the attribute, or every change while observed (`__sk_style_observe`). */
declare function __sk_style_hook(hook: (id: SkNodeId, oldValue: string | null, present: boolean) => void): void;
declare function __sk_style_observe(on: boolean): void;

declare function __sk_set_class(id: SkNodeId, classes: string): void;
/** One attribute-channel prop. `undefined` (or `null`) REMOVES it; anything else must be a string. */
declare function __sk_set_prop(id: SkNodeId, key: SkNameId, value: string | undefined): void;
declare function __sk_add_scope(id: SkNodeId, scope: string): void;
declare function __sk_copy_scopes(from: SkNodeId, to: SkNodeId): void;
/** Before a click's listeners: toggles a checkbox or radio and answers what `__sk_toggle_end` restores, or -1. */
declare function __sk_toggle_begin(id: SkNodeId): number;
/** After them: restores a cancelled click's state; `true` when `input` and `change` fire. */
declare function __sk_toggle_end(id: SkNodeId, token: number, cancelled: boolean): boolean;
/** Replaces the whole stylesheet; `extra` are the `document.head` `<style>` texts, cascaded after `css` and parsed separately. */
declare function __sk_set_stylesheet(css: string, extra?: string[]): void;

/** Arms or disarms one event bit on one node (`code` < 64). Names can share a code, so callers OR over all of them first. */
declare function __sk_set_listener(id: SkNodeId, code: SkEventCode, on: boolean): void;

/** Installs the one dispatcher the host calls for EVERY input event; returning `false` means a handler called `preventDefault()`. The host drains microtasks after the call. */
declare function __sk_set_dispatcher(fn: SkDispatcher): void;

/**
 * Installs the observer step `layoutStep(op) -> number`, called after layout and before paint. `op` 0 gathers at
 * depth 0 (answers how many observers have entries); 1 calls the next back (answers how many are left); 2 gathers
 * again at the last broadcast depth after a fresh layout; 3 raises the loop error if an observation was skipped,
 * updates intersections, and answers bit 0 = loop error raised, bit 1 = something still observed.
 */
declare function __sk_set_layout_hook(fn: (op: number) => number): void;

/** `true`: the next frame runs the layout step even if nothing lays out. `false`: every observation was released. */
declare function __sk_layout_watch(observing: boolean): void;

/** `(nodeId, code, a, b, c, d, key, data, x, y, pen) -> proceed`. `a`-`d`/`key` are per-event (`SkEventPayload`); `data` is the clipboard DataTransfer for copy/cut/paste; `x`/`y` are window CSS px for `wheel`, the movement for `mousemove`; `pen` is `[flags, count, ...readings, ...predicted]` for an event a pen caused, or a mouse move's positions since the last one (`pointerSource`). */
type SkDispatcher = (
  node: SkNodeId,
  code: SkEventCode,
  a: number,
  b: number,
  c: number,
  d: number,
  key: string | undefined,
  data: unknown,
  x?: number,
  y?: number,
  pen?: number[],
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
 * | `surfaceresize`           | width (px)   | height (px) | scale       | —          | —               |
 * | `surfaceready`            | frame width  | frame height | —          | —          | —               |
 * | `surfacelost`             | —            | —           | —           | —          | "cleared"/"device" |
 * | `surfacedisplaychange`    | headroom     | potential headroom | refresh Hz (0 ?) | SDR white nits (0 ?) | "hdr", "output-hdr", "vrr"/"fixed", space-separated |
 * | transitionend             | elapsedTime  | —           | —           | —          | property name   |
 * | animation{start,end,cancel} | elapsedTime | —          | —           | —          | @keyframes name |
 * | copy / cut / paste        | —            | —           | —           | —          | —  (`data` is the DataTransfer) |
 *
 * `mods` is a bitfield: 1 ctrl, 2 shift, 4 alt, 8 meta. `mousedown` / `mouseup` /
 * `click` carry the click count (`MouseEvent.detail`) above it. A keyboard `click`
 * (detail 0) carries `"Enter"` or `" "` in `key`; a mouse click leaves it empty.
 */
interface SkEventPayload {
  a: number;
  b: number;
  c: number;
  d: number;
  key: string | undefined;
}

/** Every event code the host can deliver, in lockstep with the host's `EV_*` table (`EV_CODES` in `runtime/event-names.ts`; `test/protocol-codes.test.ts` checks it against the Rust). Not a `const enum`: bun downgrades it and `isolatedModules` forbids ambient ones. */
type SkEventCode =
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
  | 52 // message            (an <iframe>'s document to its parent)
  | 53 // pointerlockchange  (`a` = the request it settles, 0: none)
  | 54 // pointerlockerror   (`a` = the request, `key` = "Name: message")
  | 55 // surfaceresize      (<sk-surface>)
  | 56 // surfaceready
  | 57 // surfacelost
  | 58 // surfacedisplaychange
  | 59 // mouseover
  | 60; // mouseout

/** The document selection: `"anchor,anchorOffset,anchorOwn,focus,focusOffset,focusOwn"` (UTF-16 offsets), `""` for none. `own` 1 is an offset into an element's OWN text, which the runtime turns into a Text child. */
declare function __sk_selection_get(): string;
/** Sets anchor and focus; anchor 0 empties it. Answers whether both named live, attached nodes. */
declare function __sk_selection_set(
  anchor: SkNodeId,
  anchorOffset: number,
  focus: SkNodeId,
  focusOffset: number,
): boolean;
declare function __sk_selection_text(): string;
/** `element`'s own text became the Text child `text`: a boundary in it moves with it. */
declare function __sk_selection_rebase(element: SkNodeId, text: SkNodeId): void;

/** `elementFromPoint` (`all` 0) / `elementsFromPoint` (`all` 1): comma-separated ids under viewport point `(x, y)`, topmost first; skips `pointer-events: none` and `visibility: hidden`. */
declare function __sk_hit_at(x: number, y: number, all: 0 | 1): string;
declare function __sk_pointer_capture(nodeId: number, pointerId: number, capture: boolean): boolean;

declare var __sk_waapi_core: import("./waapi").NativeWaapiCore | undefined;

/** `__sk_query`'s mode: 0 descendants, 1 self (`matches`), 2 `closest`, 3 document subtree (root included). */
type SkQueryMode = 0 | 1 | 2 | 3;

/** Selector matching through the cascade's own matcher: comma-joined ids, `""` for none, `"!<advisory>"` when the engine cannot run the selector. */
declare function __sk_query(selector: string, root: SkNodeId, mode: SkQueryMode): SkIdList;

/** The node's rectangle at the LAST PAINTED FRAME, in client-area space. Lays out first if the tree changed, except inside an input listener, where it answers `"stale"`. */
declare function __sk_bounds(id: SkNodeId): SkRect;

/**
 * `el.focus()` (op 0) / `el.blur()` (op 1). Focus answers `"moved:<old id>"` (the runtime fires blur/focusout on the
 * old node, 0 = none, then focus/focusin), `"already"`, `"pending"` (queued until the node is built; the host fires
 * the events) or `"refused"`. Blur answers `"blurred"` or `""`.
 */
declare function __sk_focus(id: SkNodeId, op: 0 | 1): string;
declare function __sk_input_select(id: SkNodeId): boolean;
/** Opens the in-window color picker for a color input; `false` when the app configured the OS dialog. */
declare function __sk_color_picker(id: SkNodeId): boolean;
/** A color input's sanitized value for its `alpha`, `colorspace` and `hdr` attributes. */
declare function __sk_color_value(text: string, alpha: boolean, colorspace: string, hdr: boolean): string;

declare function __sk_reload(): void;

declare function __sk_media_state(id: SkNodeId): SkKvLines;

type SkMediaProp =
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

/** ONE scalar media property (booleans 0/1), `undefined` for a node that is not a live media element. Cheaper than `__sk_media_state`. */
declare function __sk_media_prop(id: SkNodeId, prop: SkMediaProp): number | undefined;

type SkMediaVerb =
  | 0 // play
  | 1 // pause
  | 2 // currentTime =
  | 3 // volume =
  | 4 // muted =        (arg: 0 | 1)
  | 5 // playbackRate =
  | 6; // loop =        (arg: 0 | 1)

/** Runs one verb. Returns whether the node was a live media element at all. */
declare function __sk_media_call(id: SkNodeId, verb: SkMediaVerb, arg: number): boolean;

/** `canPlayType(type)`, probed on THIS machine. */
declare function __sk_media_can_play(id: SkNodeId, mime: string): "" | "maybe" | "probably";

/** `<iframe>` postMessage: `wire` is the JSON payload; `targetOrigin` is `"*"`, `"/"` (the app's own origin) or an origin. Dropped when there is no document or the origin does not match. */
declare function __sk_iframe_post(id: SkNodeId, wire: string, targetOrigin: string): void;
declare function __sk_iframe_supported(): boolean;
declare var skFeatures: { readonly iframe: boolean };

/** Drains the node's recent media events, comma-joined (tests only). */
declare function __sk_media_events(id: SkNodeId): string;

// Multi-window: these move a subtree's PAINT into a second OS window while the node stays in this tree; `nodeId` is
// `mirrorNode.id`. `<SkWindow>` is the only intended caller. Probe with `typeof __sk_window_open === "function"`.

/**
 * Moves the node's PAINT into a new OS window. `options` is JSON: `<SkWindow>`'s props, verbatim. `onClose` fires
 * however the window closed; `onEvent` gets `("open" | "focus" | "blur" | "resize" | "show" | "hide", detailJson)`,
 * e.g. `("open", '{"windowId":N}')`, `("resize", '{"width":W,"height":H,"scale":S}')`.
 * Returns `undefined`: the open is DEFERRED by one effect cycle (it cannot nest in the primary window's update).
 */
declare function __sk_window_open(
  nodeId: SkNodeId,
  options: SkJson,
  onClose?: () => void,
  onEvent?: (kind: string, detailJson: SkJson) => void,
): void;

/** The live half of `<SkWindow>`'s props (`title`, `bounds`, `backdrop`, `alwaysOnTop`, `titleBarHeight`, `activate`, `visible`, `fullResolution`, `ignoreMouseEvents`, `contentProtection`, `monitor`); other changes are fixed at creation: advised, ignored. */
declare function __sk_window_set_options(nodeId: SkNodeId, options: SkJson): void;

/** A caption button's click activation: performs the `--window-control` action of the nearest ancestor-or-self declaring one, as a real press does. `false` when there is none. */
declare function __sk_window_control_click(nodeId: SkNodeId): boolean;

/** Where `nodeId`, as the PRIMARY window painted it, falls in the pop-out `popoutNodeId`: `[x, y, width, height]` JSON in the pop-out's CSS px; `undefined` when either was not painted. */
declare function __sk_window_map_rect(popoutNodeId: SkNodeId, nodeId: SkNodeId): SkJson | undefined;

declare function __sk_window_close(nodeId: SkNodeId): void;

declare function __sk_window_popouts(): SkJson;

/** TEST SURFACE. Dispatches a real platform click at the window's own client coordinates; `true` if it landed. */
declare function __sk_window_click(nodeId: SkNodeId, x: number, y: number): boolean;

/** A line into the host log. Always registered by the app host (callers do not probe); a bare embedder must provide it. */
declare function __sk_log(message: string): void;
/** `console.warn`s `[skathi compat] message` the first time `key` is advised in this process; false after. */
declare function __sk_advise(key: string, message: string): boolean;
declare function __sk_advised(key: string): boolean;

/** The build's define: true under `sk dev`, false in a production bundle, where every `if (__DEV__)` branch is dropped. */
declare const __DEV__: boolean;
/** A dev-build performance advisory (`code`: `wasted-render`, `long-list`, `host-writes`): a hint on the frame being recorded, and `[skathi perf] message` in the console the first time `key` is seen. */
declare function __sk_perf_advise(code: string, key: string, message: string): boolean;
/** Counts the frames the host has built; JS work between two values belongs to one frame. */
declare function __sk_perf_frame(): number;
/** `"<elements>,<in view>"`: the element descendants of `id` and how many of them the last frame painted inside the nearest scroll container; `""` outside one or before a paint. */
declare function __sk_perf_visible(id: SkNodeId): string;

/** Optional recording controls, absent without devtools. A nonzero token must be ended in finally. */
declare var __sk_trace_callback_begin: ((callback: unknown) => number) | undefined;
declare var __sk_trace_callback_end: ((token: number) => void) | undefined;

/** Polls background discovery: `{ pending: true }` until a scan completes, then every attachable instance as JSON (`{ preferred, self, targets }`). Absent with `--features no-devtools`. */
declare function __sk_devtools_targets(): SkJson;

/** How THIS process was started as an inspector (`sk-app --devtools`); published by another process, so readers guard. */
declare var __SK_DEVTOOLS_ARGS__:
  | {
      target: string | null;
      window: number | null;
      parent: number | null;
      embedded: boolean;
    }
  | undefined;

type SkPermissionState = "granted" | "denied" | "prompt" | "unsupported";
type SkPermissionName =
  "microphone" | "camera" | "display-capture" | "notifications" | "clipboard-read" | "clipboard-write" | "geolocation";

// appHost: registered ONLY when another program embeds Skathi; otherwise `appHost.available` is false.

/** The one callback calls settle and events arrive through: `(token, ok, text)`. Token `0` is an event (`text` is `{"event", "data"}`); else `ok` true means `text` is the JSON result, false the host's message. */
declare function __sk_host_set_dispatcher(fn: (token: number, ok: boolean, text: string) => void): void;
declare function __sk_host_call(name: string, argsJson: string): number;
declare function __sk_host_functions(): string;
declare const __SK_HOST__: import("./apphost").AppHost;
/** A native module the embedder registered, built before app code; `undefined` for a name it did not. */
declare function __sk_native_module(name: string): Record<string, unknown> | undefined;

/** sk-desktop's half of `<input type=file|color>` and `<a download>`. The pickers resolve `null` when dismissed (or the same files were picked again) and `undefined` when nothing could be shown. */
declare var __SK_FS__:
  | {
      pickForInput(accept: string, multiple: boolean, directory: boolean, current: unknown): Promise<unknown>;
      pickColor(value: string): Promise<string | null | undefined>;
      download(url: string, downloadAttribute: string): void;
    }
  | undefined;

/** Transient user activation, read by sk-desktop's pickers and permission requests. */
declare var __SK_ACTIVATION__: { isActive(): boolean; consume(): void } | undefined;

/** Every SFC's `<style>` block, collected at module-eval time and joined into the first stylesheet by `mount()`. */
declare var __SK_CSS__: string[] | undefined;

/** Installed by `mount()`; the host calls it before re-evaluating a bundle. */
declare var __SK_TEARDOWN__: (() => void) | undefined;

/** The windowing host. Absent in the test harness and in embedders without the prelude (`runtime.ts` then exports an inert `appWindow`). */
declare var __SK_WINDOW__: SkWindowHost | undefined;

interface SkWindowHost {
  appWindow: SkAppWindow;
  escapeFullscreen?: () => boolean;
  defineWindow: (options: Record<string, unknown>) => void;
  /** Calls `fn` after any `appWindow` state may have changed (a new snapshot, a title set). Returns the unsubscribe. */
  onChange?: (fn: () => void) => () => void;
}

interface SkWindowRect {
  x: number;
  y: number;
  width: number;
  height: number;
}

/** `appWindow.chrome`: `client` is the client area; `contentInset` is how far host-drawn chrome insets the viewport in it (`inner* === client* - inset`; zero in custom-chrome mode). */
interface SkWindowChrome {
  titleBarStyle: string;
  displayMode: string;
  /** The Window Controls Overlay area's height (`env(titlebar-area-height)`): 0 while the area is not visible (OS chrome, fullscreen, minimized, or the fallback title bar still up). */
  titleBarHeight: number;
  fallbackTitleBar: boolean;
  overlayVisible: boolean;
  hostControls: boolean;
  client: { width: number; height: number };
  contentInset: { left: number; top: number; right: number; bottom: number };
}

/**
 * The window facade: an `EventTarget` (`resize`/`move`/`focus`/`blur`, `maximize`/`unmaximize`, `minimize`/`restore`, `fullscreenchange`, `backdropchange`, `close-requested`) with Tauri/Electron-shaped verbs. Every verb is SYNCHRONOUS (JS and the UI thread are the same thread) and returns `undefined`.
 * Its state reads (getters, `is*()`, `get*()`) are reactive in Vue templates, `computed` and `watch`, and, once
 * `@skathi-native/runtime/svelte` is imported, in Svelte templates, `$derived` and `$effect`.
 *
 * `close-requested` (cancelable) fires when the user asks to close the window (title bar, Alt+F4, taskbar);
 * `preventDefault()` keeps it open, e.g. to `hide()` it to a tray. `close()`, a tray `role: "quit"` item and the
 * session ending are not asked; `pagehide` fires for every close.
 */
interface SkAppWindow extends EventTarget {
  minimize(): void;
  maximize(): void;
  unmaximize(): void;
  toggleMaximize(): void;
  restore(): void;
  close(): void;
  center(): void;
  /** Shows and raises the window, including one that started hidden or was `hide()`n (a tray click). */
  show(): void;
  /** Hides the window and its taskbar button; the app keeps running. Where hiding is unsupported, minimizes. */
  hide(): void;
  isVisible(): boolean;
  startDragging(): void;
  startResizeDragging(): void;
  setFullscreen(on: boolean): void;
  isMaximized(): boolean;
  isMinimized(): boolean;
  isFullscreen(): boolean;
  isFocused(): boolean;
  getBounds(): SkWindowRect;
  getContentBounds(): SkWindowRect;
  setBounds(rect: Partial<SkWindowRect>): void;
  setContentBounds(rect: Partial<SkWindowRect>): void;
  setTitleBarOverlay(overlay: boolean | null | Record<string, unknown>): void;
  title: string;
  backdrop: string;
  /** What the window draws of `backdrop`: the same value, or `"none"` while a macOS native-fullscreen window draws flat. `backdropchange` fires when either moves. */
  readonly effectiveBackdrop: string;
  theme: string;
  alwaysOnTop: boolean;
  readonly scaleFactor: number;
  readonly chrome: SkWindowChrome;
  setProgressBar(...args: unknown[]): void;
  setOverlayIcon(...args: unknown[]): void;
  setBadgeCount(...args: unknown[]): void;
  setSkipTaskbar(...args: unknown[]): void;
  setIcon(...args: unknown[]): void;
}

/** Sticky activation: the host's dispatcher calls this on `keydown`/`mousedown` (gpui events never reach the prelude's EventTarget); without it `beforeunload` could never veto a close. */
declare var __skStickyActivation: (() => void) | undefined;

declare var __VUE_HMR_RUNTIME__: SkVueHmrRuntime | undefined;

interface SkVueHmrRuntime {
  createRecord(id: string, def: unknown): boolean;
  rerender(id: string, render: unknown): void;
  reload(id: string, def: unknown): void;
}
