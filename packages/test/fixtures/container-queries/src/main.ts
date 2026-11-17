import { appWindow, createApp, defineComponent, h, onMounted } from "@skathi-native/runtime";
import "./cq.css";

const dom = document;
const width = (id: string) => dom.getElementById(id)!.getBoundingClientRect().width;

type Sample = { container: number; child: number };

const state = {
  frame: 0,
  observed: [] as Sample[],
  sampling: false,
  sampled: [] as Sample[],
  appWindow,
  grow(on: boolean) {
    state.sampled = [];
    state.sampling = true;
    dom.getElementById("grow")!.classList.toggle("wide", on);
  },
};
(globalThis as { __SK_TEST_CQ__?: unknown }).__SK_TEST_CQ__ = state;

const tick = () => {
  state.frame++;
  if (state.sampling) state.sampled.push({ container: width("grow"), child: width("g") });
  requestAnimationFrame(tick);
};
requestAnimationFrame(tick);

const Root = defineComponent({
  setup() {
    onMounted(() => {
      new ResizeObserver((entries) => {
        state.observed.push({ container: entries[0]!.contentRect.width, child: width("media") });
      }).observe(dom.getElementById("shelf")!);
    });
    return () =>
      h("div", { id: "root" }, [
        h("div", { id: "shelf", class: "shelf" }, [
          h("div", { id: "card", class: "card" }, [
            h("div", { id: "media", class: "media" }),
            h("div", { id: "body", class: "body" }, "body"),
          ]),
        ]),
        h("div", { id: "page", class: "page" }, [
          h("div", { id: "panel", class: "panel" }, [h("div", { id: "badge", class: "badge" })]),
        ]),
        h("div", { id: "units", class: "units" }, [
          h("div", { id: "u", class: "u" }),
          h("div", { id: "u-min", class: "u-min" }),
        ]),
        h("div", { id: "grow", class: "grow" }, [h("div", { id: "g", class: "g" })]),
        h("div", { id: "pulse", class: "pulse" }, [h("div", { id: "beat", class: "beat" })]),
      ]);
  },
});

createApp(Root).mount();
