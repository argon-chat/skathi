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
      sticker("tint", "/src/half.json", { ...at(340, 140, 60), color: "#ff0000" }, { tint: "" }),
      sticker("tint-half", "/src/steps.json", { ...at(20, 220, 60), color: "rgba(0, 0, 255, 0.5)" }, { tint: "", play: "off" }),
      h("div", { id: "clip", style: { ...at(260, 140, 60), overflow: "hidden", background: "#eee" } }, [
        h("div", { style: { height: "300px", paddingTop: "200px" } }, [
          sticker("far", "/src/steps.json", { width: "60px", height: "60px" }),
        ]),
      ]),
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
