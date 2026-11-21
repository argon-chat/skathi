// So a spec that wants to drive the tier-3 API rather than the host fns underneath it needs the
// app to hand it over, exactly as an app with an "Open devtools" menu item would have to.
import { createApp, h } from "@skathi-native/runtime";
import { appDevtools } from "skathi:app";

(globalThis as unknown as { __SK_TEST_DEVTOOLS__: unknown }).__SK_TEST_DEVTOOLS__ = appDevtools;

createApp({
  render: () => h("div", { class: "fixture" }, `devtools: ${appDevtools.isEnabled() ? "on" : "off"}`),
}).mount();
