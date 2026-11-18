import { createApp, h } from "@skathi-native/runtime";
import "./snap.css";

const items = (n: number, cls: string, prefix: string) =>
  Array.from({ length: n }, (_, i) => h("div", { class: cls, id: `${prefix}${i}` }, `${prefix}${i}`));

createApp({
  render: () =>
    h("div", { id: "root" }, [
      h("div", { id: "carousel", tabindex: 0 }, items(6, "slide", "s")),
      h("div", { id: "near" }, items(12, "row", "n")),
      h("div", { id: "pages", tabindex: 0 }, items(10, "page", "p")),
      h("div", { id: "padded" }, items(20, "line", "l")),
      h("div", { id: "framed" }, items(3, "frame", "f")),
      h("div", { id: "ledger" }, items(6, "entry", "e")),
    ]),
}).mount();
