// The `__vn_*` host protocol, typed once.
//
// Every function below is registered on the JS global by a Rust `install()`
// (the file is named on each block), and every one of them is the ONLY way the
// JS runtime can reach the retained tree, the cascade, the media pipeline, the
// permission backend, the storage area or the voice engine. Until this file
// existed the whole seam was untyped: a call with the arguments in the wrong
// order, or a `number` where the host reads a string, produced a silently
// wrong-but-plausible result rather than a build error.
//
// The rules the seam obeys, which the types encode:
//
//   * IDS ARE DENSE u32, and `0` is the reserved "none" id (never a real node).
//     Everything crossing the boundary as an id is a `VnNodeId` / `VnNameId`.
//   * NAMES CROSS ONCE. Tags, style keys and prop keys are interned through
//     `__vn_intern` and travel as `VnNameId` afterwards, so the per-op hot path
//     never carries a string. A host fn that takes a `VnNameId` will silently
//     read garbage if handed a raw string, which is precisely what the types
//     now prevent.
//   * STRUCTURED ANSWERS ARE STRINGS, in one of three shapes: comma-joined
//     numbers (`__vn_query`, `__vn_bounds`), `name=value` lines
//     (`__vn_computed_style`, `__vn_media_state`), or JSON
//     (`__vn_permissions_*`, `__vn_storage_*`, `__vn_voice_*`). The alias types
//     below say which, so a caller cannot `JSON.parse` a line-shaped answer.
//   * A HOST FN MAY BE ABSENT. `vn-storage`, `vn-permissions` and `vn-voice`
//     are installed by the app host, not by the engine, and a build without
//     them (or the unit-test harness) simply has no such global. The runtime
//     probes with `typeof __vn_storage_info === "function"` and falls back to
//     an inert surface — so these are declared as ordinary globals and the
//     probe is what keeps that honest.
//
// Ambient by design: these are engine globals, not module exports, and the
// files that call them are evaluated as CLASSIC SCRIPTS inside the vendor IIFE.

/** A node id in `crates/vn-tree`'s arena. `0` is the reserved "none" id. */
type VnNodeId = number;
/** An interned name id (`__vn_intern`). Dense, stable for the process. */
type VnNameId = number;
/** `"12,40,3"` — matched node ids, or `""` for none. See `__vn_query`. */
type VnIdList = string;
/** `"x,y,w,h"` as f32s, or `""` when the node was not painted. */
type VnRect = string;
/** `name=value` lines, `\n`-separated. `""` when there is nothing to report. */
type VnKvLines = string;
/** A JSON document. The host never returns a bare value — always an object. */
type VnJson = string;

// ---------------------------------------------------------------------------
// the retained tree — crates/vn-tree/src/lib.rs::install
// ---------------------------------------------------------------------------

/**
 * Interns `name`, returning its dense id. Idempotent: the same string always
 * answers the same id, which is what lets the JS side memoise it forever.
 */
declare function __vn_intern(name: string): VnNameId;

/** Creates an element node with the interned `tag`. Returns its id. */
declare function __vn_create_element(tag: VnNameId): VnNodeId;
/** Creates a text node (`kind === 2`). Returns its id. */
declare function __vn_create_text(): VnNodeId;
/** Creates a comment/anchor node (`kind === 3`). Returns its id. */
declare function __vn_create_anchor(): VnNodeId;
/**
 * Creates THE root node. Called exactly once, by `createApp().mount()`.
 * `vn_css::element` answers `:root`, `html` and `body` off this one node.
 */
declare function __vn_create_root(): VnNodeId;

/** Sets a text node's content. */
declare function __vn_set_text(id: VnNodeId, text: string): void;
/** Sets an ELEMENT's own text — exclusive with its children, as in the tree. */
declare function __vn_set_element_text(id: VnNodeId, text: string): void;

/**
 * Inserts `child` into `parent` before `anchor` (`0` = append). Also MOVE: the
 * host detaches `child` from wherever it was first.
 */
declare function __vn_insert(child: VnNodeId, parent: VnNodeId, anchor: VnNodeId): void;
/** Destroys `id` AND ITS WHOLE SUBTREE. The ids are recycled afterwards. */
declare function __vn_remove(id: VnNodeId): void;

/** One inline declaration. `""` REMOVES the property (it does not set empty). */
declare function __vn_set_style(id: VnNodeId, key: VnNameId, value: string): void;
/** The element's whole class attribute, space-joined. Not additive. */
declare function __vn_set_class(id: VnNodeId, classes: string): void;
/**
 * One attribute-channel prop. `undefined` (or `null`) REMOVES it — every other
 * value must already be a string, because the host reads it as one.
 */
declare function __vn_set_prop(id: VnNodeId, key: VnNameId, value: string | undefined): void;
/** Adds a `<style scoped>` data-v id to an element. Additive, never replaces. */
declare function __vn_add_scope(id: VnNodeId, scope: string): void;
/** Replaces the whole stylesheet and marks every node for a fresh cascade. */
declare function __vn_set_stylesheet(css: string): void;

/**
 * Arms or disarms one event bit on one node (`Node::listeners` is a u64, so
 * `code` must stay below 64). The host only delivers an event whose bit is
 * armed, and several JS event NAMES can share one code — see `syncListenerFlag`
 * in `runtime.ts`, which ORs over all of them before calling this.
 */
declare function __vn_set_listener(id: VnNodeId, code: VnEventCode, on: boolean): void;

/**
 * Installs the one dispatcher the host calls for EVERY input event. Returning
 * `false` means a JS handler called `preventDefault()`, which the host honours
 * (`vn_tree::dispatch_event`). The host drains microtasks immediately after the
 * call, so Vue's patch flush lands inside the same frame's commit.
 */
declare function __vn_set_dispatcher(fn: VnDispatcher): void;

/**
 * `(nodeId, code, a, b, c, d, key) -> proceed`.
 *
 * The four numbers and the optional string are ONE payload shape reused for
 * every event; `VnEventPayload` below documents what each slot means per
 * event, mirroring `vn_tree::EventPayload`'s own doc comment.
 */
type VnDispatcher = (
  node: VnNodeId,
  code: VnEventCode,
  a: number,
  b: number,
  c: number,
  d: number,
  key: string | undefined,
) => boolean;

/**
 * What `a`/`b`/`c`/`d`/`key` carry, per event family. Documentation with a
 * compiler behind it: `synthesize()` in `runtime.ts` destructures exactly these.
 *
 * | family                    | a            | b           | c           | d          | key             |
 * |---------------------------|--------------|-------------|-------------|------------|-----------------|
 * | mouse (click/down/up/…)   | clientX      | clientY     | button      | mods       | —               |
 * | wheel                     | deltaX       | deltaY      | —           | mods       | —               |
 * | keydown/keyup             | —            | —           | —           | mods       | `KeyboardEvent.key` |
 * | input                     | —            | —           | —           | —          | the new value   |
 * | scroll                    | scrollLeft   | scrollTop   | —           | —          | —               |
 * | media (crates/vn-video)    | currentTime  | duration    | readyState  | playbackRate | —             |
 * | media `error`             | MediaError.code | currentTime | —        | —          | advisory text   |
 * | lottie `enterFrame`       | currentTime  | totalTime   | elapsed     | progress   | —               |
 * | lottie `loopComplete`     | currentTime  | totalTime   | currentLoop | —          | —               |
 * | lottie `complete`         | currentTime  | totalTime   | loops       | —          | —               |
 * | transitionend             | elapsedTime  | —           | —           | —          | property name   |
 * | animation{start,end,cancel} | elapsedTime | —          | —           | —          | @keyframes name |
 *
 * `mods` is a bitfield: 1 ctrl, 2 shift, 4 alt, 8 meta.
 */
interface VnEventPayload {
  a: number;
  b: number;
  c: number;
  d: number;
  key: string | undefined;
}

/**
 * Every event code the host can deliver, in lockstep with
 * `crates/vn-tree/src/tree.rs` (`pub const EV_*` / `EVENT_NAMES`).
 *
 * A TypeScript `const enum` was the obvious spelling and is deliberately NOT
 * used: bun does not inline one, it DOWNGRADES it to a runtime enum object, so
 * the codes would stop being free and `vendor.js` would grow a table it does
 * not need — and `isolatedModules` forbids an ambient one outright. The numbers
 * therefore stay the `EV_CODES` object literal in `runtime.ts`, which is
 * `satisfies`-checked against this type, and `test/protocol-codes.test.ts`
 * parses `tree.rs` and asserts the two tables are identical. That test is the
 * lockstep guarantee a `const enum` could never have given: it reads the Rust.
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
  | 35 // transitionend       (vn-css animator)
  | 36 // animationstart
  | 37 // animationend
  | 38; // animationcancel

// ---------------------------------------------------------------------------
// the DOM surface — crates/vn-tree/src/dom.rs
// ---------------------------------------------------------------------------

/** `__vn_query`'s mode: 0 descendants, 1 self only (`matches`), 2 `closest`. */
type VnQueryMode = 0 | 1 | 2;

/**
 * Selector matching through the CASCADE'S OWN matcher — never a second JS-side
 * one, which would be a silent divergence from what actually styles the node.
 *
 * Answers the matched ids comma-joined, `""` for no match, or `"!<advisory>"`
 * when the selector is one the engine cannot run (`:has()`, `::before`). The
 * `!` prefix is why the return type is a plain string rather than a list.
 */
declare function __vn_query(selector: string, root: VnNodeId, mode: VnQueryMode): VnIdList;

/**
 * The rectangle gpui laid the node out at during the LAST PAINTED FRAME, in the
 * window's client-area space. `""` for a node that was not painted (detached,
 * `display: none`, parked in a `<KeepAlive>` cache) — which is exactly the
 * all-zero rect a browser reports for those, so it needs no advisory.
 */
declare function __vn_bounds(id: VnNodeId): VnRect;

/** The folded computed values + every custom property, as `name=value` lines. */
declare function __vn_computed_style(id: VnNodeId): VnKvLines;

/** `__vn_scroll`'s verb: 0 read, 1 set absolute, 2 scroll by. */
type VnScrollOp = 0 | 1 | 2;

/**
 * `"left,top,scrollWidth,scrollHeight,stale"` in CSS px — the whole of
 * `scrollTop`, `scrollLeft`, `scrollTo`, `scrollBy`, `scrollWidth` and
 * `scrollHeight` in one call, because all six read the same `ScrollHandle`.
 *
 * `stale` is `1` when the CONTENT half of the answer (and the clamp derived
 * from it) is from the last painted frame, because the tree changed in this
 * same JS wave and a layout could not be forced. The offset half is never
 * stale. The runtime turns it into one advisory.
 *
 * `""` for a node the cascade never gave `overflow: scroll|auto`. That is not
 * an error: a browser reports 0 for `scrollTop` on every non-scrollable
 * element and silently ignores a write to it, which is what the runtime
 * substitutes.
 */
type VnScrollPos = string;

/**
 * Reads or moves a scroll container, through the very `ScrollHandle` gpui
 * paints the subtree with — so the answer is the position AFTER the clamp, and
 * a write is readable back on the next line with no frame in between.
 *
 * `x` / `y` are CSS px (positive = right / down). With op 1 a `NaN` axis is
 * left where it is, which is what `scrollTo({ top })` with no `left` means;
 * with op 2 it is 0. The DOM `scroll` event is fired on the next frame, not
 * from inside the call — CSSOM-View fires it from the update-the-rendering
 * loop and so do we (`crates/vn-tree/src/scroll.rs`).
 */
declare function __vn_scroll(id: VnNodeId, op: VnScrollOp, x: number, y: number): VnScrollPos;

/** `scrollIntoView`'s alignment: 0 start, 1 center, 2 end, 3 nearest. */
type VnScrollAlign = 0 | 1 | 2 | 3;

/**
 * `element.scrollIntoView({ block, inline })` — scrolls every scrollable
 * ancestor, innermost first, clamping each and carrying the element's
 * rectangle down the chain by however far each one actually moved.
 *
 * `""` when it ran, `"stale"` when it ran against a layout a frame old (see
 * `__vn_scroll`), and `"!<advisory>"` when it could not run at all (a node
 * with no rectangle), in the same shape `__vn_query` uses.
 */
declare function __vn_scroll_into_view(
  id: VnNodeId,
  block: VnScrollAlign,
  inline: VnScrollAlign,
): string;

// ---------------------------------------------------------------------------
// `<video>` / `<audio>` — crates/vn-video (CONTRACT.md §6)
// ---------------------------------------------------------------------------

/** Live media state as `name=value` lines; `""` for a node that is not one. */
declare function __vn_media_state(id: VnNodeId): VnKvLines;

/** `HTMLMediaElement`'s verbs, by code. Mirrors `vn_tree::video::media_call`. */
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

/** `canPlayType(type)` — `"" | "maybe" | "probably"`, probed on THIS machine. */
declare function __vn_media_can_play(id: VnNodeId, mime: string): "" | "maybe" | "probably";

/**
 * Drains the node's recent media events, comma-joined. A bridge and a TEST
 * surface — the events themselves ride the ordinary EV_* channel, so nothing
 * in `runtime.ts` calls this; `crates/vn-video`'s own tests do.
 */
declare function __vn_media_events(id: VnNodeId): string;

// ---------------------------------------------------------------------------
// multi-window — crates/vn-app/src/popout.rs (docs/MULTIWINDOW.md)
// ---------------------------------------------------------------------------
//
// These move a subtree's PAINT into a second OS window while the node stays in
// this tree, this component instance and this reactive graph — so `nodeId` is a
// `VnNodeId`, i.e. `mirrorNode.id` off a template ref (MULTIWINDOW.md §3.2
// invariant 1). **`<VnWindow>` (src/vnwindow.ts) is the app-facing spelling and
// the only thing that should call these**; they are written down here because
// they are the contract between the component and the host.
//
// Installed by `vn-app`, not by the engine — same category as `__vn_storage_*`,
// so a host without them simply has no such global and a caller that cannot
// assume vn-app must probe with `typeof __vn_window_open === "function"`.

/**
 * Moves the node's PAINT into a new OS window.
 *
 * `options` is JSON — `<VnWindow>`'s props, verbatim:
 * `{ title?, x?, y?, width?, height?, chrome?, backdrop?, alwaysOnTop?,
 *    resizable?, minWidth?, minHeight?, titleBarHeight?, activate? }`.
 *
 * `onClose` fires with the window, however it closed — the X button,
 * `__vn_window_close`, or the primary window going away. `onEvent` is the push
 * channel for everything else the window reports: `("open", '{"windowId":N}')`,
 * `("focus", "{}")`, `("blur", "{}")`,
 * `("resize", '{"width":W,"height":H,"scale":S}')`. It is how a pop-out's OWN
 * metrics reach JS at all: there is one JS realm, so `window.innerWidth` keeps
 * meaning the primary window's viewport (A-M04, §7.7).
 *
 * Returns `undefined`, and the open is DEFERRED by one effect cycle:
 * `App::open_window` draws the new window once before returning and this is
 * called from inside the primary window's update, so it cannot be nested there
 * (MULTIWINDOW.md §3.2 invariant 2). UN-painting, though, is immediate — the
 * primary window's next draw is already without the subtree, because a node in
 * two element trees for one frame aliases scroll state and hitboxes.
 */
declare function __vn_window_open(
  nodeId: VnNodeId,
  options: VnJson,
  onClose?: () => void,
  onEvent?: (kind: string, detailJson: VnJson) => void,
): void;

/**
 * The LIVE half of `<VnWindow>`'s props: `title` renames the window and
 * `alwaysOnTop` re-stacks it. Everything else gpui fixes at creation, so a
 * changed value ADVISES and does nothing — never a silent no-op
 * (docs/WINDOWING.md D-25/D-30).
 */
declare function __vn_window_set_options(nodeId: VnNodeId, options: VnJson): void;

/** Closes the window painting `nodeId` — the same path as the user clicking X. */
declare function __vn_window_close(nodeId: VnNodeId): void;

/**
 * `[{ node, windowId, title, chrome, width, height, focused }]` as JSON. For
 * tests and devtools; the devtools `windows` command is the richer surface.
 */
declare function __vn_window_popouts(): VnJson;

/**
 * TEST SURFACE. Dispatches a real platform click into the window painting
 * `nodeId`, at that window's own client coordinates; `true` if it landed.
 */
declare function __vn_window_click(nodeId: VnNodeId, x: number, y: number): boolean;

// ---------------------------------------------------------------------------
// host logging — crates/vn-app/src/main.rs
// ---------------------------------------------------------------------------

/**
 * A line into the host log.
 *
 * `vn-app` registers it UNCONDITIONALLY (`main.rs`, next to `__vn_set_status`
 * and `__vn_perf_mark`) and `runtime.ts` calls it unguarded, which is why this
 * is a `function` rather than an optional `var` — an app does not have to
 * probe. An embedder that installed `vn-tree` without `vn-app` would have to
 * provide it; the unit-test harness does, by binding it as a parameter.
 */
declare function __vn_log(message: string): void;

// ---------------------------------------------------------------------------
// devtools discovery — crates/vn-app/src/devtools/mod.rs (docs/DEVTOOLS-UI.md)
// ---------------------------------------------------------------------------

/**
 * Every attachable vue-native instance, as JSON:
 * `{ preferred, self: { pid, dir }, targets: [{ slot, dir, port, pid, token,
 * alive, self }] }`.
 *
 * The inspector app's ONE host dependency. Everything else it does goes over
 * the ordinary devtools wire, which redjs's `fetch` can speak by itself — but
 * discovery has to read a handshake file off disk, and `fetch` cannot.
 *
 * Registered by `devtools::install`, i.e. in every host EXCEPT one built with
 * `--features no-devtools`, so a caller must probe with
 * `typeof __vn_devtools_targets === "function"` and say what the absence means.
 * `apps/devtools/src/wire.ts` is the one caller.
 */
declare function __vn_devtools_targets(): VnJson;

// ---------------------------------------------------------------------------
// the devtools switch — crates/vn-app/src/devtools/mod.rs (docs/DEVTOOLS.md)
// ---------------------------------------------------------------------------
//
// The `appDevtools` extension's three host fns (tier 3, docs/RUNTIME.md). Same
// availability as `__vn_devtools_targets` above and the same probe: present in
// every build except `--features no-devtools`, where `appDevtools` goes inert
// and says so.

/**
 * `{ enabled, locked, port, pid, key, inspectors, slot }`.
 *
 * `enabled` is the master switch — off means there is NO listener, not merely
 * that F12 is refused. `locked` is `VN_DEVTOOLS=1` in the environment, which
 * refuses `disable()`. `key` is the separate `VN_DEVTOOLS_KEY` gate, which
 * declines F12 with the protocol left alone.
 */
declare function __vn_devtools_state(): VnJson;

/**
 * Turns the devtools on or off, and returns the SETTLED state (the same object
 * `__vn_devtools_state` returns) or `{ error }` when the change was refused.
 *
 * On → binds a fresh port with a fresh token and republishes
 * `<dist>/devtools.json`. Off → closes the listener, drops every push socket,
 * blanks the token and deletes the handshake.
 */
declare function __vn_devtools_set_enabled(on: boolean): VnJson;

/** Programmatic F12: `{ opened, window }` or `{ error }`. */
declare function __vn_devtools_open(): VnJson;

/**
 * How THIS process was started, when it was started as an inspector
 * (`vn-app --devtools`, which is what F12 spawns —
 * `crates/vn-app/src/inspector.rs`).
 *
 * `window` is the one fact no amount of scanning can recover: a target
 * process can paint several OS windows off one retained tree
 * (docs/MULTIWINDOW.md), and the key that opened this inspector was pressed
 * in exactly one of them.
 *
 * Present for every inspector — `bun devtools/ui.ts --target <slot>` execs the
 * same `vn-app --devtools` line F12 spawns. Still declared optional: it is a
 * JSON global published by another process, so every reader guards.
 */
declare var __VN_DEVTOOLS_ARGS__:
  | {
      /** The target's slot directory. */
      target: string | null;
      /** The gpui window id F12 was pressed in. */
      window: number | null;
      /** The pid this inspector dies with. */
      parent: number | null;
      /** Always `true` — the bundle came out of the executable. */
      embedded: boolean;
    }
  | undefined;

// ---------------------------------------------------------------------------
// permissions — crates/vn-permissions/src/lib.rs (docs/PERMISSIONS.md tier 2)
// ---------------------------------------------------------------------------

/** The four states. `"unsupported"` is the one the web has no word for. */
type VnPermissionState = "granted" | "denied" | "prompt" | "unsupported";
/** The W3C names `Capability::ALL` knows. */
type VnPermissionName =
  | "microphone"
  | "camera"
  | "display-capture"
  | "notifications"
  | "clipboard-read"
  | "clipboard-write"
  | "geolocation";

/** `"Windows"` / `"macOS"` — the name an advisory would use. */
declare function __vn_permissions_platform(): string;
/** The four-state read, or `""` for a name that is not a permission. */
declare function __vn_permissions_query(name: string): VnPermissionState | "";
/** `[{ name, state, canRequest }]`, as JSON. */
declare function __vn_permissions_list(): VnJson;
/**
 * Starts a request. The JSON answer is one of three shapes:
 * `{ "error": … }` (not a permission name), `{ "state": … }` (settled
 * synchronously — nothing to ask), or `{ "token": n }` (the per-frame pump will
 * deliver the result to the dispatcher under that token).
 */
declare function __vn_permissions_request(name: string): VnJson;
/** Installs the callback the pump settles pending requests through. */
declare function __vn_permissions_set_dispatcher(
  fn: (token: number, state: VnPermissionState) => void,
): void;

// ---------------------------------------------------------------------------
// storage — crates/vn-storage/src/lib.rs (docs/STORAGE.md tier 2)
// ---------------------------------------------------------------------------

/** `{ path }` or `{ error }`. Creates nothing — a path is not a directory. */
declare function __vn_storage_user_folder(kind: string, subfolder: string): VnJson;
/**
 * Points `localStorage` at `path`, creating it. `{ path }` or `{ error }`.
 * `"keep"`: the app did not say — the format stays (encryption is sticky).
 */
declare function __vn_storage_set_folder(path: string, encrypted: "true" | "false" | "keep"): VnJson;
/** `{ folder, file, encrypted, bytes, loaded, pending, provider, kinds, error }`. */
declare function __vn_storage_info(): VnJson;
/** Forces the coalesced write. `{ wrote: boolean }` or `{ error }`. */
declare function __vn_storage_flush(): VnJson;

// ---------------------------------------------------------------------------
// desktop — crates/vn-desktop/src/lib.rs (docs/SHELL.md, docs/LAUNCH.md)
// ---------------------------------------------------------------------------

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
 * true is the TIER-1 path (`<a href>`, `window.open`): http, https and mailto
 * only. False (`appShell.openExternal`) takes any scheme but `file:`.
 */
declare function __vn_shell_open_external(url: string, content: boolean): VnJson;
/** `{ token }` or `{ error, name, code? }`. Executables refused unless `allowExecutable`. */
declare function __vn_shell_open_path(path: string, allowExecutable: boolean): VnJson;
/** `{ token }` or `{ error, name }`. A folder is selected in its parent. */
declare function __vn_shell_show_item(path: string): VnJson;
/** `{ token }` or `{ error, name }`. Recycle Bin only, never a permanent delete. */
declare function __vn_shell_trash(path: string): VnJson;
/**
 * This process's launch, as `vn_desktop::launch::start` resolved it before the
 * window existed: `{ argv, cwd, url, paths, singleInstance, id, name }`.
 * Forwarded launches arrive later, as token-0 `{ kind: "launch", … }` events.
 */
declare function __vn_launch_info(): VnJson;
/**
 * Registers `scheme://` for this exe under HKCU. `{ ok, wrote, scheme }`, or a
 * refusal `{ error, name: "TypeError" }` (invalid, reserved, foreign-owned).
 */
declare function __vn_launch_register_protocol(scheme: string): VnJson;
/** Removes a registration carrying our marker AND our exe. Did it? */
declare function __vn_launch_unregister_protocol(scheme: string): boolean;
/** Ours, and pointing at this exe? */
declare function __vn_launch_is_protocol_registered(scheme: string): boolean;

// ---- File System Access and dialogs (vn-desktop fs/, dialog/) ---------------
//
// Every handle argument is a NUMBER into the host's handle table; no host fn
// takes or returns a path. `{ token }` answers settle through the dispatcher
// above; `{ error, name, code? }` is a synchronous refusal (the name fence, an
// unknown handle, a second dialog on one window).

/** `(id, name, kind) => handle` — what a dropped path's handle is made with. */
declare function __vn_fs_set_handle_factory(fn: (h: number, name: string, kind: string) => unknown): void;
/** The handle's JS object was collected. */
declare function __vn_fs_release(h: number): void;
/** Settles `{ name, size }` with the `File` as the value. */
declare function __vn_fs_get_file(h: number): VnJson;
/** Settles `{ h, name, kind }`. */
declare function __vn_fs_child(dir: number, name: string, isDirectory: boolean, create: boolean): VnJson;
declare function __vn_fs_remove(dir: number, name: string, recursive: boolean): VnJson;
/** Settles `{ same }`. */
declare function __vn_fs_same(a: number, b: number): VnJson;
/** Settles `{ path: string[] | null }`. */
declare function __vn_fs_resolve(root: number, other: number): VnJson;
/** Settles `{ cursor }`. */
declare function __vn_fs_dir_open(dir: number): VnJson;
/** Settles `{ entries: {h, name, kind}[], done }`, 256 at most. */
declare function __vn_fs_dir_next(cursor: number): VnJson;
declare function __vn_fs_dir_close(cursor: number): void;
/** Settles `{ w }`. */
declare function __vn_fs_writable_open(h: number, keepExistingData: boolean): VnJson;
/** `position` -1 = at the cursor. `data`: a string, a Uint8Array or a Blob. */
declare function __vn_fs_writable_write(w: number, position: number, data: unknown): VnJson;
declare function __vn_fs_writable_seek(w: number, position: number): VnJson;
declare function __vn_fs_writable_truncate(w: number, size: number): VnJson;
declare function __vn_fs_writable_close(w: number): VnJson;
declare function __vn_fs_writable_abort(w: number): VnJson;
/** `structuredClone` of an instance is a DataCloneError naming `interfaceName`. */
declare function __vn_fs_refuse_clone(proto: object, interfaceName: string): void;
/**
 * A picker: `{ kind: "open"|"save"|"folder", multiple?, accept?, types?,
 * excludeAcceptAllOption?, id?, startIn?, suggestedName?, result: "files"|"handles" }`.
 * Settles `{ files }` + a FileList, `{ handles }`, or `{ cancelled: true }`.
 * `currentFiles` is an input's FileList (the same selection again is a cancel).
 */
declare function __vn_dialog_open(requestJson: VnJson, currentFiles: unknown): VnJson;
/** Save As for a `blob:`/`data:` URL, resolved at the call. Settles `{ saved }` or `{ cancelled }`. */
declare function __vn_download(url: string, downloadAttribute: string): VnJson;

// ---------------------------------------------------------------------------
// voice — crates/vn-voice/src/lib.rs (CONTRACT.md, docs/VOICE.md)
// ---------------------------------------------------------------------------

/** Starts an engine for one room. `0` means "could not start". */
declare function __vn_voice_create(optionsJson: VnJson): number;
/**
 * Posts one op to the room's actor and returns its request id. `0` is the
 * SYNCHRONOUS-REFUSAL sentinel — an unknown room id, or an engine whose channel
 * is closed — never a real request id, which is what lets the shim reject
 * immediately instead of leaving a promise pending forever.
 */
declare function __vn_voice_call(roomId: number, op: number, argJson: VnJson): number;
/** A live read of the actor's plain-data mirror. `""` for an unknown room. */
declare function __vn_voice_snapshot(roomId: number): VnJson;
/** Installs the one callback every room's results and events arrive through. */
declare function __vn_voice_set_dispatcher(fn: VnVoiceDispatcher): void;
/** Posts Shutdown and JOINS the thread, so no callback can still be in flight. */
declare function __vn_voice_destroy(roomId: number): void;

/**
 * `(roomId, kind, reqId, payload)`.
 *
 * `kind` 0 = a request RESOLVED (payload is its JSON result), 1 = a request
 * REJECTED (payload is the message); anything >= 16 is a room EVENT code from
 * `crates/vn-voice/src/proto.rs` and `reqId` is 0.
 */
type VnVoiceDispatcher = (
  roomId: number,
  kind: number,
  reqId: number,
  payload: string,
) => void;

// --- what crosses in the payload -------------------------------------------
//
// Read off `crates/vn-voice/src/engine.rs`'s `Obj::new().str(…)` builders, not
// off the shim: these are what the ACTOR emits, and the point of writing them
// down is that a disagreement between the two sides becomes a type error
// instead of a field that silently reads `undefined`.
//
// Every field is optional here even where engine.rs always emits it, because
// the shim guards each one with `!== undefined` and the unit harness sends
// partial payloads on purpose. Where the guarantee matters it is stated in the
// doc comment rather than in the `?`.

/** `kind_str` — engine.rs. Never "unknown" on the wire; the shim's own
 *  `Track.Kind.Unknown` is a local default, not something the actor sends. */
type VnVoiceTrackKind = "audio" | "video" | "unknown";
/** `source_str` — engine.rs. */
type VnVoiceTrackSource =
  | "camera"
  | "microphone"
  | "screen_share"
  | "screen_share_audio"
  | "unknown";
/** The shim's own projection; the actor never sends a stream state. */
type VnVoiceStreamState = "active" | "paused" | "unknown";
/** `state_str` — engine.rs. "signalReconnecting" is the shim's own enum member
 *  and is never emitted: the Rust SDK exposes three states (voice:signal-substates). */
type VnVoiceConnectionState =
  | "disconnected"
  | "connecting"
  | "connected"
  | "reconnecting"
  | "signalReconnecting";
/** `quality_str` — engine.rs emits only the first four. "unknown" is the shim's
 *  own default, and `""` (below) is what a default-constructed roster carries. */
type VnVoiceConnectionQuality = "excellent" | "good" | "poor" | "lost" | "unknown";

/** `TrackState::json()` — all eight keys are always present on the wire. */
interface VnVoiceTrackState {
  sid: string;
  name?: string;
  /** Present on the track itself, in addition to the event's flat `identity`. */
  ownerIdentity?: string;
  kind?: VnVoiceTrackKind;
  source?: VnVoiceTrackSource;
  muted?: boolean;
  /** Always `true` for a LOCAL publication, whatever the SDK thinks. */
  subscribed?: boolean;
  isLocal?: boolean;
}

/** `ParticipantState::json()` — all eleven keys are always present on the wire. */
interface VnVoiceParticipantState {
  identity?: string;
  sid?: string;
  name?: string;
  /** `""` when unset, never null. */
  metadata?: string;
  /** Always an object, possibly empty. A FRESH object every event — see the
   *  ParticipantAttributesChanged note in the conversion report. */
  attributes?: Record<string, string>;
  isLocal?: boolean;
  isSpeaking?: boolean;
  /** DERIVED by the actor: "no unmuted audio publication exists". */
  isMuted?: boolean;
  audioLevel?: number;
  /** `""` from a `ParticipantState::default()` — i.e. the local participant
   *  before connect, and every participant after `reset_roster`. */
  connectionQuality?: VnVoiceConnectionQuality | "";
  tracks?: VnVoiceTrackState[];
  /**
   * `undefined`, and that is the whole type. `ParticipantState::json()` has no
   * `permissions` key — the actor's mirror never built one — but the shim reads
   * `state.permissions` to fill livekit-client's `Participant.permissions`.
   * Typed as `undefined` rather than dropped so the read still compiles and
   * says out loud what it produces: nothing, for every participant, always.
   */
  permissions?: undefined;
}

/** The union of every room event's payload (dispatcher `kind >= 16`). One flat
 *  interface rather than a discriminated union because the discriminant is the
 *  dispatcher's separate `kind` argument, not a field. The comment on each
 *  field names the event code that sends it. */
interface VnVoiceEventData {
  /** 17 CONNECTION_STATE. */
  state?: VnVoiceConnectionState;
  /** 18 CONNECTED — the room NAME. */
  room?: string;
  /** 18 CONNECTED. */
  localParticipant?: VnVoiceParticipantState;
  /** 22 PARTICIPANT_CONNECTED, 23 PARTICIPANT_DISCONNECTED, 25 PARTICIPANT_UPDATED. */
  participant?: VnVoiceParticipantState;
  /** 24 PARTICIPANT_ACTIVE, 26-34 track events, 36 PARTICIPANT_STATE,
   *  37 CONNECTION_QUALITY, 38 DATA_RECEIVED (`""` for an anonymous packet). */
  identity?: string;
  /** 19 DISCONNECTED. NOT one of the exported `DisconnectReason` values on
   *  every path — see the conversion report. */
  reason?: string;
  /** 39 CONNECT_FAILED. Nothing in the shim reads it. */
  error?: string;
  /** 36 PARTICIPANT_STATE. */
  isSpeaking?: boolean;
  /** 36 PARTICIPANT_STATE. */
  audioLevel?: number;
  /** 36 PARTICIPANT_STATE. */
  isMuted?: boolean;
  /** 36 PARTICIPANT_STATE. */
  connectionQuality?: VnVoiceConnectionQuality | "";
  /** 37 CONNECTION_QUALITY. */
  quality?: VnVoiceConnectionQuality;
  /** 35 ACTIVE_SPEAKERS. Unordered — a HashSet on the far side. */
  identities?: string[];
  /** 26-34, the track events. */
  track?: VnVoiceTrackState;
  /** 34 LOCAL_TRACK_REPUBLISHED, and only when the mic sid actually changed. */
  previousSid?: string;
  /** 38 DATA_RECEIVED — base64. */
  payload?: string;
  /** 38 DATA_RECEIVED. */
  reliable?: boolean;
  /** 38 DATA_RECEIVED — `""` when the packet carried none, never null. */
  topic?: string;
  /** 40 RTT. `-1` means unknown. */
  rttMs?: number;
}

/** `Roster::json()` — what `__vn_voice_snapshot` answers. All five keys are
 *  always present; `""` (not `"{}"`) comes back for an unknown room id. */
interface VnVoiceSnapshot {
  state?: VnVoiceConnectionState;
  /** The room NAME. `Room._seed` does not read it. */
  room?: string;
  rttMs?: number;
  localParticipant?: VnVoiceParticipantState;
  participants?: VnVoiceParticipantState[];
}

// ---------------------------------------------------------------------------
// the `__VN_*` globals — not host FUNCTIONS, but part of the same contract
// ---------------------------------------------------------------------------

/**
 * Every SFC's `<style>` block, collected at module-eval time by the builder and
 * joined into the first stylesheet by `mount()` (build.ts emits the pushes).
 */
declare var __VN_CSS__: string[] | undefined;

/**
 * Installed by `mount()`; called by the host before it re-evals a new bundle.
 * Unmounts the app and frees every node the mirror still holds.
 */
declare var __VN_TEARDOWN__: (() => void) | undefined;

/**
 * The windowing host (`crates/vn-app/src/window.prelude.js`, docs/WINDOWING.md
 * tier 3). Absent in the unit-test harness and in any embedder that does not
 * install the prelude — `runtime.ts` exports an inert `appWindow` then, and
 * says so once.
 */
declare var __VN_WINDOW__: VnWindowHost | undefined;

interface VnWindowHost {
  appWindow: VnAppWindow;
  defineWindow: (options: Record<string, unknown>) => void;
}

/** `{ x, y, width, height }`, in physical window coordinates. */
interface VnWindowRect {
  x: number;
  y: number;
  width: number;
  height: number;
}

/**
 * `appWindow.chrome` — the chrome-mode read, so an app can branch without a
 * media query. `vn_app::window` builds it field by field, so the shape is fixed
 * and small; it was a bag of `unknown` until an app tried to use it.
 *
 * `client` is the window's CLIENT AREA, and `contentInset` is how far the app's
 * viewport is inset inside it by host-drawn chrome — `inner* === client* -
 * inset` by construction, and in custom-chrome mode every inset is zero.
 */
interface VnWindowChrome {
  titleBarStyle: string;
  displayMode: string;
  titleBarHeight: number;
  fallbackTitleBar: boolean;
  overlayVisible: boolean;
  /** Whether the HOST draws the min/max/close cluster (`titleBarOverlay`). */
  hostControls: boolean;
  client: { width: number; height: number };
  contentInset: { left: number; top: number; right: number; bottom: number };
}

/**
 * The window facade the prelude builds. An `EventTarget` first — apps listen
 * for `resize`/`move`/`focus` on it — with the Tauri/Electron-shaped verbs
 * bolted on. Every verb is SYNCHRONOUS and returns `undefined`: JS and the UI
 * thread are the same thread here, so a promise would lie about ordering
 * (`await undefined` is legal, so ported `await appWindow.maximize()` still
 * runs).
 */
interface VnAppWindow extends VnEventTarget {
  minimize(): void;
  maximize(): void;
  unmaximize(): void;
  toggleMaximize(): void;
  restore(): void;
  close(): void;
  center(): void;
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
  alwaysOnTop: boolean;
  readonly scaleFactor: number;
  readonly chrome: VnWindowChrome;
  /** Windows taskbar integration — present, advisory-only in v1. */
  setProgressBar(...args: unknown[]): void;
  setOverlayIcon(...args: unknown[]): void;
  setBadgeCount(...args: unknown[]): void;
  setSkipTaskbar(...args: unknown[]): void;
  setIcon(...args: unknown[]): void;
}

/**
 * Sticky activation (docs/WINDOWING.md §1.9). The prelude arms it from the
 * engine's own EventTarget, which gpui events never reach — so the host's
 * dispatcher calls this instead, on `keydown`/`mousedown`. Without it
 * `beforeunload` could never veto a close, because the window would never count
 * as having been interacted with.
 */
declare var __vnStickyActivation: (() => void) | undefined;

/** Vue's dev-only HMR runtime, which `loader.ts` drives. */
declare var __VUE_HMR_RUNTIME__: VnVueHmrRuntime | undefined;

interface VnVueHmrRuntime {
  createRecord(id: string, def: unknown): boolean;
  rerender(id: string, render: unknown): void;
  reload(id: string, def: unknown): void;
}
