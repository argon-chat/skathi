import { createApp, h } from "@skathi-native/runtime";

// A spec compiles table0..table7 from its golden table and points SK_EFFECTS_DIR at them.
createApp({
  render: () =>
    h(
      "div",
      { id: "root", style: { position: "relative", width: "100%", height: "100%", background: "#000" } },
      Array.from({ length: 8 }, (_, i) =>
        h("div", {
          id: `tile${i}`,
          style: {
            position: "absolute",
            left: "0px",
            top: `${i * 60}px`,
            width: "280px",
            height: "60px",
            "--shading": `table${i}()`,
          },
        }),
      ),
    ),
}).mount("#app");
