// Ambient types for the host preludes (`prelude.ts`, `window.prelude.ts`), which are compiled into
// the binary as strings and so cannot import. tsconfig.json pulls in engine.d.ts (what redjs
// provides), protocol.d.ts (`VnAppWindow` and `__VN_WINDOW__`, shared with the runtime) and this
// file: the `__vn_window_*` host functions, the state snapshot, and the globals the prelude
// installs. `VnWindowHost` and `VnDocument` merge with declarations in those files.

// `__vn_window_*` host functions, registered by window.rs with fixed arity. Four return JSON
// strings (state, match_media, hitboxes, cursor); the rest submit an op and return undefined.

/** The window as of this frame, as JSON. See `window.rs::snapshot_json`. */
declare function __vn_window_state(): string;

/**
 * Registers the single observer. `kinds` is a comma-separated change list, or `"sync"` /
 * `"close-requested"`; `json` is the new snapshot. Returning `false` vetoes the close.
 */
declare function __vn_window_observe(fn: (kinds: string, json: string) => boolean): void;

/** `SetWindowTextW`, with the raw string. */
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
 * `mask` selects which numbers apply (bit 0 x, 1 y, 2 width, 3 height); bit 4 targets the content
 * area instead of the window.
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
 * `[{ node, area }]` as JSON: what the last painted frame of one window handed gpui. Absent
 * `windowId` is the primary window.
 */
declare function __vn_window_hitboxes(windowId?: number): string;

/** `{ x, y }` in CSS px, or `null` when the platform has no cursor to report. */
declare function __vn_window_cursor(): string;

/** Publishes one compat advisory to the host's diagnostic channel. */
declare function __vn_window_advise(message: string): void;

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
  /** What the window draws of `backdrop`: `"none"` while macOS fullscreen has no desktop picture for the glass. */
  effectiveBackdrop: string;
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

/**
 * protocol.d.ts declares only the members the runtime re-exports (`appWindow`, `defineWindow`);
 * the rest is the prelude's own debug surface, merged here so importing it from "vue-native"
 * still fails to type-check.
 */
interface VnWindowHost {
  /** Always this window, in v1. The spelling ported Tauri code uses. */
  getCurrentWindow(): VnAppWindow;
  /** Re-reads the host NOW, bypassing the one-frame snapshot. */
  refresh(): VnWindowState;
  /** Publishes one advisory, at most once per key. */
  warn(key: string, message: string): void;
  /** The last painted frame's hitboxes; WM_NCHITTEST cannot be synthesised. */
  controlHitboxes(): VnWindowHitbox[];
  cursorPos(): VnWindowCursorPos;
  state(): VnWindowState;
}

/**
 * Members the prelude installs on redjs's `document`, plus `dispatchEvent`, which redjs has but
 * engine.d.ts never named. `documentElement` is not redeclared: a merged member must repeat its
 * type exactly.
 */
interface VnDocument {
  /** CSSOM View: `true` while this window has the OS focus. */
  hasFocus(): boolean;
  /** Pops the fullscreen stack; leaves fullscreen when it empties. */
  exitFullscreen(): Promise<void>;
  /** redjs's own. */
  dispatchEvent(ev: VnEvent): boolean;
}

// Globals the prelude itself creates, so they cannot live in engine.d.ts. `var`, not `function`,
// because the prelude assigns them.

/** Pushed by redjs on resize; the prelude only seeds them at boot. */
declare var innerWidth: number;
declare var innerHeight: number;
declare var devicePixelRatio: number;

// Accessor-installed, read-only from JS; declared so `globalThis.outerWidth` type-checks.
declare var outerWidth: number;
declare var outerHeight: number;
declare var screenX: number;
declare var screenY: number;
declare var screenLeft: number;
declare var screenTop: number;

/** Resizes the VIEWPORT; `moveTo` positions the WINDOW. */
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

/** `addEventListener` is optional because the prelude probes it with `?.`. */
declare var addEventListener:
  | ((type: string, fn: (ev: VnEvent) => void, options?: unknown) => void)
  | undefined;
declare function dispatchEvent(ev: VnEvent): boolean;

/** What `emit()` needs of a target. Not `VnEventTarget`: the global's `addEventListener` is only probed. */
interface VnDispatchTarget {
  dispatchEvent(ev: VnEvent): boolean;
}

/** `screen` and its `ScreenOrientation`. */
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

/** The MediaQueryList the prelude returns. `__vnReevaluate` is the observer's re-entry point, not web API. */
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

/** `navigator.windowControlsOverlay`; the rect is the area for app content, so `x` is non-zero when controls sit on the left. */
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
 * The Fullscreen API's element half. redjs's `Element` prototype is empty, so `requestFullscreen`
 * is hung on it (hence optional); `dispatchEvent` is optional because the stack can hold whatever
 * an app passed to `requestFullscreen.call(…)`.
 */
interface VnFullscreenTarget {
  getBoundingClientRect?: () => { width: number; height: number };
  readonly __vnId?: number;
  readonly isConnected?: boolean;
  readonly ownerDocument?: { readonly documentElement?: VnFullscreenTarget };
  requestFullscreen?: (options?: VnFullscreenOptions) => Promise<void>;
  dispatchEvent?: (ev: VnEvent) => boolean;
}
interface VnFullscreenOptions {
  /** Ignored — there is no browser UI here to lock. */
  keyboardLock?: unknown;
  /** Ignored: needs the Window Management API. */
  screen?: unknown;
}
/** `| undefined` and probed, like `HTMLElement` in engine.d.ts: a redjs revision may not ship it. */
declare var Element:
  | { new (): VnFullscreenTarget; prototype: VnFullscreenTarget }
  | undefined;

/** Read off `globalThis` by Vue's esm-bundler build. Must precede the bundle. */
declare var __VUE_OPTIONS_API__: boolean;
declare var __VUE_PROD_DEVTOOLS__: boolean;
declare var __VUE_PROD_HYDRATION_MISMATCH_DETAILS__: boolean;

/** The inert `history` stub: every method is a no-op, so it is not typed as `History`. */
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
