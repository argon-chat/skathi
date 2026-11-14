// Fixture entry — a Vite-shaped app whose global stylesheet is IMPORTED from
// JS rather than linked from HTML. See test/build-graph.test.js.
import { createApp } from "@skathi-native/runtime";

import "./theme.css";
import App from "./App.vue";

createApp(App).mount();
