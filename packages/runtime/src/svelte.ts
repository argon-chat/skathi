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
import type nativeRenderer from "./svelte-renderer";
import {
  mountDocument,
  ensureDocumentRoot,
  trackWindowReads,
  type SkWindowEvents,
  type SkWindowOptions,
  type SkWindowRect,
} from "@skathi-native/runtime";

export { default as SkWindow } from "./svelte/SkWindow.svelte";
export { default as SkContextMenu } from "./svelte/SkContextMenu.svelte";
export { default as SkKbd } from "./svelte/SkKbd.svelte";
export { defineKeymap } from "@skathi-native/runtime";
export type {
  SkKeyMap,
  SkKeyMapEvent,
  SkKeyMapEventType,
  SkKeyMapHandler,
  SkKeyMapListenerOptions,
  SkKeyMapOptions,
} from "./keymap";
export type {
  SkKbdChord,
  SkKbdDeviceInput,
  SkKbdGamepadButton,
  SkKbdGamepadLayout,
  SkKbdKey,
  SkKbdKeys,
  SkKbdLabel,
  SkKbdLabels,
  SkKbdMouseButton,
  SkKbdPlatform,
  SkKbdProps,
  SkKbdSequence,
} from "./kbd";
export { default as SkMenuItem } from "./svelte/SkMenuItem.svelte";
export { default as SkMenuSeparator } from "./svelte/SkMenuSeparator.svelte";
export { default as SkSubmenu } from "./svelte/SkSubmenu.svelte";
export type { SkContextMenuHandle, SkMenuAnchor, SkMenuBackdrop } from "./context-menu";
export {
  appCapture,
  appDevtools,
  appHost,
  appFiles,
  appHaptics,
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
  AppHaptics,
  HapticOptions,
  HapticPattern,
  HapticTrigger,
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

/** `SkSurfaceEvent` in the app's types (env.d.ts), which this project does not load. */
type SurfaceEvent = Event & {
  readonly width: number;
  readonly height: number;
  readonly devicePixelRatio: number;
  readonly reason: "cleared" | "device";
};

/** `SkSurfaceDisplayEvent` in the app's types (env.d.ts). */
type SurfaceDisplayEvent = Event & {
  readonly headroom: number;
  readonly potentialHeadroom: number;
  readonly refreshRate: number | null;
  readonly sdrWhiteNits: number | null;
  readonly hdr: boolean;
  readonly outputHdr: boolean;
  readonly variableRefresh: boolean | null;
};

/** `SkLottieFrameEvent`, `SkLottieLoopEvent` and `SkLottieCompleteEvent` in the app's types (env.d.ts). */
type LottieEvent<T> = Event & { readonly currentTime: number; readonly totalTime: number } & T;

/** `SkEditorValue` in the app's types (env.d.ts). */
type EditorValue = {
  text: string;
  marks: { type: string; from: number; to: number; data?: unknown }[];
  atoms: { id: string; at: number; type: string; text?: string; data?: unknown }[];
  lines?: { line: number; type: string; data?: unknown }[];
};
/** `SkEditorChangeEvent` in the app's types (env.d.ts). */
type EditorChangeEvent = Event & { readonly changes: { from: number; to: number; insert: string }[] };

type Numberish = number | `${number}`;
type SizeHints = { width?: Numberish | undefined; height?: Numberish | undefined };

declare module "svelte/elements" {
  interface HTMLInputAttributes {
    /** `type="color"`: the value carries alpha, and the picker shows an alpha slider. */
    alpha?: boolean | undefined | null;
    /** `type="color"`: the value's color space. */
    colorspace?: "limited-srgb" | "display-p3" | undefined | null;
    /** Skathi, `type="color"`: an HDR value in linear light, with an intensity in stops in the picker. */
    hdr?: boolean | undefined | null;
    /** Before `type="color"` shows its picker; `preventDefault()` keeps it closed. */
    onshowpicker?: ((event: Event) => void) | undefined | null;
  }
  interface SvelteHTMLElements {
    /** Shows the frames native code presents to the surface `name` (Rust: `skathi::surface(name)`). */
    "sk-surface": HTMLAttributes<HTMLElement> &
      SizeHints & {
        name?: string | undefined;
        onsurfaceresize?: ((event: SurfaceEvent) => void) | undefined;
        onsurfaceready?: ((event: SurfaceEvent) => void) | undefined;
        onsurfacelost?: ((event: SurfaceEvent) => void) | undefined;
        onsurfacedisplaychange?: ((event: SurfaceDisplayEvent) => void) | undefined;
      };
    /** A transparent box the embedding program shows its native layer `name` under. */
    "sk-layer": HTMLAttributes<HTMLElement> & { name?: string | undefined };
    /** Plays a Lottie animation (`.json` or `.lottie`). */
    lottie: HTMLAttributes<HTMLElement> &
      SizeHints & {
        src?: string | undefined;
        autoplay?: Booleanish | undefined;
        /** Present: loop forever. A number `n`: play `n` more times. */
        loop?: Booleanish | Numberish | undefined;
        speed?: Numberish | undefined;
        direction?: 1 | -1 | "1" | "-1" | "forward" | "reverse" | undefined;
        mode?: "normal" | "bounce" | undefined;
        /** Two source frame numbers: `"30,59"`. */
        segment?: string | undefined;
        objectfit?: "fill" | "contain" | "cover" | "scale-down" | "none" | undefined;
        onenterframe?: ((event: LottieEvent<{ elapsed: number; progress: number }>) => void) | undefined;
        onloopcomplete?: ((event: LottieEvent<{ currentLoop: number }>) => void) | undefined;
        oncomplete?: ((event: LottieEvent<{ loops: number }>) => void) | undefined;
      };
    /** An animated sticker from Lottie (`.json`, `.tgs`) or WebM with alpha; copies of one source share a decode. */
    "sk-sticker": HTMLAttributes<HTMLElement> &
      SizeHints & {
        src?: string | undefined;
        play?: "auto" | "on" | "off" | undefined;
        /** `once` in Vue and HTML: Svelte takes any `on…` attribute for an event handler. */
        "play-once"?: Booleanish | undefined;
        tint?: Booleanish | undefined;
        "max-fps"?: Numberish | undefined;
        kind?: "auto" | "lottie" | "video" | "image" | undefined;
        onended?: ((event: Event) => void) | undefined;
      };
    /** Rich text: marks, atoms (children with `atom="<id>"`) and line blocks. `value` is set as a property. */
    "sk-editor": HTMLAttributes<HTMLElement> & {
      value?: EditorValue | undefined;
      placeholder?: string | undefined;
      readonly?: Booleanish | undefined;
      maxlength?: Numberish | undefined;
      onbeforeinput?: ((event: InputEvent) => void) | undefined;
      oninput?: ((event: InputEvent) => void) | undefined;
      onchange?: ((event: EditorChangeEvent) => void) | undefined;
      onselectionchange?: ((event: Event) => void) | undefined;
      onviewportchange?: ((event: Event) => void) | undefined;
    };
  }
}

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

const mounted: object[] = [];
let appMounted = false;

declare const __SK_SVELTE_NATIVE__: boolean;
declare function require(id: "@skathi-native/runtime/svelte-renderer"): { default: typeof nativeRenderer };

function unmountAll(): void {
  appMounted = false;
  for (const app of mounted.splice(0).reverse()) void svelteUnmount(app);
}

/**
 * Mounts `component` (on the document root unless `target` says otherwise) and returns its exports. The first mount is
 * the app; later ones, such as content mounted into a pop-out, are unmounted with it on reload.
 */
export function mount<Props extends object, Exports extends object>(
  component: ComponentType<SvelteComponent<Props>> | Component<Props, Exports>,
  options?: MountOptions<Props>,
): Exports {
  const run = (target: object): Exports => {
    // svelte-dom-shim: default DOM apps mount into Skathi's document mirror.
    const exports =
      typeof __SK_SVELTE_NATIVE__ !== "undefined" && __SK_SVELTE_NATIVE__
        ? svelteMount(component, {
            ...options,
            target: target === globalThis.document ? ensureDocumentRoot() : target,
            renderer: require("@skathi-native/runtime/svelte-renderer").default,
          } as SvelteMountOptions<Props, typeof nativeRenderer>)
        : svelteMount(component, { ...options, target } as SvelteMountOptions<Props>);
    mounted.push(exports);
    return exports;
  };
  if (appMounted && options?.target) return run(options.target);
  let exports: Exports | undefined;
  mountDocument((root) => {
    appMounted = true;
    exports = run(options?.target ?? root);
    return unmountAll;
  });
  return exports!;
}

/** Unmounts what `mount` returned, playing outros when `outro` is set. */
export function unmount(exports: object, options?: { outro?: boolean }): Promise<void> {
  const i = mounted.indexOf(exports);
  if (i >= 0) mounted.splice(i, 1);
  return svelteUnmount(exports as Record<string, unknown>, options);
}
