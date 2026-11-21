// By now the runtime has run and put its realm proxy on `WebSocket`, so the Network tee is
// installed over it.
import { createApp, h } from "@skathi-native/runtime";
import { appDevtools } from "skathi:app";

createApp({
  render: () => h("div", { id: "late", class: "fixture" }, "devtools turned on by the app"),
}).mount();

appDevtools.enable();
