import { createApp, h } from "@skathi-native/runtime";
import "./clip.css";

const RED =
  "data:image/svg+xml,<svg xmlns='http://www.w3.org/2000/svg' width='64' height='64'><rect width='64' height='64' fill='red'/></svg>";

const bands = (n: number) =>
  Array.from({ length: n }, (_, i) => h("div", { class: i % 2 ? "band band-b" : "band band-a" }));

const avatars = () => [
  h("span", { id: "avatar", class: "avatar" }, [h("img", { class: "avatar-img", src: RED, alt: "" })]),
  h("img", { id: "round-img", class: "round-img", src: RED, alt: "" }),
  h("div", { id: "oval", class: "oval" }, [h("div", { class: "oval-fill" })]),
  h("span", { id: "inh-img", class: "inh-img" }, [
    h("img", { src: RED, alt: "", style: "width: 100%; height: 100%; border-radius: inherit" }),
  ]),
  h("div", { id: "pcard", class: "pcard-x" }, [
    h("div", { class: "pc-bg" }, [h("div", { class: "pc-fill" })]),
    h("div", { class: "pc-zone" }, [h("div", { class: "pc-body" })]),
  ]),
];

const cards = () => [
  h("div", { id: "card-a", class: "card-a" }, [h("div", { id: "list-a", class: "list-a" }, bands(12))]),
  h("div", { id: "card-b", class: "card-b" }, bands(12)),
];

const perfScene = () =>
  h(
    "div",
    { id: "perf", class: "perf" },
    Array.from({ length: 9 }, (_, c) =>
      h("div", { class: "pcard" }, [
        h(
          "div",
          { class: "plist" },
          Array.from({ length: 30 }, (_, r) =>
            h("div", { class: "prow" }, [h("span", { class: `pdot pdot-${(c + r) % 3}` }), `Row ${r + 1}`]),
          ),
        ),
      ]),
    ),
  );

createApp({
  render: () =>
    h("div", { id: "root" }, [...avatars(), ...cards(), perfScene(), h("div", { id: "tick", class: "tick" })]),
}).mount();
