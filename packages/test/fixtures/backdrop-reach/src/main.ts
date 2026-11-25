import { createApp, h } from "@skathi-native/runtime";

const box = (id: string, left: number, top: number, width: number, height: number, style: Record<string, string>) =>
  h("div", {
    id,
    style: {
      position: "absolute",
      left: `${left}px`,
      top: `${top}px`,
      width: `${width}px`,
      height: `${height}px`,
      ...style,
    },
  });

const probe = (id: string, top: number, shading: string) => box(id, 100, top, 60, 30, { "--shading": shading });

createApp({
  render: () =>
    h("div", { id: "root", style: { position: "relative", width: "100%", height: "100%", background: "#fff" } }, [
      box("green", 0, 0, 170, 200, { background: "#00ff00" }),
      box("red", 170, 0, 30, 200, { background: "#ff0000" }),
      box("blue", 200, 0, 160, 200, { background: "#0000ff" }),
      probe("within-reach", 20, "reach(20px, 20px)"),
      probe("within-free", 60, "free(20px)"),
      probe("beyond-reach", 100, "reach(20px, 60px)"),
      probe("beyond-free", 140, "free(60px)"),
    ]),
}).mount("#app");
