import { createApp, h } from "@skathi-native/runtime";

const tile = (id: string, left: number, shading: string) =>
  h("div", {
    id,
    style: {
      position: "absolute",
      left: `${left}px`,
      top: "20px",
      width: "80px",
      height: "80px",
      "--shading": shading,
    },
  });

createApp({
  render: () =>
    h("div", { id: "root", style: { position: "relative", width: "100%", height: "100%", background: "#fff" } }, [
      tile("colour", 0, "stamp(url(/src/halves.png))"),
      tile("linear", 90, "raw(url(/src/halves.png))"),
      tile("missing", 180, "stamp(url(/src/missing.png))"),
      tile("other", 270, "stamp(url(/src/yellow.png))"),
    ]),
}).mount("#app");
