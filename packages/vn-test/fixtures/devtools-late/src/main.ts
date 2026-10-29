// A fixture for packages/vn-test/specs/reload.spec.ts. The devtools start OFF
// (vn.config.json) and this module turns them on. By now the runtime has run
// and put its realm proxy on `WebSocket`, so the Network tee is installed over
// it. That is the case a reload used to lose (crates/vn-app/src/devtools/net.rs,
// "Across a reload").
//
// The app runs again on `location.reload()`, and so does this call. The
// devtools are already on by then, so it changes nothing.
import { appDevtools, createApp, h } from "vue-native";

createApp({
  render: () => h("div", { id: "late", class: "fixture" }, "devtools turned on by the app"),
}).mount();

appDevtools.enable();
