import { createApp, h } from "@skathi-native/runtime";
import "./line-height.css";

const LABEL = "Open a link";
const WS = "\n  ";

const icon = () =>
  h("svg", { width: 14, height: 14, viewBox: "0 0 14 14" }, [
    h("rect", { width: 14, height: 14, fill: "currentColor" }),
  ]);

type Child = string | ReturnType<typeof h>;

const button = (id: string, children: string | Child[]) =>
  h("div", { class: "row" }, [h("button", { id, class: "btn" }, children)]);

const span = (id: string, text: string, style?: Record<string, string>, cls?: string) =>
  h("div", { class: "row" }, [h("span", { id, ...(cls ? { class: cls } : {}), ...(style ? { style } : {}) }, text)]);

const menuRow = (label: string) =>
  h("button", { style: { display: "flex", width: "100%", padding: "8px 10px", border: "0" } }, label);

createApp({
  setup() {
    return () =>
      h("div", { id: "root" }, [
        button("btn-plain", LABEL),
        button("btn-ws", `${WS}${LABEL}\n`),
        button("btn-icon", [WS, icon(), LABEL]),
        button("btn-icon-ws", [WS, icon(), `${WS}${LABEL}\n`]),
        button("btn-icon-end", [WS, icon(), `${LABEL}\n`]),
        button("btn-icon-start", [WS, icon(), `${WS}${LABEL}`]),
        span("item-plain", LABEL),
        span("item-ws", `${WS}${LABEL}\n`),

        h("div", { style: { fontSize: "13px" } }, [h("span", { id: "u13" }, "Settings…")]),
        h("div", { style: { fontSize: "13px", lineHeight: "normal" } }, [h("span", { id: "n13" }, "Settings…")]),
        h("div", { style: { fontSize: "15px" } }, [h("span", { id: "u15" }, "Settings…")]),
        h("div", { style: { fontSize: "15px", lineHeight: "normal" } }, [h("span", { id: "n15" }, "Settings…")]),
        h(
          "button",
          {
            id: "padded",
            style: {
              display: "block",
              width: "240px",
              padding: "8px 10px",
              border: "0",
              fontSize: "13px",
              textAlign: "left",
            },
          },
          "Fullscreen",
        ),

        span("pixel-undeclared", "Frame", undefined, "pixel"),
        span("pixel-normal", "Frame", { lineHeight: "normal" }, "pixel"),
        h("div", { class: "pixel", style: { lineHeight: "normal" } }, [
          h("div", { class: "row" }, [h("span", { id: "pixel-30", style: { fontSize: "30px" } }, "Frame")]),
          h("div", { class: "row" }, [h("span", { id: "serif-inherited", style: { fontFamily: "serif" } }, "Frame")]),
        ]),
        h("div", { class: "pixel" }, [
          h("div", { class: "row" }, [
            h("span", { id: "serif-declared", style: { fontFamily: "serif", lineHeight: "normal" } }, "Frame"),
          ]),
        ]),

        h("div", { id: "rows-ws" }, [" ", menuRow("A"), " ", menuRow("B")]),
        h("div", { id: "rows-plain" }, [menuRow("A"), menuRow("B")]),
      ]);
  },
}).mount();
