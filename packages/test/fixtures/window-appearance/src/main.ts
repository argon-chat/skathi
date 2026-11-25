import { createApp, h, reactive, SkWindow } from "@skathi-native/runtime";

const S = reactive({ later: false });

const changes: boolean[] = [];
const scheme = matchMedia("(prefers-color-scheme: dark)");
scheme.addEventListener("change", () => changes.push(scheme.matches));

createApp({
  setup() {
    return () =>
      h("div", { id: "app", style: { position: "absolute", inset: "0", background: "#101017" } }, [
        h("div", { id: "bar", style: { height: "36px", background: "#16161f" } }),
        h(SkWindow, { title: "Pane", bounds: { x: 80, y: 120, width: 220, height: 140 } }, () =>
          h("div", { style: { width: "100%", height: "100%" } }, "pane"),
        ),
        S.later
          ? h(SkWindow, { title: "Later", focusable: false, bounds: { x: 340, y: 120, width: 220, height: 140 } }, () =>
              h("div", { style: { width: "100%", height: "100%" } }, "later"),
            )
          : null,
      ]);
  },
}).mount();

(globalThis as Record<string, unknown>).__SK_TEST_APPEARANCE__ = { S, changes };
