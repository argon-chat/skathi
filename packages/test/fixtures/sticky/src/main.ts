import { createApp, defineComponent, h, ref } from "@skathi-native/runtime";
import "./sticky.css";

const state = {
  clicks: [] as string[],
};
(globalThis as { __SK_TEST_STICKY__?: unknown }).__SK_TEST_STICKY__ = state;

const rows = (section: number) =>
  Array.from({ length: 10 }, (_, i) =>
    h("div", { class: "row", "data-row": `${section}-${i}`, onClick: () => state.clicks.push(`row-${section}-${i}`) }),
  );

const section = (n: number, extra: string) =>
  h("div", { id: `section-${n}`, class: "section" }, [
    h("div", { id: `head-${n}`, class: `head ${extra}`, onClick: () => state.clicks.push(`head-${n}`) }),
    ...rows(n),
  ]);

const gridRow = (r: number) =>
  h(
    "div",
    { class: "grid-row" },
    Array.from({ length: 10 }, (_, c) => h("div", { id: `cell-${r}-${c}`, class: c === 0 ? "cell first" : "cell" })),
  );

// Rows added on demand for timing scroll frames with many sticky boxes.
const extra = ref(0);
(globalThis as { __SK_STICKY_ROWS__?: (n: number) => void }).__SK_STICKY_ROWS__ = (n: number) => {
  extra.value = n;
};

const Root = defineComponent({
  setup() {
    return () =>
      h("div", { id: "root" }, [
        h("div", { id: "list", class: "list" }, [section(1, ""), section(2, "second"), section(3, "")]),
        h("div", { id: "grid", class: "grid" }, [
          h("div", { id: "grid-head", class: "grid-row grid-head" }, [
            h("div", { id: "corner", class: "cell first corner" }),
            ...Array.from({ length: 9 }, (_, c) => h("div", { id: `col-${c + 1}`, class: "cell" })),
          ]),
          ...Array.from({ length: 30 + extra.value }, (_, r) => gridRow(r)),
        ]),
        h("div", { id: "outer", class: "outer" }, [
          h("div", { class: "outer-gap" }),
          h("div", { id: "inner", class: "inner" }, [
            h("div", { id: "inner-head", class: "inner-head" }),
            h("div", { class: "inner-body" }),
          ]),
          h("div", { class: "outer-tail" }),
        ]),
      ]);
  },
});

createApp(Root).mount();
