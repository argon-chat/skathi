// The vue-native inspector — a vue-native app that inspects OTHER vue-native
// apps (docs/DEVTOOLS-UI.md).
//
// Nothing special about this bootstrap: it is the stock `createApp().mount()`
// every app in this repo uses. The inspector is not a special build, not a
// panel welded into the host, not a second renderer — it is an app, which is
// the whole reason it exists in this shape: every bug in the framework is a bug
// in the devtools, and gets found on the day it lands.
import { createApp } from "vue-native";
import App from "./App.vue";

createApp(App).mount();
