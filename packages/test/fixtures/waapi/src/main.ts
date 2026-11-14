import { createApp, h } from "@skathi-native/runtime";
import "./waapi.css";

const CELL = 40;
const REST = 10;

createApp({
  render: () =>
    h("div", { id: "root" }, [
      h("div", { id: "wrap", class: "wrap" }, [
        h(
          "div",
          { id: "strip", class: "strip", style: { transform: `translateY(${-REST * CELL}px)` } },
          Array.from({ length: 30 }, (_, i) => h("div", { class: `cell c${i % 2}` })),
        ),
      ]),
      h("div", { id: "fade", class: "box" }),
      h("div", { id: "seek", class: "box" }),
      h("div", { id: "cancel", class: "box" }),
      h("div", { id: "settle", class: "box" }),
    ]),
}).mount();
