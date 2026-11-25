import { createApp, h, ref } from "@skathi-native/runtime";
import { appHost } from "skathi:app";

const glass = ref(true);
Object.assign(globalThis, {
  showGlass: (on: boolean) => (glass.value = on),
  hostCall: (name: string, ...args: unknown[]) => appHost.call(name, ...args),
});

const at = (left: number, top: number, width: number, height: number, extra = "") =>
  `position: absolute; left: ${left}px; top: ${top}px; width: ${width}px; height: ${height}px; ${extra}`;

createApp({
  render: () =>
    h("div", { id: "root", style: "position: absolute; inset: 0" }, [
      h("div", {
        id: "page",
        style: at(0, 0, 360, 240, "background: rgba(255, 255, 255, 0.25)"),
      }),
      h("sk-layer", {
        id: "layer",
        name: "pictured",
        style: at(40, 40, 200, 120, "border-radius: 24px"),
      }),
      h("div", {
        id: "glass",
        style: at(80, 60, 120, 80, `backdrop-filter: blur(4px)${glass.value ? "" : "; display: none"}`),
      }),
      h("div", {
        id: "corner",
        style: at(20, 20, 60, 60, "backdrop-filter: blur(0.5px)"),
      }),
      h("sk-surface", {
        id: "inline",
        name: "pictured",
        style: at(260, 170, 80, 50, "object-fit: fill"),
      }),
    ]),
}).mount();
