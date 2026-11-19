import { createApp } from "@skathi-native/runtime";
import App from "./App.vue";
import { run } from "./scenarios";

const mainAt = performance.now();
createApp(App).mount();
void run(mainAt);
