import { createApp, h } from "@skathi-native/runtime";
import "./vertical-align.css";

const PIXEL =
  "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mNkYAAAAAYAAjCB0C8AAAAASUVORK5CYII=";

/** One line of text: a reference word, then the item under test, then more text. */
const line = (name: string, item: ReturnType<typeof h>, cls?: string) =>
  h("p", { id: `${name}-line`, ...(cls ? { class: cls } : {}) }, [
    "x",
    h("span", { id: `${name}-ref` }, "ref"),
    item,
    "z",
  ]);

const box = (name: string, align?: string) =>
  h("span", { id: name, class: "box", style: align ? { verticalAlign: align } : {} });

createApp({
  setup() {
    return () =>
      h("div", { id: "root" }, [
        h("p", { id: "plain-line" }, ["x", h("span", { id: "plain-ref" }, "ref"), "z"]),
        line("sup", h("sup", { id: "sup" }, "2")),
        line("sub", h("sub", { id: "sub" }, "2")),
        line("shift", h("span", { id: "shift", style: { verticalAlign: "8px" } }, "up")),
        line("base", box("base"), "big"),
        line("middle", box("middle", "middle"), "big"),
        line("top", box("top", "top")),
        line("bottom", box("bottom", "bottom")),
        line("text-top", box("text-top", "text-top")),
        line("text-bottom", box("text-bottom", "text-bottom")),
        line("img-base", h("img", { id: "img-base", class: "pic", src: PIXEL }), "big"),
        line("img", h("img", { id: "img", class: "pic", src: PIXEL, style: { verticalAlign: "middle" } }), "big"),
        h("div", { id: "ex-ruler", class: "big" }),
      ]);
  },
}).mount();
