import { createApp, h, reactive } from "vue-native";

const events: string[] = [];
const frames = { play: 0 };
const state = reactive({ toggle: false, extra: [] as { id: string; src: string; left: number }[] });
(globalThis as unknown as { __VN_TEST_LOTTIE__: unknown }).__VN_TEST_LOTTIE__ = {
  events,
  frames,
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

type Detail = { loops?: number; currentLoop?: number };
const lottie = (id: string, src: string, style: Record<string, string>, extra: Record<string, unknown> = {}) =>
  h("lottie", {
    id,
    src,
    style,
    ...extra,
    "onLoopComplete": (e: CustomEvent<Detail>) => events.push(`loopComplete:${id}:${e.detail?.currentLoop}`),
    onComplete: (e: CustomEvent<Detail>) => events.push(`complete:${id}:${e.detail?.loops}`),
    ...(id === "play" ? { onEnterFrame: () => frames.play++ } : {}),
  });

createApp({
  render: () =>
    h("div", { id: "root", style: { position: "relative", width: "100%", height: "100%", background: "#fff" } }, [
      lottie("play", "/src/steps.json", at(20, 20, 100), { autoplay: "", loop: "" }),
      lottie("count", "/src/steps.json", at(140, 20, 60), { autoplay: "", loop: "1", speed: "3" }),
      lottie("still", "/src/steps.json", at(220, 20, 60)),
      lottie("seg", "/src/steps.json", at(300, 20, 60), { autoplay: "", loop: "", segment: "30,59" }),
      lottie("half", "/src/half.json", at(380, 20, 60)),
      lottie("toggle", "/src/steps.json", at(20, 140, 60), { autoplay: state.toggle ? "" : "false", loop: "" }),
      ...state.extra.map((e) => lottie(e.id, e.src, at(e.left, 140, 60), { autoplay: "", loop: "" })),
    ]),
}).mount();
