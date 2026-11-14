import { createApp, h } from "@skathi-native/runtime";
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
