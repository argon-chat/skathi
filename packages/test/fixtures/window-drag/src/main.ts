import { createApp, h } from "@skathi-native/runtime";
import { appWindow } from "skathi:app";

const ticks: number[] = [];
const frames: number[] = [];
let presses = 0;
const stamp = (list: number[]) => {
  list.push(Date.now());
  if (list.length > 4000) list.splice(0, 2000);
};
setInterval(() => stamp(ticks), 10);
const frame = () => {
  stamp(frames);
  requestAnimationFrame(frame);
};
requestAnimationFrame(frame);

createApp({
  render: () =>
    h("div", { id: "app", style: { height: "100vh", background: "#1e1f22" } }, [
      h("div", {
        id: "bar",
        style: { height: "40px", background: "#2b2d31" },
        onMousedown: (e: MouseEvent) => {
          presses++;
          if (e.button === 0) appWindow.startDragging();
        },
      }),
    ]),
}).mount();

const within = (list: number[], from: number, to: number) => list.filter((t) => t > from && t < to).length;

(globalThis as Record<string, unknown>).__SK_TEST_DRAG__ = {
  get presses() {
    return presses;
  },
  between: (from: number, to: number) => ({ ticks: within(ticks, from, to), frames: within(frames, from, to) }),
};
