// A fixture for packages/vn-test/specs/backdrop.spec.ts: the window's backdrop
// material (docs/WINDOWING.md §4.4). DWM composites a backdrop UNDER what the
// app paints, so nearly the whole window is left unpainted: `#clear` is the
// transparent area the spec reads the material from, and `#solid`, an opaque
// red square in the top-left corner, is what proves a screenshot is the window
// at all. The window opens on `acrylic` (its vn.config.json); the spec
// switches the rest through `appWindow.backdrop`.
//
// Render functions rather than an SFC, for the reason fixtures/layout gives.
import { createApp, h } from "vue-native";

createApp({
  render: () =>
    h("div", { id: "clear", style: { position: "absolute", inset: "0" } }, [
      h("div", { id: "solid", style: { position: "absolute", left: "8px", top: "8px", width: "24px", height: "24px", background: "#ff0000" } }),
    ]),
}).mount();
