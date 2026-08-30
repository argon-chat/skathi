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
// packages/vue-native/src/runtime.js.

import { createApp } from "vue-native";

import App from "./App.vue";

createApp(App).mount();
