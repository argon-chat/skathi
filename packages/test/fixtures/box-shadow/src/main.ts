import { createApp, h } from "@skathi-native/runtime";
import "./box-shadow.css";

createApp({
  render: () =>
    h(
      "div",
      { id: "root" },
      ["ring", "blur", "mixed", "ellipse", "square", "inset", "clipped"].map((id) => h("div", { id, class: "cell" })),
    ),
}).mount();
