import { createApp } from "vue-native";
import { createPinia } from "pinia";
import AppShell from "./AppShell.vue";
import { router } from "./router";

// Stock browser-Vue bootstrap: both plugins are the real published packages,
// seeded into the module registry by packages/vue-native/src/loader.ts.
//
createApp(AppShell).use(createPinia()).use(router).mount();
