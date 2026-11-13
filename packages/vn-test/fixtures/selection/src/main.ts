import { createApp, h } from "vue-native";

const state = { changes: 0, starts: 0, copies: [] as string[] };
(globalThis as unknown as { __VN_TEST_SEL__: unknown }).__VN_TEST_SEL__ = state;

document.addEventListener("selectionchange", () => state.changes++);
document.addEventListener("selectstart", () => state.starts++);
document.addEventListener("copy", () => state.copies.push(String(getSelection())));

const App = {
  render: () =>
    h("div", { id: "doc", style: { padding: "12px 16px", fontSize: "16px", lineHeight: "24px" } }, [
      h("p", { id: "p1", style: { margin: "0 0 12px 0" } }, ["Hello ", h("b", { id: "big" }, "big"), " world"]),
      h("p", { id: "p2", style: { margin: "0 0 12px 0" } }, "Second paragraph here"),
      h("p", { id: "none", style: { margin: "0 0 12px 0", userSelect: "none" } }, "Not selectable"),
      h("p", { id: "p3", style: { margin: "0 0 12px 0" } }, "Third one"),
      h("button", { id: "btn" }, "Press"),
      h("input", { id: "field", value: "field text", style: { marginLeft: "8px" } }),
      h(
        "div",
        { id: "scroller", style: { marginTop: "8px", height: "48px", width: "220px", overflow: "auto" } },
        Array.from({ length: 12 }, (_, i) => h("div", { id: `line${i + 1}` }, `Line ${i + 1}`)),
      ),
    ]),
};

createApp(App).mount("#app");
