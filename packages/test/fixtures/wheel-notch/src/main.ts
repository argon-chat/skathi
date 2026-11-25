// One wheel notch over a scroll container, a <textarea> and an <sk-editor>, with the `wheel` events
// it fires and the app's wheel scroll scale.
import { createApp, h } from "@skathi-native/runtime";
import { app } from "skathi:app";

const wheels: { deltaX: number; deltaY: number; deltaMode: number }[] = [];
addEventListener("wheel", (e) => wheels.push({ deltaX: e.deltaX, deltaY: e.deltaY, deltaMode: e.deltaMode }));

const lines = Array.from({ length: 400 }, (_, i) => `line ${i}`).join("\n");

(globalThis as { __SK_TEST_WHEEL__?: unknown }).__SK_TEST_WHEEL__ = { app, wheels };

const pane = { width: "180px", height: "160px", fontSize: "14px" };

createApp({
  setup() {
    return () =>
      h("div", { id: "root", style: { display: "flex", gap: "12px", padding: "12px" } }, [
        h("div", { id: "scroller", style: { ...pane, overflow: "auto", background: "#202830" } }, [
          h("div", { style: { height: "6000px" } }),
        ]),
        h("textarea", { id: "area", value: lines, style: { ...pane, resize: "none" } }),
        h("sk-editor", {
          id: "ed",
          readonly: true,
          value: { text: lines, marks: [], atoms: [] },
          style: { ...pane, background: "#fff", color: "#111" },
        }),
      ]);
  },
}).mount();
