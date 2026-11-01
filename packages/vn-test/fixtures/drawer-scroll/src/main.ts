// A fixture for packages/vn-test/specs/drawer-scroll.spec.ts: Argon's
// settings drawer (SettingsWindow.vue in @argon/ui's DrawerContent, which is
// vaul-vue's). The drawer is `position: fixed` at the bottom with `height:
// 0.5rem`, `max-height: 80vh` and `min-height: 95%` (min wins: 95% of the
// window), a flex column holding the handle, a header and a `flex: 1; height:
// 100%` row whose right column is the scroller (`flex: 1; overflow-y: auto`)
// with the category page in it. In Chromium the row is held to the drawer's
// height (its automatic minimum size is the smaller of its content and its
// definite `height: 100%`), so the scroller is too and scrolls.
//
// The same markup and sheet load in Chromium from drawer.html, for the
// numbers the spec expects.
// Render functions rather than an SFC, for the reason fixtures/layout gives.
import { createApp, h } from "vue-native";
import "./drawer.css";

const ROWS = 36;

createApp({
  render: () =>
    h("div", { id: "root" }, [
      h("div", { id: "drawer", class: "drawer" }, [
        h("div", { class: "handle" }),
        h("div", { id: "header", class: "header" }, [
          h("div", [h("h2", "Settings"), h("p", "Manage your settings")]),
          h("div", { class: "close" }),
        ]),
        h("div", { id: "layout", class: "layout" }, [
          h(
            "nav",
            { id: "nav", class: "nav" },
            Array.from({ length: 8 }, (_, i) => h("div", { class: "nav-item" }, `Category ${i + 1}`)),
          ),
          h(
            "div",
            { id: "content", class: "content" },
            Array.from({ length: ROWS }, (_, i) =>
              h("div", { class: i === ROWS - 1 ? "row last" : "row", id: i === ROWS - 1 ? "last" : undefined }, `Row ${i + 1}`),
            ),
          ),
        ]),
      ]),
    ]),
}).mount();
