// The regression fixture for "a layout read before the first frame".
//
// This is `apps/meet-probe`'s join card reduced to its bones. That app wraps
// its tab panel in a height-animated box and, from `onMounted` + `nextTick`,
// measures the panel and writes the answer back as the wrapper's `height`:
//
//     const active = contentRef.value.querySelector(":scope > div.relative");
//     contentHeight.value = `${active.scrollHeight}px`;
//
// In a browser the read forces a layout, so it answers 160 and the card is 160
// tall. Before the fix it answered 0 here — nothing had been painted when it
// ran — the app wrote `height: 0px`, `overflow: hidden` clipped the whole card
// body away, and NOTHING ever re-measured it: the value it stored was final.
//
// The fixture reports what `scrollHeight` answered at every stage a real app
// measures from, so the spec can pin which of them are a browser's answer.
import { createApp, h, nextTick, onMounted, ref } from "vue-native";

const STAGES = ["mounted", "nextTick", "raf", "timeout", "late"] as const;

/** What `scrollHeight` answered, per stage — read by the spec through `text`. */
const seen = ref<Record<string, number>>({});
/** The measured height, written back the way the app under study writes it. */
const boxHeight = ref("auto");

function measure(stage: string) {
  const el = document.querySelector("#probe-content") as HTMLElement | null;
  const px = el ? el.scrollHeight : -1;
  seen.value = { ...seen.value, [stage]: px };
  return px;
}

createApp({
  setup() {
    onMounted(() => {
      measure("mounted");
      void nextTick(() => {
        // EXACTLY the app's line: measure, then commit the answer as a height.
        boxHeight.value = `${measure("nextTick")}px`;
      });
      requestAnimationFrame(() => measure("raf"));
      setTimeout(() => measure("timeout"), 0);
      setTimeout(() => measure("late"), 400);
    });
    return () =>
      h("div", { id: "root" }, [
        h(
          "div",
          { id: "probe-box", style: { overflow: "hidden", height: boxHeight.value } },
          [
            h("div", { id: "probe-content", style: { padding: "20px" } }, [
              h("div", { style: { height: "60px", background: "#cc3333" } }, "one"),
              h("div", { style: { height: "60px", background: "#33cc33" } }, "two"),
            ]),
          ],
        ),
        // One box per stage, so the vnode SHAPE never changes across a patch
        // and only the text does — the spec reads them by id.
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
