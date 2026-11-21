// Ambient types for the host preludes (`prelude.ts`, `window.prelude.ts`), which are compiled into
// the binary as strings and so cannot import. tsconfig.json pulls in engine.d.ts (what redjs
// provides), protocol.d.ts (`SkAppWindow` and `__SK_WINDOW__`, shared with the runtime) and this
// file: the `__sk_window_*` host functions, the state snapshot, and the globals the prelude
// installs. `SkWindowHost` and `Document` merge with declarations in those files.

// `__sk_window_*` host functions, registered by window.rs with fixed arity. Four return JSON
// strings (state, match_media, hitboxes, cursor); the rest submit an op and return undefined.

declare function __sk_window_state(): string;

/**
 * Registers the single observer. `kinds` is a comma-separated change list, or `"sync"` /
 * `"close-requested"`; `json` is the new snapshot. Returning `false` vetoes the close.
 */
declare function __sk_window_observe(fn: (kinds: string, json: string) => boolean): void;

declare function __sk_window_install_controls(window: SkAppWindow): void;
declare function __sk_window_set_title(title: string): void;

/**
 * The arity-0 verbs, by name: `minimize`, `maximize`, `unmaximize`,
 * `toggleMaximize`, `close`, `center`, `show`, `hide`, `startDragging`. An unknown name is an
 * advisory on the Rust side, not a throw.
 */
declare function __sk_window_verb(verb: string): void;

declare function __sk_window_set_fullscreen(on: boolean, transition?: "system" | "fade"): void;

declare function __sk_window_activate(raise: boolean): void;

/**
 * `mask` selects which numbers apply (bit 0 x, 1 y, 2 width, 3 height); bit 4 targets the content
 * area instead of the window.
 */
declare function __sk_window_set_bounds(x: number, y: number, w: number, h: number, mask: number): void;

declare function __sk_window_set_always_on_top(on: boolean): void;
declare function __sk_window_set_backdrop(name: string): void;
declare function __sk_window_set_theme(name: string): void;

/**
 * `height` is `NaN` for "unchanged"; `mode` is -1 keep / 0 off / 1 on;
 * the two colours are CSS hex or `""` for the theme default.
 */
declare function __sk_window_set_overlay(height: number, mode: number, color: string, symbolColor: string): void;

declare function __sk_window_match_media(query: string): string;
declare function __sk_window_pressure_media_demand(demand: boolean): void;
declare function __sk_window_pressure_media_listener(query: MediaQueryList): boolean;

/**
 * `[{ node, area }]` as JSON: what the last painted frame of one window handed gpui. Absent
 * `windowId` is the primary window.
 */
declare function __sk_window_hitboxes(windowId?: number): string;

declare function __sk_window_cursor(): string;

declare function __sk_window_advise(message: string): void;

interface SkWindowState {
  innerWidth: number;
  innerHeight: number;
  outer: SkWindowRect;
  content: SkWindowRect;
  devicePixelRatio: number;
  maximized: boolean;
  minimized: boolean;
  visible: boolean;
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
  trafficLightPosition: { x: number; y: number } | null;
  /** The area left to APP CONTENT, not the titlebar's own box. */
  titleBarArea: SkWindowRect;
  fallbackTitleBar: boolean;
  hostControls: boolean;
  clientWidth: number;
  clientHeight: number;
  /** How far the app root sits inside the client area. `client - inset === inner`. */
  contentInset: { left: number; top: number; right: number; bottom: number };
  /** `sk.config.json`'s `window.focus`, or `SK_WINDOW_FOCUS`. */
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

interface SkMatchMediaResult {
  matches: boolean;
  pressure?: boolean;
  error?: string;
}

interface SkWindowHitbox {
  node: number;
  area: "drag" | "no-drag" | "minimize" | "maximize" | "close";
}

type SkWindowCursorPos = { x: number; y: number } | null;

/**
 * protocol.d.ts declares only the members the runtime re-exports (`appWindow`, `defineWindow`);
 * the rest is the prelude's own debug surface, merged here so importing it from "@skathi-native/runtime"
 * still fails to type-check.
 */
interface SkWindowHost {
  getCurrentWindow(): SkAppWindow;
  refresh(): SkWindowState;
  warn(key: string, message: string): void;
  /** The last painted frame's hitboxes; WM_NCHITTEST cannot be synthesised. */
  controlHitboxes(): SkWindowHitbox[];
  cursorPos(): SkWindowCursorPos;
  state(): SkWindowState;
}

/** Members the prelude installs on redjs's `document`, and the ones it reads. */
interface Document {
  readonly documentElement: unknown;
  hasFocus(): boolean;
  exitFullscreen(): Promise<void>;
  dispatchEvent(ev: Event): boolean;
}

declare var innerWidth: number;
declare var innerHeight: number;

// Globals the prelude itself creates, so they cannot live in engine.d.ts. `var`, not `function`,
// because the prelude assigns them.

// Accessor-installed, read-only from JS; declared so `globalThis.screenX` type-checks.
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
declare var addEventListener: ((type: string, fn: (ev: Event) => void, options?: unknown) => void) | undefined;
declare function dispatchEvent(ev: Event): boolean;

/** What `emit()` needs of a target. Not `EventTarget`: the global's `addEventListener` is only probed. */
interface SkDispatchTarget {
  dispatchEvent(ev: Event): boolean;
}

interface ScreenOrientation extends EventTarget {
  readonly type: "landscape-primary" | "portrait-primary";
  readonly angle: number;
  lock(): Promise<never>;
  unlock(): void;
}
interface Screen extends EventTarget {
  readonly width: number;
  readonly height: number;
  readonly availWidth: number;
  readonly availHeight: number;
  readonly availLeft: number;
  readonly availTop: number;
  readonly colorDepth: number;
  readonly pixelDepth: number;
  readonly isExtended: boolean;
  readonly orientation: ScreenOrientation;
}
declare var screen: Screen;

/** The MediaQueryList the prelude returns. `__skReevaluate` is the observer's re-entry point, not web API. */
interface MediaQueryList extends EventTarget {
  readonly media: string;
  readonly matches: boolean;
  onchange: ((this: MediaQueryList, ev: Event) => void) | null;
  addListener(fn: (ev: Event) => void): void;
  removeListener(fn: (ev: Event) => void): void;
  __skReevaluate(): void;
}
declare var matchMedia: (query: unknown) => MediaQueryList;

/** `navigator.windowControlsOverlay`; the rect is the area for app content, so `x` is non-zero when controls sit on the left. */
interface SkTitlebarAreaRect extends SkWindowRect {
  top: number;
  left: number;
  right: number;
  bottom: number;
  toJSON(): SkWindowRect;
}
interface WindowControlsOverlay extends EventTarget {
  readonly visible: boolean;
  getTitlebarAreaRect(): SkTitlebarAreaRect;
  ongeometrychange: ((this: WindowControlsOverlay, ev: Event) => void) | null;
}

/**
 * The Fullscreen API's element half. redjs's `Element` prototype is empty, so `requestFullscreen`
 * is hung on it (hence optional); `dispatchEvent` is optional because the stack can hold whatever
 * an app passed to `requestFullscreen.call(…)`.
 */
interface SkFullscreenTarget {
  getBoundingClientRect?: () => { width: number; height: number };
  readonly __skId?: number;
  readonly isConnected?: boolean;
  readonly ownerDocument?: { readonly documentElement?: SkFullscreenTarget };
  requestFullscreen?: (options?: FullscreenOptions) => Promise<void>;
  dispatchEvent?: (ev: Event) => boolean;
}
interface FullscreenOptions {
  keyboardLock?: unknown;
  screen?: unknown;
}
/** `| undefined` and probed, like `HTMLElement` in engine.d.ts: a redjs revision may not ship it. */
declare var Element: { new (): SkFullscreenTarget; prototype: SkFullscreenTarget } | undefined;

/** Read off `globalThis` by Vue's esm-bundler build. Must precede the bundle. */
declare var __VUE_OPTIONS_API__: boolean;
declare var __VUE_PROD_DEVTOOLS__: boolean;
declare var __VUE_PROD_HYDRATION_MISMATCH_DETAILS__: boolean;

interface History {
  state: null;
  length: number;
  scrollRestoration: string;
  pushState(): void;
  replaceState(): void;
  go(): void;
  back(): void;
  forward(): void;
}
declare var history: History;
