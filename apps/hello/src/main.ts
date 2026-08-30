import { createApp } from "vue-native";
import { createPinia } from "pinia";
import AppShell from "./AppShell.vue";
import { router } from "./router";

// Stock browser-Vue bootstrap: both plugins are the real published packages,
// seeded into the module registry by packages/vue-native/src/loader.ts.
//
// The cast is `VnMountableApp` (apps/env.d.ts): `mount()` really does take no
// container here, but `createApp`'s RETURN type still says it needs one.
(createApp(AppShell).use(createPinia()).use(router) as unknown as VnMountableApp).mount();
