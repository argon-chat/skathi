import { createApp, h } from "vue-native";

createApp({
  render: () =>
    h("div", { id: "clear", style: { position: "absolute", inset: "0" } }, [
      h("div", {
        id: "solid",
        style: { position: "absolute", left: "8px", top: "8px", width: "24px", height: "24px", background: "#ff0000" },
      }),
    ]),
}).mount();
