// The regression fixture for "a boot-time `rem` recompile is not a style change".
//
// `rem` is flattened to px when the STYLESHEET IS COMPILED — gpui's style
// fields are `Pixels` — so an app that sets its own root font-size makes the
// engine compile twice: once against the 16 px default, then again against the
// real basis. `restyle_all` used to learn the basis by CASCADING the document,
// which commits `node.computed`; the pass after the recompile therefore looked
// to the animator like an author's style change, and every element carrying a
// `transition` animated from the 16 px-basis geometry to the real one.
//
// Nothing on screen showed it: no frame ran between the two passes. A LAYOUT
// READ did. `apps/meet-probe`'s join card measures its panel from `onMounted`,
// which goes through `sync_layout`'s forced frame, which is t=0 of that
// transition — so the card measured 293 px where the settled layout (and a
// browser) says 278, and wrote the wrong number back as its own height for the
// life of the process (research/reports/m23/tails.md §3).
//
// Every length below is `rem`, so every one of them moves with the basis, and
// each of the three that carries a `transition` is one of the three the join
// card was wrong about: the container's PADDING, a sibling's MARGIN, and a
// box's HEIGHT.
//
//                             at 14 px (right)   at 16 px (the transition's start)
//   padding      1.5rem  ×2         42                     48
//   the fixed row                   60                     60
//   margin-top   1.5rem             21                     24
//   height       3rem               42                     48
//                                  ---                    ---
//                                  165                    180
import { createApp, h, nextTick, onMounted, ref } from "vue-native";

const STAGES = ["mounted", "nextTick", "raf", "late"] as const;

/** The height the settled layout has, and therefore the only right answer. */
const REAL = 165;

const seen = ref<Record<string, number>>({});

function measure(stage: string) {
  const el = document.querySelector("#probe-panel") as HTMLElement | null;
  const px = el ? el.scrollHeight : -1;
  seen.value = { ...seen.value, [stage]: px };
  return px;
}

// EXACTLY what `@argon/theme` does, and the whole reason the sheet is compiled
// twice: the root font-size is an INLINE style written before the app mounts,
// so the cold compile cannot have known it.
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
      // Long after any 300 ms transition would have finished, so the spec can
      // assert that the early stages saw the SETTLED number rather than a
      // number that merely happened to be close.
      setTimeout(() => measure("late"), 600);
    });
    return () =>
      h("div", { id: "root" }, [
        // A runtime `<style>`, which `sync_runtime_sheets` installs into the
        // cascade — the same path a component library's injected sheet takes.
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
