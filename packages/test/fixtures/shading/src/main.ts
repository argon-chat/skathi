import { createApp, h } from "@skathi-native/runtime";
import "./shading.css";

const App = {
  render: () =>
    h("div", { id: "page", style: { display: "flex", gap: "8px", padding: "8px" } }, [
      h("div", { id: "glass", class: "tile glass" }),
      h("div", { id: "locked", class: "tile locked" }),
      h("div", { id: "inline", class: "tile", style: { "--shading": "glow(8px, 0.5)" } }),
      h("div", { id: "plain", class: "tile" }),
    ]),
};

createApp(App).mount("#app");
