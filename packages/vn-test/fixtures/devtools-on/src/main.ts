// So a spec that wants to drive the tier-3 API rather than the host fns underneath it needs the
// app to hand it over, exactly as an app with an "Open devtools" menu item would have to.
import { appDevtools, createApp, h } from "vue-native";

(globalThis as unknown as { __VN_TEST_DEVTOOLS__: unknown }).__VN_TEST_DEVTOOLS__ = appDevtools;

createApp({
  render: () =>
    h("div", { class: "fixture" }, `devtools: ${appDevtools.isEnabled() ? "on" : "off"}`),
}).mount();
