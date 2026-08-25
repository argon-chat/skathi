import { createApp } from "vue-native";
import AppShell from "./AppShell.vue";
import { router } from "./router.js";

createApp(AppShell).use(router).mount();
