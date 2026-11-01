// A fixture for packages/vn-test/specs/waapi.spec.ts: `el.animate()` on the
// shapes Argon uses it for. `#strip` is the counter digit's strip
// (apps/client/src/components/motionCounter/Digit.vue): 30 cells of 40px in a
// 40px window, resting on cell 10 through an inline transform, which a roll
// animates with `el.animate()` and leaves alone. The four boxes are for the
// fill, seek, cancel and finish-handler cases. The spec drives everything
// through `app.eval`.
// Render functions rather than an SFC, for the reason fixtures/layout gives.
import { createApp, h } from "vue-native";
import "./waapi.css";

const CELL = 40;
const REST = 10;

createApp({
  render: () =>
    h("div", { id: "root" }, [
      h("div", { id: "wrap", class: "wrap" }, [
        h(
          "div",
          { id: "strip", class: "strip", style: { transform: `translateY(${-REST * CELL}px)` } },
          Array.from({ length: 30 }, (_, i) => h("div", { class: `cell c${i % 2}` })),
        ),
      ]),
      h("div", { id: "fade", class: "box" }),
      h("div", { id: "seek", class: "box" }),
      h("div", { id: "cancel", class: "box" }),
      h("div", { id: "settle", class: "box" }),
    ]),
}).mount();
