// A fixture for packages/vn-test/specs/devtools-optout.spec.ts. Its twin next
// door is byte-identical; the two differ only in `vn.config.json`.
//
// The global is the point. The devtools `eval` command runs in GLOBAL scope
// (`(0, eval)(src)` in `crates/vn-app/src/devtools/mod.rs`), and `appDevtools`
// is a MODULE EXPORT — not reachable from there in a dev host (where modules
// live in the `__VN__` registry) and not in a prod one (where the whole graph
// is one closed IIFE). So a spec that wants to drive the tier-3 API rather than
// the host fns underneath it needs the app to hand it over, exactly as an app
// with an "Open devtools" menu item would have to.
import { appDevtools, createApp, h } from "vue-native";

(globalThis as unknown as { __VN_TEST_DEVTOOLS__: unknown }).__VN_TEST_DEVTOOLS__ = appDevtools;

createApp({
  render: () =>
    h("div", { class: "fixture" }, `devtools: ${appDevtools.isEnabled() ? "on" : "off"}`),
}).mount();
