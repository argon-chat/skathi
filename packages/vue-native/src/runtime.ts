// vue-native JS runtime: a @vue/runtime-core custom renderer over the __vn_* host protocol.
// Apps import `createApp` from "vue-native"; compiled SFCs import their helpers from "vue".

import { createRenderer, defineComponent, version } from "@vue/runtime-core";
import type { App, ComponentPublicInstance } from "@vue/runtime-core";
import "./apphost";
import { warnCompat } from "./compat";
import type { MirrorElement } from "./runtime/types";
import { hooks } from "./runtime/state";
import { looseById, looseRoots, nodesById } from "./runtime/nodes";
import { mk, nodeOps } from "./runtime/node-ops";
import { patchProp } from "./runtime/patch-prop";
import {
  disposeGlobalListener,
  dndRearm,
  dndState,
  globalHandlers,
  isMirrorNode,
  syncListenerFlag,
} from "./runtime/listeners";
import { pointer, pointerCaptures, releaseCapturedPointer } from "./runtime/pointer-capture";
import { observers } from "./runtime/mutation-observer";
import { installStylesheet } from "./runtime/stylesheet";
import "./runtime/reflect";
import "./runtime/dnd";
import "./runtime/document";

export { appCapture, appLaunch, appPermissions, appShell, appStorage } from "./appapi";
export type { AppDesktopError, CapturePermission, CaptureSource, ScreenCaptureSource, VnLaunchEvent } from "./appapi";
export { useCssModule, useCssVars } from "./runtime/css-vars";
export {
  vModelCheckbox,
  vModelDynamic,
  vModelRadio,
  vModelSelect,
  vModelText,
  vShow,
  withKeys,
  withModifiers,
} from "./runtime/directives";
export { appWindow, defineWindow, getCurrentWindow } from "./runtime/window";
export { appDevtools } from "./runtime/devtools";
export type { DevtoolsInfo } from "./runtime/devtools";
export { Transition, TransitionGroup, resolveTransitionProps } from "./runtime/transitions";

export let appRoot: MirrorElement | null = null; // created on mount or a pre-mount body insertion

const renderer = createRenderer({ ...nodeOps, patchProp });

function teardownDocumentRoot(root: MirrorElement, unmount?: () => void): void {
  if (!root.__vnId) return;
  try {
    unmount?.();
  } catch (e) {
    __vn_log("teardown: " + e);
  }
  pointer.buttons = 0;
  for (const pointerId of Array.from(pointerCaptures.keys())) releaseCapturedPointer(pointerId);
  dndState.gesture = null;
  dndState.seen = false;
  dndState.swallowClick = false;
  __vn_remove(root.__vnId);
  // <KeepAlive> creates its cache container with createElement and never inserts or removes it; what is left in the
  // mirror after app.unmount() is such an orphan, freed here or the host arena grows by one node per KeepAlive on each
  // hot restart.
  for (const id of Array.from(nodesById.keys())) __vn_remove(id);
  // …and every loose subtree, held by script or not: the next app's nodes reuse these ids.
  for (const [id, r] of looseRoots) __vn_remove(r.box || id);
  for (const n of nodesById.values()) n.__vnId = 0;
  for (const ref of looseById.values()) {
    const n = ref.deref();
    if (n) n.__vnId = 0;
  }
  root.__vnId = 0;
  nodesById.clear();
  looseById.clear();
  looseRoots.clear();
  globalThis.__VN_CSS__ = [];
  globalThis.__VN_TEARDOWN__ = undefined;
  appRoot = null;
  observers.clear();
  __vn_style_observe(false);
  for (const map of globalHandlers.values()) {
    for (const buckets of Object.values(map)) {
      if (buckets) for (const entry of [...buckets.extraC, ...buckets.extraB]) disposeGlobalListener(entry);
    }
  }
  globalHandlers.clear();
}

export function ensureDocumentRoot(): MirrorElement {
  if (appRoot) return appRoot;
  // Named `html`, not `ROOT`: this root answers to `document.documentElement` and `document.body`, and DOM-walking
  // libraries stop at `html`: under any other name @floating-ui's `getOverflowAncestors` loops forever. The Rust tag
  // stays "root".
  const root = mk(__vn_create_root(), 1, "html");
  appRoot = root;
  globalThis.__VN_TEARDOWN__ = () => teardownDocumentRoot(root);
  // Listeners added to `document`/`window` at module scope had no root to carry their bit; sync them now.
  for (const m of globalHandlers.values()) for (const name of Object.keys(m)) syncListenerFlag(root, name);
  dndRearm();
  // Module-scope writes to `document.documentElement` (the `dark` class, `--radius`) land before first render.
  hooks.flushDocumentElement?.(root);
  return root;
}

/**
 * What `createApp` hands back: Vue's `App`, except `mount()` without a container keeps the native window-root shorthand;
 * a selector or Element mounts there. It extends `App` rather than `Omit`-ing `mount`, since a mapped type resolves
 * `this` to the source type and breaks `createApp(App).use(router).mount()`.
 */
export interface VnApp extends App<MirrorElement> {
  mount(container?: unknown): ComponentPublicInstance;
}

export function createApp(rootComponent: unknown, rootProps: Record<string, unknown> | null = null): VnApp {
  const app = renderer.createApp(rootComponent as never, rootProps);
  const mount = app.mount.bind(app);
  app.mount = (container?: unknown) => {
    if (globalThis.__VN_CSS__?.length) {
      installStylesheet(globalThis.__VN_CSS__.join("\n"));
    }
    const root = ensureDocumentRoot();
    let target: MirrorElement = root;
    let explicitContainer = container != null;
    if (typeof container === "string") {
      // Root-only native entries use '#app' without an index.html container: prefer a real element, else keep that alias.
      const resolved = (
        globalThis.document?.querySelector as ((selector: string) => MirrorElement | null) | undefined
      )?.(container);
      const legacyRoot = !resolved && container === "#app";
      if (legacyRoot) explicitContainer = false;
      const found = resolved ?? (legacyRoot ? root : null);
      if (!found) {
        warnCompat(
          "mount:container",
          `app.mount(${JSON.stringify(container)}) found no element; the app was not mounted`,
        );
        return undefined as never;
      }
      target = found as MirrorElement;
    } else if (container != null) {
      if (!isMirrorNode(container) || container.kind !== 1) {
        throw new TypeError("app.mount() expects an Element or a CSS selector");
      }
      target = container as MirrorElement;
    }
    if (explicitContainer) target.textContent = "";
    const vm = mount(target);
    if (explicitContainer) {
      target.removeAttribute("v-cloak");
      target.setAttribute("data-v-app", "");
    }
    globalThis.__VN_TEARDOWN__ = () => teardownDocumentRoot(root, () => app.unmount());
    __vn_log(`Vue ${version} app mounted (vue-native runtime)`);
    return vm;
  };
  return app as VnApp;
}

// runtime-dom's other exports: a missing name breaks the build of every app using it (`useCssVars` is what `v-bind()`
// in `<style>` compiles to). Each exists: the real thing where possible, an advisory where not.

export const render = renderer.render;

export { nodeOps, patchProp };

function notSupported(msg: string): Error {
  return typeof DOMException === "function"
    ? new DOMException(msg, "NotSupportedError")
    : Object.assign(new Error(msg), { name: "NotSupportedError" });
}
const NO_CUSTOM_ELEMENTS =
  "vue-native has no custom elements (no customElements registry, no shadow DOM) — " +
  "use the component directly instead of defining an element for it";

export class VueElement {
  constructor() {
    warnCompat("customElement", `VueElement — ${NO_CUSTOM_ELEMENTS}`);
    throw notSupported(`VueElement: ${NO_CUSTOM_ELEMENTS}`);
  }
}
export function defineCustomElement(options: unknown, extraOptions?: unknown): typeof VueElement {
  warnCompat("defineCustomElement", `defineCustomElement() — ${NO_CUSTOM_ELEMENTS}`);
  const def = defineComponent(options as never, extraOptions as never);
  return class VueCustomElement extends VueElement {
    static def = def;
  };
}
export const defineSSRCustomElement = (options: unknown, extraOptions?: unknown): typeof VueElement =>
  defineCustomElement(options, extraOptions);
export function useHost(): null {
  warnCompat("useHost", `useHost() — ${NO_CUSTOM_ELEMENTS}; it answers null`);
  return null;
}
export function useShadowRoot(): null {
  warnCompat("useShadowRoot", `useShadowRoot() — ${NO_CUSTOM_ELEMENTS}; it answers null`);
  return null;
}
export function createSSRApp(rootComponent: unknown, rootProps: Record<string, unknown> | null = null): VnApp {
  warnCompat(
    "createSSRApp",
    "createSSRApp() — a vue-native window has no server-rendered markup to hydrate; it behaves as createApp()",
  );
  return createApp(rootComponent, rootProps);
}
export const hydrate: typeof render = (vnode, container) => {
  warnCompat(
    "hydrate",
    "hydrate() — nothing to hydrate in a vue-native window; it renders from scratch, as render() does",
  );
  render(vnode, container);
};
export const initDirectivesForSSR = (): void => {};

// `<VnWindow>`: a second OS window painting part of this tree, addressed by `mirrorNode.__vnId`.
export { VnWindow, VN_WINDOW_CREATION_TIME_PROPS } from "./vnwindow";
export type { VnWindowBounds, VnWindowMetrics, VnWindowTarget } from "./vnwindow";

// `appHost`: functions and events of a program embedding vue-native as a library; inert in the exe.
export { appHost } from "./apphost";
export { appTray } from "./tray";
export type { AppTray, TrayOptions, TrayPixels, TrayMenuItem, TrayMenuEntry } from "./tray";
export { appHotkeys } from "./hotkeys";
export type { AppHotkeys, HotkeyBinding, HotkeyButton, HotkeyChord, HotkeyEvent, HotkeyStatus } from "./hotkeys";
export { appLocale } from "./locale";
export type { AppLocale, MessageArgs, Messages } from "./locale";
export type { AppHost, AppHostError } from "./apphost";

// Compiled SFC code and app code import everything Vue-ish from here
// ("vue" / "vue-native" are aliased to this module). Local exports win over
// the star re-export, so our createApp shadows runtime-core's.
export * from "@vue/runtime-core";
