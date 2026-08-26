// shadcn-vue compatibility trial — entry point.
//
// See research/reports/m10/shadcn.md for what this app measures and how.
// Everything under src/components/ui is UNMODIFIED upstream shadcn-vue v2.2.0
// (fetched by tools/fetch-registry.ts); everything else in src/ is the harness.

import { createApp } from "vue-native";

// The DOM surface reka-ui expects but our host does not have yet. Import order
// matters: it must run before any reka-ui module is required, i.e. before
// App.vue is pulled in. Set `globalThis.__VN_NO_DOM_SHIM__ = true` (or comment
// the import) to measure the STOCK host instead — that is the A/B in the report.
import "./compat/dom-shim.js";

import App from "./App.vue";

createApp(App).mount();
