import { createApp, h, nextTick, onMounted, ref } from "@skathi-native/runtime";

const dom = document;

const STAGES = ["mounted", "nextTick", "raf", "timeout", "late"] as const;

const seen = ref<Record<string, number>>({});
const boxHeight = ref("auto");

function measure(stage: string) {
  const el = dom.querySelector("#probe-content") as HTMLElement | null;
  const px = el ? el.scrollHeight : -1;
  seen.value = { ...seen.value, [stage]: px };
  return px;
}

createApp({
  setup() {
    onMounted(() => {
      measure("mounted");
      void nextTick(() => {
        boxHeight.value = `${measure("nextTick")}px`;
      });
      requestAnimationFrame!(() => measure("raf"));
      setTimeout(() => measure("timeout"), 0);
      setTimeout(() => measure("late"), 400);
    });
    return () =>
      h("div", { id: "root" }, [
        h("div", { id: "probe-box", style: { overflow: "hidden", height: boxHeight.value } }, [
          h("div", { id: "probe-content", style: { padding: "20px" } }, [
            h("div", { style: { height: "60px", background: "#cc3333" } }, "one"),
            h("div", { style: { height: "60px", background: "#33cc33" } }, "two"),
          ]),
        ]),
        // One box per stage, so the vnode SHAPE never changes across a patch and only the text
        // does — the spec reads them by id.
        h(
          "div",
          { id: "probe-seen" },
          STAGES.map((k) =>
            h("div", { id: `stage-${k}` }, `${k}=${seen.value[k] === undefined ? "?" : seen.value[k]}`),
          ),
        ),
      ]);
  },
}).mount();
