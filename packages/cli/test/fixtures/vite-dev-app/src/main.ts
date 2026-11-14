import { createApp } from "vue";
import App from "./App.vue";

declare const __DEV_FIX__: { on: boolean };
(globalThis as Record<string, unknown>).__devFix = __DEV_FIX__.on;

createApp(App).mount("#app");
