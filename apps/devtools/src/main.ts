// The vue-native inspector — a vue-native app that inspects OTHER vue-native
// apps (docs/DEVTOOLS-UI.md).
//
// Nothing special about this bootstrap: it is the stock `createApp().mount()`
// every app in this repo uses. The inspector is not a special build, not a
// panel welded into the host, not a second renderer — it is an app, which is
// the whole reason it exists in this shape: every bug in the framework is a bug
// in the devtools, and gets found on the day it lands.
//
// v2 changed WHERE THE BUNDLE COMES FROM and nothing about this file: the same
// `bun prod.ts` output is `include_str!`'d into `vn-app` by its build script, so
// F12 opens an inspector with no dist folder anywhere in the picture
// (crates/vn-app/src/inspector.rs). `bun devtools/ui.ts --target <slot>` opens
// that same embedded bundle from a shell — it execs `vn-app --devtools`, and no
// longer builds a dist slot of its own.
import { createApp } from "vue-native";
import "./theme.css";
import App from "./App.vue";

createApp(App).mount();
