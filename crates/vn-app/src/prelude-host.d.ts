// The ambient surface the HOST preludes stand on — and nothing else.
//
// `prelude.ts` and `window.prelude.ts` are not part of the runtime bundle: the
// Rust host `include_str!`s their generated JavaScript and hands each string to
// `rt.eval` before the app bundle exists (crates/vn-app/src/lib.rs). So they
// have no imports to carry types in, and `tsconfig.json` next to this file is
// what puts a type on every free name they touch. It pulls in THREE things:
//
//   packages/vue-native/src/engine.d.ts    what redjs itself provides —
//                                          `console`, `Event`, `EventTarget`,
//                                          `document`, `navigator`. Reused
//                                          rather than restated, because a
//                                          second copy of "what the engine has"
//                                          is a second copy that can drift.
//   packages/vue-native/src/protocol.d.ts  `VnAppWindow` / `VnWindowHost` /
//                                          `VnWindowRect` / `__VN_WINDOW__` —
//                                          the surface `runtime.ts` re-exports
//                                          as `import { appWindow }`. The
//                                          prelude BUILDS what that file
//                                          CONSUMES, so both sides answering to
//                                          one declaration is the point: an
//                                          appWindow verb that changes shape
//                                          here stops compiling there.
//   this file                              the rest: the `__vn_window_*` host
//                                          functions, the shape of the state
//                                          snapshot they speak in, and the
//                                          tier-1 globals the prelude INSTALLS
//                                          (`screen`, `resizeTo`, `matchMedia`
//                                          …), which by definition cannot be
//                                          declared anywhere that describes the
//                                          engine as it already is.
//
// Two declarations here MERGE with `protocol.d.ts` rather than shadowing it —
// `VnWindowHost` and `VnDocument` — and each says below why the addition
// belongs on this side of the seam.
//
// Named `prelude-host.d.ts`, not `window.prelude.d.ts`: a `.d.ts` whose stem
// matches a `.ts` next to it reads as that file's EMITTED declarations, which
// this is the opposite of.

// ---------------------------------------------------------------------------
// crates/vn-app/src/window.rs::install — the `__vn_window_*` host functions
// ---------------------------------------------------------------------------
//
// Every one of these is registered by `window::install` with a fixed arity, and
// each is the only way the prelude can reach gpui. Two return JSON as a string
// (`__vn_window_state`, `__vn_window_match_media`, `__vn_window_hitboxes`,
// `__vn_window_cursor`); the rest submit an `Op` and answer `undefined`.

/** The window as of this frame, as JSON. See `window.rs::snapshot_json`. */
declare function __vn_window_state(): string;

/**
 * Registers THE observer — one callback, not a list. `kinds` is a
 * comma-separated change list (or the single word `"sync"` / `"close-requested"`);
 * `json` is the new snapshot. The boolean return is the `beforeunload` veto:
 * `false` means "do not close".
 */
declare function __vn_window_observe(fn: (kinds: string, json: string) => boolean): void;

/** `SetWindowTextW`. The RAW string, un-normalised — the taskbar shows it. */
declare function __vn_window_set_title(title: string): void;

/**
 * The arity-0 verbs, by name: `minimize`, `maximize`, `unmaximize`,
 * `toggleMaximize`, `close`, `center`, `startDragging`. An unknown name is an
 * advisory on the Rust side, not a throw.
 */
declare function __vn_window_verb(verb: string): void;

declare function __vn_window_set_fullscreen(on: boolean): void;

/** `true` raises the window; `false` requests attention (flashes the taskbar). */
declare function __vn_window_activate(raise: boolean): void;

/**
 * `mask` says which of the four numbers to honour — bit 0 x, 1 y, 2 width,
 * 3 height — and bit 4 switches the rect from the WINDOW to the CONTENT area.
 * An unset bit means "leave it alone", which is why the coordinates are passed
 * as `0` rather than omitted.
 */
declare function __vn_window_set_bounds(
  x: number,
  y: number,
  w: number,
  h: number,
  mask: number,
): void;

declare function __vn_window_set_always_on_top(on: boolean): void;
declare function __vn_window_set_backdrop(name: string): void;
/** `"system" | "light" | "dark"`; anything else is advised and ignored. */
declare function __vn_window_set_theme(name: string): void;

/**
 * `height` is `NaN` for "unchanged"; `mode` is -1 keep / 0 off / 1 on;
 * the two colours are CSS hex or `""` for the theme default.
 */
declare function __vn_window_set_overlay(
  height: number,
  mode: number,
  color: string,
  symbolColor: string,
): void;

/** `{ matches, error? }` as JSON — the cascade's own evaluator, same env. */
declare function __vn_window_match_media(query: string): string;

/**
 * `[{ node, area }]` as JSON — what the last painted frame of ONE window handed
 * gpui. Absent `windowId` = the primary window (docs/MULTIWINDOW.md §7.6: a
 * `<VnWindow chrome="hidden">` draws its own caption and has its own list).
 */
declare function __vn_window_hitboxes(windowId?: number): string;

/** `{ x, y }` in CSS px, or `null` when the platform has no cursor to report. */
declare function __vn_window_cursor(): string;

/** Publishes one compat advisory to the host's diagnostic channel. */
declare function __vn_window_advise(message: string): void;

// ---------------------------------------------------------------------------
// what those JSON answers deserialise to
// ---------------------------------------------------------------------------

/** `window.rs::snapshot_json`, field for field. Everything is CSS px. */
interface VnWindowState {
  innerWidth: number;
  innerHeight: number;
  /** The whole window, screen-relative. */
  outer: VnWindowRect;
  /** The content area, screen-relative. */
  content: VnWindowRect;
  devicePixelRatio: number;
  maximized: boolean;
  minimized: boolean;
  fullscreen: boolean;
  focused: boolean;
  cloaked: boolean;
  title: string;
  backdrop: string;
  /** `appWindow.theme`: `"system"`, `"light"` or `"dark"`. */
  theme: string;
  /** The window's dark mode as DWM has it, read back — `null` when it will not say. */
  systemDarkMode: boolean | null;
  /** What DWM believes, read back — `null` when it will not say. */
  systemBackdrop: number | null;
  alwaysOnTop: boolean;
  /** `"browser"` / `"standalone"` / `"fullscreen"` — the `display-mode` MQ. */
  displayMode: string;
  titleBarStyle: "default" | "hidden";
  titleBarOverlayVisible: boolean;
  /** The area left to APP CONTENT, not the titlebar's own box. */
  titleBarArea: VnWindowRect;
  fallbackTitleBar: boolean;
  /** Whether the HOST draws the min/max/close cluster. */
  hostControls: boolean;
  clientWidth: number;
  clientHeight: number;
  /** How far the app root sits inside the client area. `client - inset === inner`. */
  contentInset: { left: number; top: number; right: number; bottom: number };
  /** `vn.config.json`'s `window.focus`, or `VN_WINDOW_FOCUS`. */
  focusPolicy: "raise" | "attention";
  screen: {
    width: number;
    height: number;
    availWidth: number;
    availHeight: number;
    availLeft: number;
    availTop: number;
    isExtended: boolean;
  };
}

/** `__vn_window_match_media`. `error` is present only when the query failed. */
interface VnMatchMediaResult {
  matches: boolean;
  error?: string;
}

/** One `WindowControlArea` the last painted frame registered. */
interface VnWindowHitbox {
  node: number;
  area: "drag" | "no-drag" | "minimize" | "maximize" | "close";
}

/** `__vn_window_cursor` — `null` when the platform has no cursor. */
type VnWindowCursorPos = { x: number; y: number } | null;

// ---------------------------------------------------------------------------
// merged into `packages/vue-native/src/protocol.d.ts`
// ---------------------------------------------------------------------------

/**
 * `protocol.d.ts` declares the two members of `__VN_WINDOW__` that the RUNTIME
 * re-exports (`appWindow`, `defineWindow`) — deliberately, because that file
 * describes the contract from the consumer's side. The rest of the object is
 * the prelude's own debug and escape-hatch surface: `refresh()` for an exact
 * read, and the two eval-reachable hooks docs/WINDOWING.md §13 points a test
 * transcript at. Declaring them here, merged, keeps the consumer's view honest
 * (importing `controlHitboxes` from "vue-native" still does not type-check)
 * while letting the producer publish the whole literal without an excess
 * property error.
 */
interface VnWindowHost {
  /** Always this window, in v1. The spelling ported Tauri code uses. */
  getCurrentWindow(): VnAppWindow;
  /** Re-reads the host NOW, bypassing the one-frame snapshot. */
  refresh(): VnWindowState;
  /** Publishes one advisory, at most once per key. */
  warn(key: string, message: string): void;
  /** `WM_NCHITTEST` is a real OS message — a synthesised click cannot reach
   *  one, so this reports what the last painted frame actually registered. */
  controlHitboxes(): VnWindowHitbox[];
  cursorPos(): VnWindowCursorPos;
  state(): VnWindowState;
}

/**
 * The four members the PRELUDE installs on redjs's `document`, plus the one
 * (`dispatchEvent`) redjs already has but `engine.d.ts` never needed to name —
 * that file describes what `runtime.ts` may assume, and `runtime.ts` dispatches
 * at mirror nodes, never at the document.
 *
 * Merging rather than casting at each use site: without it every
 * `document.dispatchEvent(...)` here would resolve through `VnDocument`'s
 * `[key: string]: unknown` index signature and fail as "not callable", and the
 * cast that fixes it would have to be repeated six times.
 *
 * `documentElement` is deliberately NOT re-declared: `engine.d.ts` already
 * types it `unknown`, and a merged member must repeat its type exactly. Its one
 * assignment site casts instead, and says so.
 */
interface VnDocument {
  /** CSSOM View: `true` while this window has the OS focus. */
  hasFocus(): boolean;
  /** Pops the fullscreen stack; leaves fullscreen when it empties. */
  exitFullscreen(): Promise<void>;
  /** redjs's own. */
  dispatchEvent(ev: VnEvent): boolean;
}

// ---------------------------------------------------------------------------
// the tier-1 globals — docs/WINDOWING.md §1
// ---------------------------------------------------------------------------
//
// Everything below is a global the prelude ITSELF creates, so none of it can
// live in `engine.d.ts` (which is the inventory of what redjs ships before any
// of this runs). Declared `var`, never `function`, because the prelude assigns
// them: `declare function resizeTo()` would make `globalThis.resizeTo = …` an
// error, and the assignment is the whole point.

/** Pushed by redjs on resize; the prelude only seeds them at boot. */
declare var innerWidth: number;
declare var innerHeight: number;
declare var devicePixelRatio: number;

// Accessor-installed (`Object.defineProperty`), read-only from JS. The prelude
// never reads them itself; they are declared so that `globalThis.outerWidth` is
// a number to anything type-checked against this file rather than an error.
declare var outerWidth: number;
declare var outerHeight: number;
declare var screenX: number;
declare var screenY: number;
declare var screenLeft: number;
declare var screenTop: number;

/** Resizes the VIEWPORT; `moveTo` positions the WINDOW. The spec's asymmetry. */
declare var resizeTo: (width: number, height: number) => void;
declare var resizeBy: (dw: number, dh: number) => void;
declare var moveTo: (x: number, y: number) => void;
declare var moveBy: (dx: number, dy: number) => void;

declare var close: () => void;
/** Requests attention rather than raising, unless `window.focus: "raise"`. */
declare var focus: () => void;
/** Does nothing, loudly — as it does in every browser since it was removed. */
declare var blur: () => void;
/** Not supported (v1 is single-window); advises and returns `null`. */
declare var open: () => null;
/** Not implemented; advises and rejects with `NotSupportedError`. */
declare var getScreenDetails: () => Promise<never>;

/**
 * redjs dispatches the trusted `resize`/input events at the global, so these
 * two exist before the prelude runs — but only in the HOST. `addEventListener`
 * is optional because the prelude probes it with `?.` (`crates/vn-app` is not
 * the only embedder of this engine), and it is the wrapping of THIS function
 * that turns an `unload` listener into an advisory.
 */
declare var addEventListener:
  | ((type: string, fn: (ev: VnEvent) => void, options?: unknown) => void)
  | undefined;
declare function dispatchEvent(ev: VnEvent): boolean;

/**
 * What `emit()` needs of a target, and no more. Not `VnEventTarget`: the global
 * is one of the two things events are fired at here, and it is NOT one —
 * `addEventListener` on it is probed, not promised (above), so requiring the
 * full interface would reject `globalThis` for a member `emit` never touches.
 */
interface VnDispatchTarget {
  dispatchEvent(ev: VnEvent): boolean;
}

/** `screen`, and the `ScreenOrientation` hanging off it (CSSOM View §1.5). */
interface VnScreenOrientation extends VnEventTarget {
  readonly type: "landscape-primary" | "portrait-primary";
  readonly angle: number;
  /** Rejects with `NotSupportedError`, as it does in every desktop browser. */
  lock(): Promise<never>;
  unlock(): void;
}
interface VnScreen extends VnEventTarget {
  readonly width: number;
  readonly height: number;
  readonly availWidth: number;
  readonly availHeight: number;
  readonly availLeft: number;
  readonly availTop: number;
  /** 24, unconditionally — what the spec says to answer when unknown. */
  readonly colorDepth: number;
  readonly pixelDepth: number;
  readonly isExtended: boolean;
  readonly orientation: VnScreenOrientation;
}
declare var screen: VnScreen;

/**
 * The MQL the prelude hands back — bridged to the cascade's evaluator, so
 * `matchMedia("(max-width: 600px)")` and `@media (max-width: 600px)` can never
 * disagree. `__vnReevaluate` is the host observer's re-entry point; it is not
 * web API, which is why it carries the `__vn` prefix.
 */
interface VnMediaQueryList extends VnEventTarget {
  readonly media: string;
  readonly matches: boolean;
  onchange: ((this: VnMediaQueryList, ev: VnEvent) => void) | null;
  /** Deprecated in the spec, not removed — and still what shipped libraries call. */
  addListener(fn: (ev: VnEvent) => void): void;
  removeListener(fn: (ev: VnEvent) => void): void;
  __vnReevaluate(): void;
}
declare var matchMedia: (query: unknown) => VnMediaQueryList;

/**
 * `navigator.windowControlsOverlay`. The rect describes the area available to
 * APP CONTENT: on macOS the traffic lights sit LEFT, so `x` is non-zero there
 * and correct code never assumes a side.
 */
interface VnTitlebarAreaRect extends VnWindowRect {
  top: number;
  left: number;
  right: number;
  bottom: number;
  toJSON(): VnWindowRect;
}
interface VnWindowControlsOverlay extends VnEventTarget {
  readonly visible: boolean;
  getTitlebarAreaRect(): VnTitlebarAreaRect;
  ongeometrychange: ((this: VnWindowControlsOverlay, ev: VnEvent) => void) | null;
}

/**
 * The Fullscreen API's element half. redjs's `Element` has an EMPTY prototype
 * (see `engine.d.ts` on `HTMLElement`), so `requestFullscreen` is something the
 * prelude hangs on it rather than something it finds there — hence optional.
 * `dispatchEvent` is optional for the same reason `fireFs` probes for it: the
 * stack can hold whatever an app passed to `requestFullscreen.call(…)`.
 */
interface VnFullscreenTarget {
  requestFullscreen?: (options?: VnFullscreenOptions) => Promise<void>;
  dispatchEvent?: (ev: VnEvent) => boolean;
}
interface VnFullscreenOptions {
  /** Ignored — there is no browser UI here to lock. */
  keyboardLock?: unknown;
  /** Needs the Window Management API, which is single-window-gated here. */
  screen?: unknown;
}
/**
 * Declared `| undefined` and probed with `typeof Element === "function"`,
 * exactly as `engine.d.ts` declares `HTMLElement`: which DOM classes a redjs
 * revision ships is not a promise, and the probe is what keeps that honest.
 */
declare var Element:
  | { new (): VnFullscreenTarget; prototype: VnFullscreenTarget }
  | undefined;

// ---------------------------------------------------------------------------
// prelude.ts — the three Vue flags and the `history` stub
// ---------------------------------------------------------------------------

/** Read off `globalThis` by Vue's esm-bundler build. Must precede the bundle. */
declare var __VUE_OPTIONS_API__: boolean;
declare var __VUE_PROD_DEVTOOLS__: boolean;
declare var __VUE_PROD_HYDRATION_MISMATCH_DETAILS__: boolean;

/**
 * The inert `history` stub. redjs ships no `history`, and vue-router's dev
 * build touches it at startup even under memory history. Deliberately not the
 * real interface: every method is a no-op, and typing it as `History` would
 * promise a session stack that does not exist.
 */
interface VnHistoryStub {
  state: null;
  length: number;
  scrollRestoration: string;
  pushState(): void;
  replaceState(): void;
  go(): void;
  back(): void;
  forward(): void;
}
declare var history: VnHistoryStub;
