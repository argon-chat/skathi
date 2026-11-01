// A fixture for packages/vn-test/specs/sticker.spec.ts: `<vn-sticker>`,
// vue-native's stickers and animated emoji (crates/vn-tree/src/sticker.rs),
// laid out on white.
//
// - `steps.json` is a 100x100 Lottie at 30 fps whose square is red for frames
//   0-29, green for 30-59 and blue for 60-89 (hold keyframes), so a pixel tells
//   which second of the loop is on screen.
// - `half.json` is a still 100x100 black square at 50% opacity: on white a
//   straight-alpha pipeline shows mid grey, a premultiplied one darker.
// - `star.tgs` is vn-lottie's spinning star.
//
// `#a` and `#b` are two copies of `steps.json` at one size (one target, one
// clock); `#small` is a third at emoji size; `#off` has `play="off"` (stays
// red); `#once` plays one loop and ends on blue; `#half`; `#star`; `#bad` names
// a file that is not there; `#far` is scrolled out of `#clip`, an
// `overflow: hidden` box, so it is never visible; `#deep` sits in an in-flow
// subtree big enough to be a retained-render cache boundary (crates/vn-tree/
// src/cache.rs), whose replays must still show its newest frame.
// `__VN_TEST_STICKER__.events` records `<type>:<id>` for every load, error and
// ended; `add(id, src, left)` puts one more 40 px sticker on the bottom row
// (a spec's loopback URL), and `state.deep` is `#deep`'s size.
// Render functions, as in `layout`.
import { createApp, h, reactive } from "vue-native";

const events: string[] = [];
const state = reactive({ deep: 40, extra: [] as { id: string; src: string; left: number }[] });
(globalThis as unknown as { __VN_TEST_STICKER__: unknown }).__VN_TEST_STICKER__ = {
  events,
  state,
  add: (id: string, src: string, left: number) => state.extra.push({ id, src, left }),
};

const at = (left: number, top: number, size: number) => ({
  position: "absolute",
  left: `${left}px`,
  top: `${top}px`,
  width: `${size}px`,
  height: `${size}px`,
});

const sticker = (id: string, src: string, style: Record<string, string>, extra: Record<string, unknown> = {}) =>
  h("vn-sticker", {
    id,
    src,
    style,
    ...extra,
    onLoad: () => events.push(`load:${id}`),
    onError: () => events.push(`error:${id}`),
    onEnded: () => events.push(`ended:${id}`),
  });

createApp({
  render: () =>
    h("div", { id: "root", style: { position: "relative", width: "100%", height: "100%", background: "#fff" } }, [
      sticker("a", "/src/steps.json", at(20, 20, 100)),
      sticker("b", "/src/steps.json", at(140, 20, 100)),
      sticker("small", "/src/steps.json", at(260, 20, 24)),
      sticker("half", "/src/half.json", at(300, 20, 60)),
      sticker("star", "/src/star.tgs", at(380, 20, 80)),
      sticker("off", "/src/steps.json", at(20, 140, 60), { play: "off" }),
      sticker("once", "/src/steps.json", at(100, 140, 60), { once: "" }),
      sticker("bad", "/src/missing.json", at(180, 140, 60)),
      // Tinted: the frame's pixels take the element's colour by its alpha.
      sticker("tint", "/src/half.json", { ...at(340, 140, 60), color: "#ff0000" }, { tint: "" }),
      sticker("tint-half", "/src/steps.json", { ...at(20, 220, 60), color: "rgba(0, 0, 255, 0.5)" }, { tint: "", play: "off" }),
      h("div", { id: "clip", style: { ...at(260, 140, 60), overflow: "hidden", background: "#eee" } }, [
        // In flow: an absolutely positioned box may escape the clip, and
        // counts as visible.
        h("div", { style: { height: "300px", paddingTop: "200px" } }, [
          sticker("far", "/src/steps.json", { width: "60px", height: "60px" }),
        ]),
      ]),
      // The panel is out of flow, and so refused as a boundary; the in-flow
      // box inside it, five nodes deep, is not.
      h("div", { id: "panel", style: { ...at(100, 220, 120), height: "80px" } }, [
        h("div", { id: "deep-box", style: { display: "flex", alignItems: "center", gap: "4px" } }, [
          h("span", { style: { width: "8px", height: "8px", background: "#ccc" } }),
          h("span", { style: { width: "8px", height: "8px", background: "#ccc" } }),
          h("div", { style: { display: "flex" } }, [
            sticker("deep", "/src/steps.json", { width: `${state.deep}px`, height: `${state.deep}px` }),
          ]),
        ]),
      ]),
      ...state.extra.map((e) => sticker(e.id, e.src, at(e.left, 260, 40))),
    ]),
}).mount();
