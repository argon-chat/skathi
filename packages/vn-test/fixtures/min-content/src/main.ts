// A fixture for packages/vn-test/specs/min-content.spec.ts: text's min-content
// width in a flex row. The row is Argon's dashboard stat row
// (apps/client/src/components/home/widgets/DailyStatsWidget.vue): an icon and
// a label on the left, the value on the right, `justify-content:
// space-between`, at five widths. Each row also has a `nowrap` twin, whose
// label must not shrink below its one line. Render functions rather than an
// SFC, for the reason fixtures/layout gives.
import { createApp, h } from "vue-native";

const WIDTHS = [100, 120, 140, 180, 240];

const row = (w: number, nowrap: boolean) => {
  const id = `${nowrap ? "n" : "r"}${w}`;
  return h(
    "div",
    {
      id: `row-${id}`,
      style: {
        display: "flex",
        alignItems: "center",
        justifyContent: "space-between",
        padding: "8px",
        width: `${w}px`,
        boxSizing: "border-box",
        border: "1px solid #888888",
        margin: "4px",
      },
    },
    [
      h("div", { id: `left-${id}`, style: { display: "flex", alignItems: "center", gap: "8px" } }, [
        h("div", { style: { padding: "6px" } }, [h("div", { style: { width: "16px", height: "16px", background: "#3388ff" } })]),
        h(
          "span",
          { id: `label-${id}`, style: { fontSize: "12px", fontWeight: "500", whiteSpace: nowrap ? "nowrap" : "normal" } },
          "Time in Voice",
        ),
      ]),
      h("span", { id: `value-${id}`, style: { fontSize: "14px", fontWeight: "700" } }, "0h 0m"),
    ],
  );
};

createApp({
  render: () =>
    h("div", { id: "root", style: { fontFamily: "Arial, sans-serif", background: "#ffffff", color: "#000000", display: "flex", gap: "8px" } }, [
      h("div", { id: "normal" }, WIDTHS.map((w) => row(w, false))),
      h("div", { id: "nowrap" }, WIDTHS.map((w) => row(w, true))),
    ]),
}).mount();
