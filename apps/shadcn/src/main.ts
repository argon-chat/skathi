// Everything under src/components/ui is UNMODIFIED upstream shadcn-vue v2.2.0 (fetched by
// tools/fetch-registry.ts); everything else in src/ is the harness.

import { createApp } from "@skathi-native/runtime";

import App from "./App.vue";

/**
 * Browser Vue mounts into a container you pick out of a document — `mount("#app")` — because a
 * page can hold any number of roots.
 */
interface NativeApp {
  mount(): void;
}

(createApp(App) as unknown as NativeApp).mount();
