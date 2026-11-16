import { createApp } from "@skathi-native/runtime";
import App from "./App.vue";
import { installClosing } from "./closing";
import { installDemo } from "./demo";
import "./styles.css";

installClosing();
installDemo();
createApp(App).mount();
