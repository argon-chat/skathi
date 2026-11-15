import { createApp, h } from "@skathi-native/runtime";
import "./cssmisc.css";

const ordered = (id: string, cls: string) =>
  h("div", { id, class: cls }, [
    h("div", { id: `${id}-a`, class: "item o-2" }, "A"),
    h("div", { id: `${id}-b`, class: "item o-neg" }, "B"),
    h("div", { id: `${id}-c`, class: "item" }, "C"),
  ]);

createApp({
  render: () => h("div", { id: "root" }, [ordered("flex", "flexrow"), ordered("grid", "gridrow")]),
}).mount();
