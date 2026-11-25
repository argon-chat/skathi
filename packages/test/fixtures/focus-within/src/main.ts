import { createApp, h } from "@skathi-native/runtime";
import "./focus-within.css";

const App = {
  render: () =>
    h("div", { id: "page" }, [
      h("label", { id: "l" }, [h("input", { id: "i", type: "number", placeholder: "other" }), h("span", "s")]),
      h("div", { id: "box" }, [h("div", { id: "inner", tabindex: 0 }), h("textarea", { id: "ta" })]),
      h("div", { id: "solo", tabindex: 0 }),
    ]),
};

createApp(App).mount("#app");
