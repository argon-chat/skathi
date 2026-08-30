// shadcn-vue compatibility trial — entry point.
//
// See research/reports/m10/shadcn.md for what this app measures and how, and
// research/reports/m11/compat6.md for what closing the gaps changed.
// Everything under src/components/ui is UNMODIFIED upstream shadcn-vue v2.2.0
// (fetched by tools/fetch-registry.ts); everything else in src/ is the harness.
//
// There is no `compat/dom-shim.js` any more: the DOM surface reka-ui expects
// (closest/matches/querySelector, getBoundingClientRect, getComputedStyle,
// MutationObserver, DocumentFragment, the pointer events) is the RUNTIME'S now,
// backed by the real cascade matcher and gpui's real layout bounds — see
// packages/vue-native/src/runtime.ts.

import { createApp } from "vue-native";

import App from "./App.vue";

/**
 * What `createApp()` hands back HERE, in the one respect it differs from what
 * browser Vue hands back.
 *
 * `mount()` IS NULLARY. Browser Vue mounts into a container you pick out of a
 * document — `mount("#app")` — because a page can hold any number of roots.
 * There is no document to pick out of here and exactly one root, the window's
 * tree, so the runtime's own `mount()` takes nothing
 * (packages/vue-native/src/runtime.ts).
 *
 * tsconfig.json types "vue-native" as the real `vue` package, which is right
 * for every other symbol this app reaches through that name and wrong for this
 * single signature — so the divergence is asserted here, in the open, on the
 * one line it applies to, rather than buried in a module mapping that would
 * then mistype the other two thousand. The assertion is narrow enough to be
 * safe: `mount(…)` cannot be mistyped through a shape with no argument to
 * mistype.
 */
interface NativeApp {
  mount(): void;
}

(createApp(App) as unknown as NativeApp).mount();
