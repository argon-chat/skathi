import { createApp, h } from "@skathi-native/runtime";
import "./layerfx.css";

const App = {
  render: () =>
    h("div", { id: "page" }, [
      h("div", { class: "row" }, [
        h("div", { id: "swapped", class: "tile swap" }, [h("div", { id: "child", class: "child" })]),
        h("div", { id: "faded", class: "tile swap faded" }),
        h("div", { id: "counter", class: "tile count" }),
        h("div", { id: "appear", class: "tile appear" }),
      ]),
      h("div", { class: "row" }, [
        h("div", { id: "positioned", class: "tile swap" }, [
          h("div", { id: "pinned", class: "pinned" }),
          h("div", { id: "raised", class: "raised" }),
          h("div", { id: "fixed", class: "fixed" }),
        ]),
        h("div", { id: "outside", class: "tile" }, [h("div", { class: "pinned" })]),
      ]),
    ]),
};

createApp(App).mount("#app");
