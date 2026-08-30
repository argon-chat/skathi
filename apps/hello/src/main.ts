import { createApp } from "vue-native";
import { createPinia } from "pinia";
import AppShell from "./AppShell.vue";
import { router } from "./router.js";

// Stock browser-Vue bootstrap: both plugins are the real published packages,
// seeded into the module registry by packages/vue-native/src/loader.js.
createApp(AppShell).use(createPinia()).use(router).mount();
