// The `__vn_*` host protocol. Ambient: engine globals, evaluated as classic scripts.
//
// - Ids are dense u32; `0` is the reserved "none" id.
// - Names cross once: tags, style keys and prop keys are interned via `__vn_intern`
//   and travel as `VnNameId`; a host fn taking a `VnNameId` misreads a raw string.
// - Structured answers are strings: comma-joined numbers, `name=value` lines, or JSON.
// - A host fn may be absent (storage, permissions, windowing, devtools are installed
//   by the app host): probe with `typeof fn === "function"`.

type VnNodeId = number;
type VnNameId = number;
type VnIdList = string;
/**
 * `"x,y,w,h"` as f32s, `""` when the node was not painted, or `"stale"` when it
 * has not been laid out yet and the host could not force a layout.
 * When a CSS `transform` moved the picture, four more follow,
 * `"x,y,w,h,tx,ty,tw,th"`: the drawn bounding box (getBoundingClientRect()'s answer);
 * the first four stay the layout box.
 */
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
/** Queries the owner OS window; an optional boolean queues its fullscreen state.
 * ownerRoot is zero for the primary window or a live VnWindow root id. */
declare function __vn_fullscreen_window(ownerRoot: VnNodeId, on?: boolean): boolean;

/**
 * One inline declaration. `""` REMOVES the property. A value may end in `!important`;
 * an updated key keeps its place, a new one is appended.
 */
declare function __vn_set_style(id: VnNodeId, key: VnNameId, value: string): void;
/** Does the engine accept `key: value` as one declaration? Absent on an older host. */
declare function __vn_style_accepts(key: VnNameId, value: string): boolean;
declare function __vn_set_class(id: VnNodeId, classes: string): void;
/** One attribute-channel prop. `undefined` (or `null`) REMOVES it; anything else must be a string. */
declare function __vn_set_prop(id: VnNodeId, key: VnNameId, value: string | undefined): void;
declare function __vn_add_scope(id: VnNodeId, scope: string): void;
/**
 * Replaces the whole stylesheet and marks every node for a fresh cascade.
 * `extra` are the `<style>` texts injected into `document.head`, in order; they
 * cascade after `css` and are parsed separately from it.
 */
declare function __vn_set_stylesheet(css: string, extra?: string[]): void;

/**
 * Arms or disarms one event bit on one node (`code` must stay below 64). Several JS
 * event names can share a code, so callers OR over all of them first.
 */
declare function __vn_set_listener(id: VnNodeId, code: VnEventCode, on: boolean): void;

/**
 * Installs the one dispatcher the host calls for EVERY input event. Returning
 * `false` means a handler called `preventDefault()`. The host drains microtasks
 * right after the call, so Vue's patch flush lands in the same frame's commit.
 */
declare function __vn_set_dispatcher(fn: VnDispatcher): void;

/**
 * Installs the observer step `layoutStep(op) -> number`, called after a frame's
 * layout and before its paint. `op` 0 gathers at depth 0 and answers how many
 * observers have entries; 1 calls the next one back and answers how many are left;
 * 2 gathers again at the depth the last broadcast reached, after a fresh layout;
 * 3 raises the loop error if an observation was skipped, updates intersections,
 * and answers bit 0 = loop error raised, bit 1 = something is still observed.
 */
declare function __vn_set_layout_hook(fn: (op: number) => number): void;

/**
 * `true`: the next frame runs the layout step even if it lays nothing out.
 * `false`: every observation was released.
 */
declare function __vn_layout_watch(observing: boolean): void;

/**
 * `(nodeId, code, a, b, c, d, key, data, x, y) -> proceed`. `a`-`d` and `key` are
 * one payload shape reused per event (see `VnEventPayload`). `data` is the
 * `clipboardData` DataTransfer for copy / cut / paste, else `undefined`. `x`/`y` are
 * the pointer in window CSS px, only for a `wheel` (whose `a`/`b` are its deltas).
 */
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

/**
 * Every event code the host can deliver, in lockstep with the host's `EV_*` table.
 * Not a `const enum` (bun downgrades it to a runtime object; `isolatedModules`
 * forbids ambient ones): the numbers live in `EV_CODES` in `runtime.ts`, and
 * `test/protocol-codes.test.ts` asserts it matches the Rust.
 */
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

/**
 * The document selection: `"anchor,anchorOffset,anchorOwn,focus,focusOffset,focusOwn"`
 * (DOM boundary points, UTF-16 offsets in a text), or `""` for none. `own` 1 is
 * an offset into an element's OWN text, which the runtime turns into a Text child.
 */
declare function __vn_selection_get(): string;
/** Sets anchor and focus; anchor 0 empties it. Answers whether both named live, attached nodes. */
declare function __vn_selection_set(anchor: VnNodeId, anchorOffset: number, focus: VnNodeId, focusOffset: number): boolean;
/** `Selection.toString()`: the selected text as shown — line breaks between blocks, `user-select: none` left out. */
declare function __vn_selection_text(): string;
/** `element`'s own text became the Text child `text`: a boundary in it moves with it. */
declare function __vn_selection_rebase(element: VnNodeId, text: VnNodeId): void;

/**
 * `elementFromPoint` (`all` 0) / `elementsFromPoint` (`all` 1): element ids under
 * the viewport point `(x, y)`, topmost first, comma-separated, skipping
 * `pointer-events: none` and `visibility: hidden`. `""` outside the viewport.
 */
declare function __vn_hit_at(x: number, y: number, all: 0 | 1): string;
declare function __vn_pointer_capture(nodeId: number, pointerId: number, capture: boolean): boolean;

declare var __vn_waapi_core: import("./waapi").NativeWaapiCore | undefined;

/** `__vn_query`'s mode: 0 descendants, 1 self (`matches`), 2 `closest`, 3 document subtree (root included). */
type VnQueryMode = 0 | 1 | 2 | 3;

/**
 * Selector matching through the cascade's own matcher.
 * Answers matched ids comma-joined, `""` for no match, or `"!<advisory>"` when the
 * engine cannot run the selector (`:has()`, `::before`).
 */
declare function __vn_query(selector: string, root: VnNodeId, mode: VnQueryMode): VnIdList;

/**
 * The node's rectangle at the LAST PAINTED FRAME, in client-area space. `""` for a
 * node that was not painted (detached, `display: none`, parked in a `<KeepAlive>`).
 * If the tree changed since the last layout the host lays out first, except inside
 * an input listener, where it answers `"stale"` (the runtime reads that as all-zero
 * and advises once per call site).
 */
declare function __vn_bounds(id: VnNodeId): VnRect;

declare function __vn_computed_style(id: VnNodeId): VnKvLines;

type VnScrollOp = 0 | 1 | 2;

/**
 * `"left,top,scrollWidth,scrollHeight,stale"` in CSS px. `stale` is `1` when the
 * content half (and the clamp derived from it) is from the last painted frame
 * because a layout could not be forced; the offset half is never stale.
 * `""` for a node without `overflow: scroll|auto`; the runtime substitutes 0.
 */
type VnScrollPos = string;

/**
 * Reads or moves a scroll container; the answer is the position after the clamp.
 * `x` / `y` are CSS px (positive = right / down). With op 1 a `NaN` axis is left
 * where it is; with op 2 it is 0. The DOM `scroll` event fires on the next frame.
 */
declare function __vn_scroll(id: VnNodeId, op: VnScrollOp, x: number, y: number): VnScrollPos;

type VnScrollAlign = 0 | 1 | 2 | 3;

/**
 * `element.scrollIntoView({ block, inline })` through every scrollable ancestor.
 * `""` when it ran, `"stale"` when it ran against a frame-old layout, `"!<advisory>"`
 * when it could not run (a node with no rectangle).
 */
declare function __vn_scroll_into_view(
  id: VnNodeId,
  block: VnScrollAlign,
  inline: VnScrollAlign,
): string;

/**
 * `el.focus()` (op 0) / `el.blur()` (op 1).
 * Focus answers `"moved:<old id>"` (the runtime fires blur/focusout on the old node,
 * 0 = none, then focus/focusin), `"already"`, `"pending"` (queued until the node is
 * built; the host fires the events), or `"refused"` (not focusable or not in the
 * document). Blur answers `"blurred"` or `""`.
 */
declare function __vn_focus(id: VnNodeId, op: 0 | 1): string;
declare function __vn_input_select(id: VnNodeId): boolean;

/** `location.reload()`: unloads and re-runs the app at the next frame. Returns at once. */
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

/**
 * ONE scalar media property (booleans are 0/1), or `undefined` for a node that is
 * not a live media element. Cheaper than `__vn_media_state`, which reads them all.
 */
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

/**
 * `<iframe>` postMessage. `wire` is the JSON payload; `targetOrigin` is `"*"`, `"/"`
 * (the app's own origin) or an origin. Dropped when the frame has no document or
 * its origin does not match.
 */
declare function __vn_iframe_post(id: VnNodeId, wire: string, targetOrigin: string): void;
declare function __vn_iframe_supported(): boolean;
declare var vnFeatures: { readonly iframe: boolean };

/** Drains the node's recent media events, comma-joined. For tests; events normally ride the EV_* channel. */
declare function __vn_media_events(id: VnNodeId): string;

// Multi-window. These move a subtree's PAINT into a second OS window while the node
// stays in this tree and reactive graph; `nodeId` is `mirrorNode.id` off a template
// ref. `<VnWindow>` (src/vnwindow.ts) is the only intended caller. Installed by the
// app host, so probe with `typeof __vn_window_open === "function"`.

/**
 * Moves the node's PAINT into a new OS window.
 *
 * `options` is JSON — `<VnWindow>`'s props, verbatim: `{ title?, x?, y?, width?,
 * height?, chrome?, backdrop?, alwaysOnTop?, resizable?, minWidth?, minHeight?,
 * titleBarHeight?, activate?, visible?, cover?, taskbar?, fullResolution?,
 * ignoreMouseEvents?, focusable?, contentProtection?, monitor? }`.
 *
 * `onClose` fires however the window closed. `onEvent` receives the window's other
 * reports: `("open", '{"windowId":N}')`, `("focus", "{}")`, `("blur", "{}")`,
 * `("resize", '{"width":W,"height":H,"scale":S}')`,
 * `("show", '{"firstFrameMs":T,"width":W,"height":H,"scale":S}')`, `("hide", "{}")`.
 * There is one JS realm, so `window.innerWidth` stays the primary window's.
 *
 * Returns `undefined`; the open is DEFERRED by one effect cycle (it cannot nest in
 * the primary window's update). Un-painting from the primary window is immediate.
 */
declare function __vn_window_open(
  nodeId: VnNodeId,
  options: VnJson,
  onClose?: () => void,
  onEvent?: (kind: string, detailJson: VnJson) => void,
): void;

/**
 * The live half of `<VnWindow>`'s props: `title`, `alwaysOnTop`, `visible`,
 * `fullResolution`, `ignoreMouseEvents`, `contentProtection`, `monitor`. Any other
 * changed value is fixed at creation, so it advises and does nothing.
 */
declare function __vn_window_set_options(nodeId: VnNodeId, options: VnJson): void;

/**
 * Where `nodeId`, as the PRIMARY window painted it, falls in the pop-out painting
 * `popoutNodeId`: `[x, y, width, height]` as JSON, in the pop-out's CSS px relative
 * to its painted root, across monitors and DPIs. `undefined` when either node was
 * not painted.
 */
declare function __vn_window_map_rect(popoutNodeId: VnNodeId, nodeId: VnNodeId): VnJson | undefined;

declare function __vn_window_close(nodeId: VnNodeId): void;

declare function __vn_window_popouts(): VnJson;

/** TEST SURFACE. Dispatches a real platform click at the window's own client coordinates; `true` if it landed. */
declare function __vn_window_click(nodeId: VnNodeId, x: number, y: number): boolean;

/**
 * A line into the host log. Always registered by the app host, so it is a `function`
 * and callers do not probe; a bare embedder must provide it.
 */
declare function __vn_log(message: string): void;

/** Optional recording controls; absent in hosts built without devtools.
 * A nonzero token must be ended in finally; zero means recording is inactive.
 */
declare var __vn_trace_callback_begin: ((callback: unknown) => number) | undefined;
declare var __vn_trace_callback_end: ((token: number) => void) | undefined;

/**
 * Polls background discovery: `{ pending: true }` until a scan completes, then every
 * attachable vue-native instance as JSON:
 * `{ preferred, self: { pid, dir }, targets: [{ slot, dir, port, pid, token, alive, self }] }`.
 * Absent in a host built with `--features no-devtools`; probe before calling.
 */
declare function __vn_devtools_targets(): VnJson;

// The devtools switch. Same availability and probe as `__vn_devtools_targets`.

/**
 * `{ enabled, locked, port, pid, key, inspectors, slot }`.
 * `enabled` off means there is NO listener. `locked` is `VN_DEVTOOLS=1`, which
 * refuses `disable()`. `key` is the `VN_DEVTOOLS_KEY` gate, which declines F12 only.
 */
declare function __vn_devtools_state(): VnJson;

/**
 * Turns the devtools on or off; returns the settled state or `{ error }` when refused.
 * On binds a fresh port and token and republishes `<dist>/devtools.json`; off closes
 * the listener, drops push sockets, blanks the token and deletes the handshake.
 */
declare function __vn_devtools_set_enabled(on: boolean): VnJson;

declare function __vn_devtools_open(): VnJson;

/**
 * How THIS process was started as an inspector (`vn-app --devtools`, which F12
 * spawns). Optional: it is published by another process, so readers guard.
 */
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
  | "microphone"
  | "camera"
  | "display-capture"
  | "notifications"
  | "clipboard-read"
  | "clipboard-write"
  | "geolocation";

declare function __vn_permissions_platform(): string;
declare function __vn_permissions_query(name: string): VnPermissionState | "";
declare function __vn_permissions_list(): VnJson;
/**
 * Starts a request. The JSON answer is `{ "error": … }` (not a permission name),
 * `{ "state": … }` (settled synchronously), or `{ "token": n }` (the per-frame pump
 * delivers the result to the dispatcher under that token).
 */
declare function __vn_permissions_request(name: string): VnJson;
/** Where the user turns a refused permission back on (macOS screen recording); `""` where the platform adds nothing. */
declare function __vn_permissions_hint(name: string): string;
declare function __vn_permissions_set_dispatcher(
  fn: (token: number, state: VnPermissionState) => void,
): void;

/** `{ path }` or `{ error }`. Creates nothing — a path is not a directory. */
declare function __vn_storage_user_folder(kind: string, subfolder: string): VnJson;
/**
 * Points `localStorage` at `path`, creating it. `{ path }` or `{ error }`.
 * `"keep"`: the app did not say — the format stays (encryption is sticky).
 */
declare function __vn_storage_set_folder(path: string, encrypted: "true" | "false" | "keep"): VnJson;
declare function __vn_storage_info(): VnJson;
declare function __vn_storage_flush(): VnJson;

// appHost. Registered ONLY when another program embeds vue-native; otherwise none of
// these exist and `appHost.available` is false.

/**
 * The one callback calls settle and events arrive through: `(token, ok, text)`.
 * Token `0` is an event, `text` `{"event", "data"}`. Otherwise `ok` true means
 * `text` is the JSON result, false the host's message.
 */
declare function __vn_host_set_dispatcher(fn: (token: number, ok: boolean, text: string) => void): void;
declare function __vn_host_call(name: string, argsJson: string): number;
declare function __vn_host_functions(): string;
declare const __VN_HOST__: import("./apphost").AppHost;

/**
 * The one callback the per-frame pump settles every desktop call through:
 * `(token, ok, payloadJson, value)`. `ok` false means `payloadJson` is
 * `{ name, message, code?, osError? }`. `value` is a JS value the host made on
 * delivery (a path-backed `File`, a `FileList`), else `undefined`. Token `0`
 * is an UNSOLICITED host event whose `payload.kind` names it (`"launch"`).
 */
declare function __vn_desktop_set_dispatcher(
  fn: (token: number, ok: boolean, payloadJson: VnJson, value: unknown) => void,
): void;
/**
 * `{ token }`, or a synchronous refusal `{ error, name, code? }`. `content`
 * true is the `<a href>` / `window.open` path: http, https and mailto only.
 * False (`appShell.openExternal`) takes any scheme but `file:`.
 */
declare function __vn_shell_open_external(url: string, content: boolean): VnJson;
/** `{ token }` or `{ error, name, code? }`. Executables refused unless `allowExecutable`. */
declare function __vn_shell_open_path(path: string, allowExecutable: boolean): VnJson;
declare function __vn_shell_show_item(path: string): VnJson;
declare function __vn_shell_trash(path: string): VnJson;
/**
 * This process's launch, resolved before the window existed:
 * `{ argv, cwd, url, paths, singleInstance, id, name }`. Forwarded launches arrive
 * later as token-0 `{ kind: "launch", … }` events.
 */
declare function __vn_launch_info(): VnJson;
/**
 * Registers `scheme://` for this exe under HKCU. `{ ok, wrote, scheme }`, or a
 * refusal `{ error, name: "TypeError" }` (invalid, reserved, foreign-owned).
 */
declare function __vn_launch_register_protocol(scheme: string): VnJson;
declare function __vn_launch_unregister_protocol(scheme: string): boolean;
declare function __vn_launch_is_protocol_registered(scheme: string): boolean;

// File System Access and dialogs. Every handle argument is a NUMBER into the host's
// handle table; no host fn takes or returns a path. `{ token }` answers settle through
// the dispatcher above; `{ error, name, code? }` is a synchronous refusal.

/** `(id, name, kind) => handle` — what a dropped path's handle is made with. */
declare function __vn_fs_set_handle_factory(fn: (h: number, name: string, kind: string) => unknown): void;
declare function __vn_fs_release(h: number): void;
/** Settles `{ name, size }` with the `File` as the value. */
declare function __vn_fs_get_file(h: number): VnJson;
declare function __vn_fs_child(dir: number, name: string, isDirectory: boolean, create: boolean): VnJson;
declare function __vn_fs_remove(dir: number, name: string, recursive: boolean): VnJson;
declare function __vn_fs_same(a: number, b: number): VnJson;
declare function __vn_fs_resolve(root: number, other: number): VnJson;
declare function __vn_fs_dir_open(dir: number): VnJson;
/** Settles `{ entries: {h, name, kind}[], done }`, 256 at most. */
declare function __vn_fs_dir_next(cursor: number): VnJson;
declare function __vn_fs_dir_close(cursor: number): void;
declare function __vn_fs_writable_open(h: number, keepExistingData: boolean): VnJson;
/** `position` -1 = at the cursor. `data`: a string, a Uint8Array or a Blob. */
declare function __vn_fs_writable_write(w: number, position: number, data: unknown): VnJson;
declare function __vn_fs_writable_seek(w: number, position: number): VnJson;
declare function __vn_fs_writable_truncate(w: number, size: number): VnJson;
declare function __vn_fs_writable_close(w: number): VnJson;
declare function __vn_fs_writable_abort(w: number): VnJson;
declare function __vn_fs_refuse_clone(proto: object, interfaceName: string): void;
/**
 * Makes instances of `proto` (FileSystemHandle.prototype) [Serializable]:
 * `idOf(handle)` names the id the host writes; a clone comes back through the
 * handle factory. The first call wins.
 */
declare function __vn_fs_set_serializer(proto: object, idOf: (handle: object) => number | undefined): void;
/** `"granted"` or `"prompt"`; `request` grants the handle's path (on a user activation only). */
declare function __vn_fs_permission(h: number, request: boolean): "granted" | "prompt";
/**
 * A picker: `{ kind: "open"|"save"|"folder", multiple?, accept?, types?,
 * excludeAcceptAllOption?, id?, startIn?, suggestedName?, result: "files"|"handles" }`.
 * Settles `{ files }` + a FileList, `{ handles }`, or `{ cancelled: true }`.
 * `currentFiles` is an input's FileList (the same selection again is a cancel).
 */
declare function __vn_dialog_open(requestJson: VnJson, currentFiles: unknown): VnJson;
/** Save As for a `blob:`/`data:` URL, resolved at the call. Settles `{ saved }` or `{ cancelled }`. */
declare function __vn_download(url: string, downloadAttribute: string): VnJson;
declare function __vn_desktop_get_screen_sources(typesJson: string, optionsJson: string): VnJson;

/** Every SFC's `<style>` block, collected at module-eval time and joined into the first stylesheet by `mount()`. */
declare var __VN_CSS__: string[] | undefined;

/** Installed by `mount()`; the host calls it before re-evaluating a bundle. Unmounts the app and frees every node the mirror holds. */
declare var __VN_TEARDOWN__: (() => void) | undefined;

/**
 * The windowing host. Absent in the unit-test harness and in embedders that do not
 * install the prelude; `runtime.ts` then exports an inert `appWindow`.
 */
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

/**
 * `appWindow.chrome` — the chrome-mode read. `client` is the window's client area;
 * `contentInset` is how far the app's viewport is inset in it by host-drawn chrome
 * (`inner* === client* - inset`; every inset is zero in custom-chrome mode).
 */
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

/**
 * The window facade: an `EventTarget` (`resize`/`move`/`focus`) with Tauri/Electron-
 * shaped verbs. Every verb is SYNCHRONOUS and returns `undefined`: JS and the UI
 * thread are the same thread, so a promise would misstate ordering.
 */
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
  /** What the window draws of `backdrop`: the same value, or `"none"` while a
   *  macOS native-fullscreen window draws a flat background instead. `backdropchange`
   *  fires when either moves. */
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

/**
 * Sticky activation. The host's dispatcher calls this on `keydown`/`mousedown`
 * because gpui events never reach the prelude's EventTarget; without it
 * `beforeunload` could never veto a close.
 */
declare var __vnStickyActivation: (() => void) | undefined;

declare var __VUE_HMR_RUNTIME__: VnVueHmrRuntime | undefined;

interface VnVueHmrRuntime {
  createRecord(id: string, def: unknown): boolean;
  rerender(id: string, render: unknown): void;
  reload(id: string, def: unknown): void;
}
