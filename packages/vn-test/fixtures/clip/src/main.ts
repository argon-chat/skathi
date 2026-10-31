// A fixture for packages/vn-test/specs/clip.spec.ts: `overflow` clipping under
// a border-radius. The Argon client's avatars are reka's Avatar, an AvatarRoot
// spelled `rounded-full overflow-hidden` around an <img>, and its cards are
// `rounded-* overflow-hidden` around lists that scroll. gpui's content masks
// were rectangles, so all of these clipped square.
//
// The left column is what the spec measures. The right is the perf probe's
// scene: nine rounded cards, each a scrolling list of rows with a round dot,
// so a frame is mostly primitives under a rounded mask. Render functions
// rather than an SFC, for the reason fixtures/layout gives.
import { createApp, h } from "vue-native";
import "./clip.css";

/** An opaque red square, as an <img> source. */
const RED =
  "data:image/svg+xml,<svg xmlns='http://www.w3.org/2000/svg' width='64' height='64'><rect width='64' height='64' fill='red'/></svg>";

/** Full-width bands, alternating, with no gap: whatever sits in a corner of
 *  the card is painted, so a corner left unclipped shows. */
const bands = (n: number) =>
  Array.from({ length: n }, (_, i) => h("div", { class: i % 2 ? "band band-b" : "band band-a" }));

const avatars = () => [
  // reka's AvatarRoot + AvatarImage.
  h("span", { id: "avatar", class: "avatar" }, [h("img", { class: "avatar-img", src: RED, alt: "" })]),
  // An <img> rounded by its own `border-radius: 50%`.
  h("img", { id: "round-img", class: "round-img", src: RED, alt: "" }),
  // `50%` on a box that is not square, clipping its child.
  h("div", { id: "oval", class: "oval" }, [h("div", { class: "oval-fill" })]),
];

const cards = () => [
  // A rounded `overflow: hidden` card around a list that scrolls.
  h("div", { id: "card-a", class: "card-a" }, [h("div", { id: "list-a", class: "list-a" }, bands(12))]),
  // A rounded scroller with a border: the clip is the padding box, at the
  // inner radius (24 - 4).
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
