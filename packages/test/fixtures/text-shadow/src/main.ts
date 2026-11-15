import { createApp, h } from "@skathi-native/runtime";
import "./text-shadow.css";

createApp({
  render: () =>
    h("div", { id: "root" }, [
      h("div", { id: "hard", class: "cell" }, "HI"),
      h("div", { id: "soft-ref", class: "cell" }, "HI"),
      h("div", { id: "soft", class: "cell" }, "HI"),
      h("div", { id: "multi", class: "cell" }, "H"),
      h("div", { id: "inherit", class: "cell" }, [h("span", "H"), h("div", { id: "inherit-none" }, "H")]),
      h("div", { id: "current", class: "cell" }, [h("span", { id: "current-child" }, "H")]),
      h("div", { id: "trans", class: "cell" }, "H"),
      h("div", { id: "ellipsis", class: "cell" }, "Shadowed text that overflows its box"),
      h("div", { id: "bgclip", class: "cell" }, "HI"),
    ]),
}).mount();
