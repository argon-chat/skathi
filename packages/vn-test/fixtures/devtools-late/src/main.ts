// By now the runtime has run and put its realm proxy on `WebSocket`, so the Network tee is
// installed over it.
import { appDevtools, createApp, h } from "vue-native";

createApp({
  render: () => h("div", { id: "late", class: "fixture" }, "devtools turned on by the app"),
}).mount();

appDevtools.enable();
