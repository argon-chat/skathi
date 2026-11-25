import { createApp, h } from "@skathi-native/runtime";

// Cells of the same background, each holding one element with an effect at the same place, so two
// cells' elements see the same backdrop: a ramp in the first two rows, a flat colour in the third.
const RAMP = "linear-gradient(to right, #204080, #e0a040)";
const FLAT = "#3a7bd5";
const rows: Array<[string, Array<string | null>]> = [
  [
    RAMP,
    [
      null,
      "glasshand(url(/src/normals.png), 10px, 1)",
      "glasslib(url(/src/normals.png), 10px, 1)",
      "lenshand(18px, 24px, 0.12)",
      "lenslib(18px, 24px, 0.12)",
    ],
  ],
  [RAMP, [null, ...Array.from({ length: 7 }, (_, mode) => `kernels(${mode}, 2px)`)]],
  [FLAT, [...Array.from({ length: 7 }, (_, mode) => `kernels(${mode}, 2px)`), "button(16px, 24px, #ffffff20)"]],
];

createApp({
  render: () =>
    h(
      "div",
      { id: "root", style: { position: "relative", width: "100%", height: "100%", background: "#000" } },
      rows.flatMap(([background, cells], row) =>
        cells.map((shading, col) =>
          h(
            "div",
            {
              style: {
                position: "absolute",
                left: `${col * 120}px`,
                top: `${row * 120}px`,
                width: "120px",
                height: "120px",
                background,
              },
            },
            [
              h("div", {
                id: `c${row}${col}`,
                style: {
                  position: "absolute",
                  left: "20px",
                  top: "20px",
                  width: "80px",
                  height: "80px",
                  "border-radius": "24px",
                  ...(shading ? { "--shading": shading } : {}),
                },
              }),
            ],
          ),
        ),
      ),
    ),
}).mount("#app");
