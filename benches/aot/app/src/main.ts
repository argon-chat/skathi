import { createApp } from "@skathi-native/runtime";
import App from "./App.vue";
import { bench } from "./bench";

bench.mainAt = performance.now();
createApp(App).mount();
