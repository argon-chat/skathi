import { createApp, h } from "@skathi-native/runtime";

// The spec compiles color0..color5 from the golden table and points SK_EFFECTS_DIR at them.
createApp({
  render: () =>
    h(
      "div",
      { id: "root", style: { position: "relative", width: "100%", height: "100%", background: "#000" } },
      Array.from({ length: 6 }, (_, i) =>
        h("div", {
          id: `tile${i}`,
          style: {
            position: "absolute",
            left: "0px",
            top: `${i * 40}px`,
            width: "280px",
            height: "40px",
            "--shading": `color${i}()`,
          },
        }),
      ),
    ),
}).mount("#app");
