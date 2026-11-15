import { createApp, h } from "@skathi-native/runtime";
import "./layerfx.css";

const App = {
  render: () =>
    h("div", { id: "page", style: { display: "flex", gap: "8px", padding: "8px" } }, [
      h("div", { id: "swapped", class: "tile swap" }, [h("div", { id: "child", class: "child" })]),
      h("div", { id: "faded", class: "tile swap faded" }),
      h("div", { id: "counter", class: "tile count" }),
    ]),
};

createApp(App).mount("#app");
