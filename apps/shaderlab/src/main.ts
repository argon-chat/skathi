import { createApp } from "@skathi-native/runtime";
import App from "./App.vue";

// shaderlab is deliberately the smallest app in the repo: one component, no router, no store.
createApp(App).mount();
