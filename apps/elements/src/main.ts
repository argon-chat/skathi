import { createApp } from "@skathi-native/runtime";
import App from "./App.vue";
import { installDemo } from "./demo";
import "./styles.css";

installDemo();
createApp(App).mount();
