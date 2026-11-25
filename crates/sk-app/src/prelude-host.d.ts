// Ambient types for the host preludes (`prelude.ts`, `window.prelude.ts`), which are compiled into
// the binary as strings and so cannot import. tsconfig.json pulls in engine.d.ts (what redjs
// provides), protocol.d.ts (`SkAppWindow` and `__SK_WINDOW__`, shared with the runtime) and this
// file. `SkWindowHost` and `Document` merge with declarations in those files.

declare function __sk_window_advise(message: string): void;

/**
 * protocol.d.ts declares only the members the runtime re-exports (`appWindow`, `defineWindow`);
 * the rest is the host's own surface, merged here so importing it from "@skathi-native/runtime"
 * still fails to type-check.
 */
interface SkWindowHost {
  /** Re-reads the window now; geometry writes otherwise show a frame later. */
  refresh(): void;
}

/** Members the prelude installs on redjs's `document`, and the ones it reads. */
interface Document {
  readonly documentElement: unknown;
  exitFullscreen(): Promise<void>;
  dispatchEvent(ev: Event): boolean;
}

declare var innerWidth: number;
declare var innerHeight: number;

/** Installed by the runtime before any element can ask for fullscreen. */
interface MutationObserver {
  observe(target: unknown, options: object): void;
  disconnect(): void;
}
declare var MutationObserver: new (callback: () => void) => MutationObserver;

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
declare var Element: { new (): SkFullscreenTarget; prototype: SkFullscreenTarget };

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
