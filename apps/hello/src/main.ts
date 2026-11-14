import { createApp } from "@skathi-native/runtime";
import { createPinia } from "pinia";
import AppShell from "./AppShell.vue";
import { router } from "./router";

createApp(AppShell).use(createPinia()).use(router).mount();
