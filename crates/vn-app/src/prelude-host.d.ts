// Ambient types for the host preludes (`prelude.ts`, `window.prelude.ts`), which are compiled into
// the binary as strings and so cannot import. tsconfig.json pulls in engine.d.ts (what redjs
// provides), protocol.d.ts (`VnAppWindow` and `__VN_WINDOW__`, shared with the runtime) and this
// file: the `__vn_window_*` host functions, the state snapshot, and the globals the prelude
// installs. `VnWindowHost` and `VnDocument` merge with declarations in those files.

// `__vn_window_*` host functions, registered by window.rs with fixed arity. Four return JSON
// strings (state, match_media, hitboxes, cursor); the rest submit an op and return undefined.

declare function __vn_window_state(): string;

/**
 * Registers the single observer. `kinds` is a comma-separated change list, or `"sync"` /
 * `"close-requested"`; `json` is the new snapshot. Returning `false` vetoes the close.
 */
declare function __vn_window_observe(fn: (kinds: string, json: string) => boolean): void;

declare function __vn_window_set_title(title: string): void;

/**
 * The arity-0 verbs, by name: `minimize`, `maximize`, `unmaximize`,
 * `toggleMaximize`, `close`, `center`, `startDragging`. An unknown name is an
 * advisory on the Rust side, not a throw.
 */
declare function __vn_window_verb(verb: string): void;

declare function __vn_window_set_fullscreen(on: boolean): void;

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

declare function __vn_window_match_media(query: string): string;

/**
 * `[{ node, area }]` as JSON: what the last painted frame of one window handed gpui. Absent
 * `windowId` is the primary window.
 */
declare function __vn_window_hitboxes(windowId?: number): string;

declare function __vn_window_cursor(): string;

declare function __vn_window_advise(message: string): void;

interface VnWindowState {
  innerWidth: number;
  innerHeight: number;
  outer: VnWindowRect;
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
  theme: string;
  /** The window's dark mode as DWM has it, read back — `null` when it will not say. */
  systemDarkMode: boolean | null;
  systemBackdrop: number | null;
  alwaysOnTop: boolean;
  displayMode: string;
  titleBarStyle: "default" | "hidden";
  titleBarOverlayVisible: boolean;
  /** The area left to APP CONTENT, not the titlebar's own box. */
  titleBarArea: VnWindowRect;
  fallbackTitleBar: boolean;
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

interface VnMatchMediaResult {
  matches: boolean;
  error?: string;
}

interface VnWindowHitbox {
  node: number;
  area: "drag" | "no-drag" | "minimize" | "maximize" | "close";
}

type VnWindowCursorPos = { x: number; y: number } | null;

/**
 * protocol.d.ts declares only the members the runtime re-exports (`appWindow`, `defineWindow`);
 * the rest is the prelude's own debug surface, merged here so importing it from "vue-native"
 * still fails to type-check.
 */
interface VnWindowHost {
  getCurrentWindow(): VnAppWindow;
  refresh(): VnWindowState;
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
  hasFocus(): boolean;
  exitFullscreen(): Promise<void>;
  dispatchEvent(ev: VnEvent): boolean;
}

// Globals the prelude itself creates, so they cannot live in engine.d.ts. `var`, not `function`,
// because the prelude assigns them.

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

declare var resizeTo: (width: number, height: number) => void;
declare var resizeBy: (dw: number, dh: number) => void;
declare var moveTo: (x: number, y: number) => void;
declare var moveBy: (dx: number, dy: number) => void;

declare var close: () => void;
declare var focus: () => void;
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

interface VnScreenOrientation extends VnEventTarget {
  readonly type: "landscape-primary" | "portrait-primary";
  readonly angle: number;
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
  keyboardLock?: unknown;
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
