import { createApp, h, nextTick, ref } from "@skathi-native/runtime";
import "./view-transition.css";

const flipped = ref(false);

const flip = () => {
  flipped.value = !flipped.value;
  return nextTick();
};

Object.assign(globalThis, { __flip: flip });

createApp({
  render: () =>
    h("div", { id: "page", class: flipped.value ? "dark" : "light" }, [
      h("div", { id: "card", class: flipped.value ? "card wide" : "card" }),
    ]),
}).mount();
