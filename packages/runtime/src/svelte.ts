// `@skathi-native/runtime/svelte`: everything a Svelte 5 app imports from Skathi. It mounts the app on the window's
// document root, makes `appWindow`'s state reads reactive in runes, and has `<SkWindow>` as a Svelte component.

import {
  type Component,
  type ComponentType,
  mount as svelteMount,
  type MountOptions as SvelteMountOptions,
  type Snippet,
  type SvelteComponent,
  unmount as svelteUnmount,
} from "svelte";
import type { HTMLAttributes } from "svelte/elements";
import { createSubscriber } from "svelte/reactivity";
import {
  mountDocument,
  trackWindowReads,
  type SkWindowEvents,
  type SkWindowOptions,
  type SkWindowRect,
} from "@skathi-native/runtime";

export { default as SkWindow } from "./svelte/SkWindow.svelte";
export {
  appCapture,
  appDevtools,
  appHost,
  appHotkeys,
  appLaunch,
  appLocale,
  appPermissions,
  appShell,
  appStorage,
  appTray,
  appWindow,
  defineWindow,
  getCurrentWindow,
  SK_WINDOW_CREATION_TIME_PROPS,
} from "@skathi-native/runtime";
export type {
  AppDesktopError,
  AppHost,
  AppHostError,
  AppHotkeys,
  AppLocale,
  AppTray,
  CapturePermission,
  CaptureSource,
  DevtoolsInfo,
  HotkeyBinding,
  HotkeyButton,
  HotkeyChord,
  HotkeyEvent,
  HotkeyStatus,
  MessageArgs,
  Messages,
  ScreenCaptureSource,
  SkLaunchEvent,
  SkWindowBounds,
  SkWindowMetrics,
  SkWindowOptions,
  SkWindowRect,
  SkWindowShown,
  TrayMenuEntry,
  TrayMenuItem,
  TrayOptions,
  TrayPixels,
} from "@skathi-native/runtime";

/** `<SkWindow>`'s events as Svelte callback props: `onopen`, `onclose`, `onfocus`, `onblur`, `onresize`, `onshow`, `onhide`. */
export type SkWindowCallbacks = { [K in keyof SkWindowEvents as `on${K}`]?: SkWindowEvents[K] | undefined };

/** `<SkWindow>`'s props in Svelte. Other attributes, `class` and `style` land on the root `div.sk-window-root`. */
export interface SkWindowProps
  extends
    SkWindowOptions,
    SkWindowCallbacks,
    Omit<HTMLAttributes<HTMLElement>, "title" | "children" | keyof SkWindowCallbacks> {
  children?: Snippet | undefined;
  /** `bind:el`: the pop-out's root element, the target to mount or move content into. */
  el?: HTMLElement | null | undefined;
  /** `bind:windowId`: the OS window's id once open, 0 before and after. */
  windowId?: number | undefined;
  /** `bind:width`: the pop-out's own viewport width, CSS px. */
  width?: number | undefined;
  /** `bind:height`: the pop-out's own viewport height, CSS px. */
  height?: number | undefined;
  /** `bind:scale`: the pop-out's device pixel ratio. */
  scale?: number | undefined;
}

/** What `bind:this` on a `<SkWindow>` gives. */
export interface SkWindowExports {
  /** Where an element of the MAIN window is, in this window's CSS px relative to its root; null if either is unpainted. */
  mapFromMain(target: Element | null | undefined): SkWindowRect | null;
}

export type SkWindowComponent = Component<
  SkWindowProps,
  SkWindowExports,
  "el" | "windowId" | "width" | "height" | "scale"
>;

// `appWindow`'s state reads (`isMaximized()`, `title`, `chrome`, …) re-run the Svelte effect, `$derived` or template
// expression that made them, as they re-run Vue's.
const onWindowChange = globalThis.__SK_WINDOW__?.onChange;
if (onWindowChange) {
  const subscribe = createSubscriber((update) => onWindowChange(update));
  trackWindowReads("svelte", subscribe);
}

/** Svelte's mount options; `target` defaults to the document root. */
export type MountOptions<Props extends object> = Omit<SvelteMountOptions<Props>, "target"> & {
  target?: SvelteMountOptions<Props>["target"];
};

const mounted = new Set<object>();

/** Mounts `component` (on the document root unless `target` says otherwise) and returns its exports. */
export function mount<Props extends object, Exports extends object>(
  component: ComponentType<SvelteComponent<Props>> | Component<Props, Exports>,
  options?: MountOptions<Props>,
): Exports {
  let exports: Exports | undefined;
  mountDocument((root) => {
    const target = options?.target ?? root;
    // svelte-dom-shim: Svelte's DOM mount() renders into the mirror root through the emulated DOM; the custom renderer's mount replaces it (sveltejs/svelte#18042)
    exports = svelteMount(component, { ...options, target } as SvelteMountOptions<Props>) as Exports;
    const app = exports;
    mounted.add(app);
    return () => {
      if (mounted.delete(app)) void svelteUnmount(app);
    };
  });
  return exports!;
}

/** Unmounts what `mount` returned, playing outros when `outro` is set. */
export function unmount(exports: object, options?: { outro?: boolean }): Promise<void> {
  mounted.delete(exports);
  return svelteUnmount(exports as Record<string, unknown>, options);
}
