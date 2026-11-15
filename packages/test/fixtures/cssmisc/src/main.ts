import { createApp, h } from "@skathi-native/runtime";
import "./cssmisc.css";

const ordered = (id: string, cls: string) =>
  h("div", { id, class: cls }, [
    h("div", { id: `${id}-a`, class: "item o-2" }, "A"),
    h("div", { id: `${id}-b`, class: "item o-neg" }, "B"),
    h("div", { id: `${id}-c`, class: "item" }, "C"),
  ]);

// 40x40 PNGs: an SVG's natural size is not its CSS size here.
const RED_SQUARE =
  "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAACgAAAAoCAIAAAADnC86AAAALElEQVR42u3NsQkAAAjAsP7/tD4huASyp6kXicVisVgsFovFYrFYLBaLxXcWqvU6G92VM/sAAAAASUVORK5CYII=";
const RED_OVER_BLUE =
  "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAACgAAAAoCAIAAAADnC86AAAAL0lEQVR42u3QsQkAMAzAMP//dHpFyFCBd4GbOikwGAwGg8Hgj2CzwWAwGAwGg9d7cNs6GyLw6J8AAAAASUVORK5CYII=";

const positioned = () => [
  h("img", { id: "op-none", class: "op op-none", src: RED_SQUARE, alt: "" }),
  h("img", { id: "op-cover", class: "op op-cover", src: RED_OVER_BLUE, alt: "" }),
  h("img", { id: "op-anim", class: "op op-anim", src: RED_SQUARE, alt: "" }),
];

const colours = () => [
  h("div", { id: "c-oklch", class: "swatch c-oklch" }),
  h("div", { id: "c-mix", class: "swatch c-mix" }),
  h("div", { id: "c-border", class: "swatch c-border" }),
];

const contained = () => [h("div", { id: "contain", class: "contain" }, [h("div", { class: "spill" })])];

const rows = (id: string) =>
  Array.from({ length: 20 }, (_, i) => h("div", { id: `${id}-row-${i}`, class: "row" }, `${i}`));

const scrollers = () => [
  h("div", { id: "sc-smooth", class: "scroller smooth" }, rows("sc-smooth")),
  h("div", { id: "sc-plain", class: "scroller plain" }, rows("sc-plain")),
];

createApp({
  render: () =>
    h("div", { id: "root" }, [
      ordered("flex", "flexrow"),
      ordered("grid", "gridrow"),
      ...positioned(),
      ...colours(),
      ...contained(),
      ...scrollers(),
    ]),
}).mount();
