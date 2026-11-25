import { createApp, h } from "@skathi-native/runtime";
import { appWindow } from "skathi:app";

const ticks: number[] = [];
const frames: number[] = [];
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

type Direction = Parameters<typeof appWindow.startResizeDragging>[0];

const strip = (id: string, style: Record<string, string>, direction?: Direction) =>
  h("div", {
    id,
    style: { position: "fixed", ...style },
    onPointerdown: () => (direction ? appWindow.startResizeDragging(direction) : appWindow.startResizeDragging()),
  });

createApp({
  render: () =>
    h("div", { id: "app", style: { height: "100vh", background: "#1e1f22" } }, [
      // The repro from the issue: no direction, so the edge comes from the pointer.
      strip("right", { right: "0", top: "10px", bottom: "10px", width: "5px", cursor: "e-resize" }),
      strip("left", { left: "0", top: "10px", bottom: "10px", width: "5px", cursor: "w-resize" }, "West"),
      strip("top", { top: "0", left: "10px", right: "10px", height: "5px", cursor: "n-resize" }, "North"),
      strip("bottom", { bottom: "0", left: "10px", right: "10px", height: "5px", cursor: "s-resize" }, "South"),
      strip("corner", { right: "0", bottom: "0", width: "10px", height: "10px", cursor: "se-resize" }),
    ]),
}).mount();

const within = (list: number[], from: number, to: number) => list.filter((t) => t > from && t < to).length;
(globalThis as Record<string, unknown>).__SK_TEST_RESIZE__ = {
  between: (from: number, to: number) => ({ ticks: within(ticks, from, to), frames: within(frames, from, to) }),
};
