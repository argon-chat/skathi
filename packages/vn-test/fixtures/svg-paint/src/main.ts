import { createApp, h, reactive } from "vue-native";
import "./svg-paint.css";

const state = reactive({ alt: false });

const square = (attrs: Record<string, unknown> = {}) =>
  h("rect", { x: "10", y: "10", width: "80", height: "40", ...attrs });

const cells = () => [
  h("div", { id: "dock", class: "cell dock" }, [
    h("svg", { id: "outline", class: ["outline", { alt: state.alt }], width: "100", height: "60" }, [
      h("path", { d: "M10 10 H90 V50 H10 Z" }),
    ]),
  ]),
  h("svg", { id: "attr-loses", class: "cell attr-loses", fill: "rgb(0, 0, 255)", width: "100", height: "60" }, [square()]),
  h("svg", { id: "inner-rule", class: "cell inner-rule", width: "100", height: "60" }, [square({ fill: "rgb(0, 0, 255)" })]),
  h("svg", { id: "inline-wins", class: "cell inline-wins", width: "100", height: "60" }, [
    square({ style: { fill: "rgb(0, 160, 0)" } }),
  ]),
  h("svg", { id: "inherit-loses", class: "cell inherit-loses", width: "100", height: "60" }, [
    square({ fill: "rgb(0, 160, 0)" }),
  ]),
  h("svg", { id: "shadowed", class: "cell shadowed", viewBox: "0 0 60 60" }, [h("circle", { cx: "30", cy: "30", r: "20" })]),
];

(globalThis as Record<string, unknown>).__VN_TEST_SVG__ = {
  state,
  toggle(): boolean {
    state.alt = !state.alt;
    return state.alt;
  },
};

createApp({
  render: () => h("div", { id: "root" }, cells()),
}).mount();
