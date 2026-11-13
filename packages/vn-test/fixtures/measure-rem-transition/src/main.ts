import { createApp, h, nextTick, onMounted, ref } from "vue-native";

const STAGES = ["mounted", "nextTick", "raf", "late"] as const;

const REAL = 165;

const seen = ref<Record<string, number>>({});

function measure(stage: string) {
  const el = document.querySelector("#probe-panel") as HTMLElement | null;
  const px = el ? el.scrollHeight : -1;
  seen.value = { ...seen.value, [stage]: px };
  return px;
}

document.documentElement.style.fontSize = "14px";

const CSS = `
#probe-panel {
  /* Everything a transition-all container animates when the basis moves. */
  transition: all 300ms ease-out;
  padding: 1.5rem;
  background: #16202c;
}
#probe-row {
  height: 60px;
  background: #cc3333;
}
#probe-button {
  transition: all 300ms ease-out;
  margin-top: 1.5rem;
  height: 3rem;
  background: #3355cc;
}
`;

createApp({
  setup() {
    onMounted(() => {
      measure("mounted");
      void nextTick(() => measure("nextTick"));
      requestAnimationFrame(() => measure("raf"));
      // Long after any 300 ms transition would have finished, so the spec can assert that the
      // early stages saw the SETTLED number rather than a number that merely happened to be close.
      setTimeout(() => measure("late"), 600);
    });
    return () =>
      h("div", { id: "root" }, [
        h("style", {}, CSS),
        h("div", { id: "probe-panel" }, [
          h("div", { id: "probe-row" }, "row"),
          h("div", { id: "probe-button" }, "button"),
        ]),
        h(
          "div",
          { id: "probe-seen" },
          STAGES.map((k) =>
            h("div", { id: `stage-${k}` }, `${k}=${seen.value[k] === undefined ? "?" : seen.value[k]}`),
          ),
        ),
        h("div", { id: "probe-real" }, `real=${REAL}`),
      ]);
  },
}).mount();
