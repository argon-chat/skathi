import { createApp, h } from "@skathi-native/runtime";
import "./corner-shape.css";

const keywords = ["round", "squircle", "bevel", "scoop", "notch", "square", "superellipse-3", "superellipse-neg"];

createApp({
  render: () =>
    h("div", { id: "root" }, [
      ...keywords.map((id, i) => h("div", { id, class: "box", style: { left: `${20 + i * 100}px`, top: "20px" } })),
      h("div", { id: "anim", class: "box" }),
      h("div", { id: "clip", class: "box" }, [h("div", { class: "fill" })]),
      h("div", { id: "shadow", class: "box" }),
      h("div", { id: "mixed", class: "box" }),
    ]),
}).mount();
