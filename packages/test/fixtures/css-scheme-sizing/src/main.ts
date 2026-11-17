import { createApp, h } from "@skathi-native/runtime";
import "./scheme-sizing.css";

// 40x40, so its natural ratio is 1.
const RED_SQUARE =
  "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAACgAAAAoCAIAAAADnC86AAAALElEQVR42u3NsQkAAAjAsP7/tD4huASyp6kXicVisVgsFovFYrFYLBaLxXcWqvU6G92VM/sAAAAASUVORK5CYII=";

const box = (id: string, cls = "", children?: ReturnType<typeof h>[]) => h("div", { id, class: cls }, children);
/** Each sized element is the one block child of a positioned slot, so it lays out in normal flow. */
const slot = (id: string, child: ReturnType<typeof h>) => h("div", { id: `${id}-slot`, class: "slot" }, [child]);
const img = (id: string) => h("img", { id, src: RED_SQUARE, alt: "" });

const ratios = () => [
  slot("ar-width", box("ar-width")),
  slot("ar-height", box("ar-height")),
  slot("ar-max", box("ar-max")),
  slot("ar-min", box("ar-min")),
  slot("ar-auto", box("ar-auto", "ratio-2")),
  slot("ar-auto-ratio", box("ar-auto-ratio")),
  slot("ar-content", box("ar-content", "", [box("ar-content-inner")])),
  slot("ar-anim", box("ar-anim")),
  slot("img-ratio", img("img-ratio")),
  slot("img-auto-ratio", img("img-auto-ratio")),
  slot("img-relative", img("img-relative")),
];

const cells = (id: string, keys: string[]) =>
  box(
    id,
    "flow",
    keys.map((k) => box(`${id}-${k}`, "cell")),
  );

const flows = () => [
  cells("ff-wrap", ["a", "b", "c"]),
  cells("ff-column", ["a", "b"]),
  cells("ff-reverse", ["a", "b", "c"]),
];

const schemes = () => [
  box("ld", "tone"),
  box("ld-light", "tone"),
  box("ld-dark", "tone"),
  box("ld-normal", "tone"),
  box("ld-var", "tone"),
  box("cursor-wait", "", [box("cursor-child", "tone")]),
  box("dark-panel", "panel", [h("input", { id: "cb-dark", type: "checkbox" })]),
  box("light-panel", "panel", [h("input", { id: "cb-light", type: "checkbox" })]),
  box("device"),
];

createApp({
  render: () => h("div", { id: "root" }, [...ratios(), ...flows(), ...schemes()]),
}).mount();
