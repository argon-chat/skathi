// A fixture for packages/vn-test/specs/query-state.spec.ts (m26, docs/RUNTIME.md
// §5.27): a script's `matches` / `querySelectorAll` / `closest` filter
// `:hover`, `:active`, `:focus`, `:focus-visible` and `:focus-within` by the
// live state, as in a browser.
//
// * `#a` and `#b` are buttons in `#row`. `#b:focus-visible` paints a blue
//   ring, so the spec can see the painter agree with the query.
// * `#field` is a text field whose `:focus-visible` paints a red ring: a click
//   into a text field shows focus in Chromium.
// * `#pad` holds `#pad-inner`, to hover and to press.
//
// Render functions and a `.css` import, as in `layout` (see ../README.md).
import { createApp, h } from "vue-native";
import "./query-state.css";

const App = {
  render: () =>
    h("div", { id: "page" }, [
      h("div", { id: "row" }, [h("button", { id: "a" }, "a"), h("button", { id: "b" }, "b")]),
      h("input", { id: "field", type: "text" }),
      h("div", { id: "pad" }, [h("div", { id: "pad-inner" })]),
    ]),
};

createApp(App).mount("#app");
