import { createApp, h } from "@skathi-native/runtime";
import "./style.css";

const options = () => [h("option", { value: "s" }, "Short"), h("option", { value: "l" }, "A longer option")];
const trim = (id: string, className: string, text = "SAVE", tag = "p") =>
  h(tag, { id, class: "example " + className }, text);

createApp({
  render: () =>
    h("main", [
      h("input", { id: "empty", type: "text" }),
      h("input", { id: "input", type: "text", placeholder: "Placeholder" }),
      h("textarea", { id: "area", rows: "4", placeholder: "Type here" }),
      h("textarea", { id: "bounded-area", rows: "4", value: "a word ".repeat(20) }),
      h("select", { id: "select", value: "s" }, options()),
      h("select", { id: "fixed-select", value: "s" }, options()),
      h("section", { id: "type" }, [
        trim("untrimmed", ""),
        trim("cap-start", "cap-start", "SAVE", "button"),
        trim("alphabetic-end", "alphabetic-end"),
        trim("cap-both", "cap-both"),
        trim("ex-both", "ex-both"),
        trim("cap-two-lines", "cap-both", "SAVE\nSAVE"),
      ]),
    ]),
}).mount("#app");
