import { createApp, h } from "@skathi-native/runtime";
import "./transformed-text.css";

const TEXT = "Saved draw-forward.png";
const PREFIXES = { s: "S", save: "Save", saved: "Saved", space: "Saved ", draw: "Saved draw", hyphen: "Saved draw-" };

createApp({
  render: () =>
    h("div", { id: "root" }, [
      h("p", { id: "centred", class: "line" }, TEXT),
      h("p", { id: "nudged", class: "line" }, TEXT),
      h("p", { id: "rotated", class: "line" }, TEXT),
      ...Object.entries(PREFIXES).map(([key, text]) => h("p", { id: `prefix-${key}`, class: "line prefix" }, text)),
    ]),
}).mount();
