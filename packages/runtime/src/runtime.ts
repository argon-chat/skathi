// Skathi JS runtime: a @vue/runtime-core custom renderer over the DOM surface (dom.ts).
// Apps import `createApp` from "@skathi-native/runtime"; compiled SFCs import their helpers from "vue".

import {
  Fragment,
  Teleport,
  createRenderer,
  defineComponent,
  isRef,
  setDevtoolsHook,
  shallowRef,
  toRaw,
  version,
} from "@vue/runtime-core";
import type { App, ComponentPublicInstance } from "@vue/runtime-core";
export * from "./dom";
import { warnCompat } from "./compat";
import { ensureDocumentRoot, teardownDocumentRoot } from "./runtime/root";
import type { MirrorElement } from "./runtime/types";
import { nodeOps, patchTeleport } from "./runtime/node-ops";
import { patchProp } from "./runtime/patch-prop";
import { isMirrorNode } from "./runtime/listeners";
import { installStylesheet } from "./runtime/stylesheet";
import { watchVueRenders } from "./runtime/perf-advice";
import { trackWindowReads } from "./runtime/window";

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
export { Transition, TransitionGroup, resolveTransitionProps } from "./runtime/transitions";
export { SkWindow } from "./skwindow-vue";

// `appWindow`'s state reads (`isMaximized()`, `title`, `chrome`, …) re-run the effect, computed or render that made them.
const windowState = shallowRef(0);
trackWindowReads(
  "vue",
  () => void windowState.value,
  () => void windowState.value++,
);

const makeRenderer = () => createRenderer({ ...nodeOps, patchProp });
let renderer: ReturnType<typeof makeRenderer> | null = null;

/** Vue's renderer, made on first use: an app that renders through the DOM alone does not carry it. */
function vueRenderer(): ReturnType<typeof makeRenderer> {
  if (!renderer) {
    patchTeleport(Teleport);
    renderer = makeRenderer();
    if (__DEV__) watchVueRenders({ Fragment, isRef, setDevtoolsHook, toRaw });
  }
  return renderer;
}

/**
 * What `createApp` hands back: Vue's `App`, except `mount()` without a container keeps the native window-root shorthand;
 * a selector or Element mounts there. It extends `App` rather than `Omit`-ing `mount`, since a mapped type resolves
 * `this` to the source type and breaks `createApp(App).use(router).mount()`.
 */
export interface SkApp extends App<MirrorElement> {
  mount(container?: unknown): ComponentPublicInstance;
}

export function createApp(rootComponent: unknown, rootProps: Record<string, unknown> | null = null): SkApp {
  const app = vueRenderer().createApp(rootComponent as never, rootProps);
  const mount = app.mount.bind(app);
  app.mount = (container?: unknown) => {
    if (globalThis.__SK_CSS__?.length) {
      installStylesheet(globalThis.__SK_CSS__.join("\n"));
    }
    const root = ensureDocumentRoot();
    let target: MirrorElement = root;
    let explicitContainer = container != null;
    if (typeof container === "string") {
      // Root-only native entries use '#app' without an index.html container: prefer a real element, else keep that alias.
      const resolved = (
        globalThis.document as unknown as { querySelector?: (selector: string) => MirrorElement | null } | undefined
      )?.querySelector?.(container);
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
    globalThis.__SK_TEARDOWN__ = () => teardownDocumentRoot(root, () => app.unmount());
    __sk_log(`Vue ${version} app mounted (Skathi runtime)`);
    return vm;
  };
  return app as SkApp;
}

// runtime-dom's other exports: a missing name breaks the build of every app using it (`useCssVars` is what `v-bind()`
// in `<style>` compiles to). Each exists: the real thing where possible, an advisory where not.

export const render: ReturnType<typeof makeRenderer>["render"] = (vnode, container, namespace) =>
  vueRenderer().render(vnode, container, namespace);

export { nodeOps, patchProp };

function notSupported(msg: string): Error {
  return typeof DOMException === "function"
    ? new DOMException(msg, "NotSupportedError")
    : Object.assign(new Error(msg), { name: "NotSupportedError" });
}
const NO_CUSTOM_ELEMENTS =
  "Skathi has no custom elements (no customElements registry, no shadow DOM) — " +
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
export function createSSRApp(rootComponent: unknown, rootProps: Record<string, unknown> | null = null): SkApp {
  warnCompat(
    "createSSRApp",
    "createSSRApp() — a Skathi window has no server-rendered markup to hydrate; it behaves as createApp()",
  );
  return createApp(rootComponent, rootProps);
}
export const hydrate: typeof render = (vnode, container) => {
  warnCompat("hydrate", "hydrate() — nothing to hydrate in a Skathi window; it renders from scratch, as render() does");
  render(vnode, container);
};
export const initDirectivesForSSR = (): void => {};

// Compiled SFC code and app code import everything Vue-ish from here
// ("vue" / "@skathi-native/runtime" are aliased to this module). Local exports win over
// the star re-export, so our createApp shadows runtime-core's.
export * from "@vue/runtime-core";
