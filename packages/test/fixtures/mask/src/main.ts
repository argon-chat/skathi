import { createApp, h } from "@skathi-native/runtime";
import "./mask.css";

createApp({
  render: () =>
    h("div", { id: "root" }, [
      h("div", { id: "icon", class: "cell icon" }),
      h("div", { class: "cell zoom" }, [h("div", { id: "icon-scaled", class: "icon small" })]),
      h("div", { id: "dot", class: "cell dot" }),
      h("div", { id: "fade", class: "cell fade" }),
      h("div", { id: "stripes", class: "cell stripes" }),
      h("div", { id: "subtree", class: "cell subtree" }, [h("div", { class: "fill" }), h("span", "masked")]),
      h("div", { id: "luminance", class: "cell luminance" }),
      h("div", { id: "webkit", class: "cell webkit" }),
      h("div", { id: "pseudo", class: "cell pseudo" }),
      h("div", { id: "edge", class: "cell edge" }),
    ]),
}).mount();
