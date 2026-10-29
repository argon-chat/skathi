// The regression fixture for "a layout read in a LATER wave" (m26 client sweep
// #8).
//
// `measure-on-mount` pins the reads made before the first frame. This one
// mounts its lists afterwards, from the waves an app mounts things from: a
// timer, a fetch completion, a click. Each list does what the Argon chat list
// does: measure its own width in `onMounted`, keep the answer, and size its
// bubbles from it. Before #8 every one of those reads answered 0. The wave ran
// inside a gpui update that held the window, the forced layout could not draw,
// and the list sized its bubbles at `0px`.
import { createApp, defineComponent, h, onMounted, ref } from "vue-native";

const STAGES = ["timeout", "fetch", "click", "realclick"] as const;
type Stage = (typeof STAGES)[number];

/** Which lists are mounted. */
const shown = ref<Partial<Record<Stage, boolean>>>({});
/** What each list's `onMounted` read: `offsetWidth,rect height,scrollHeight`. */
const seen = ref<Partial<Record<Stage, string>>>({});

function show(stage: Stage) {
  shown.value = { ...shown.value, [stage]: true };
}

const List = defineComponent({
  props: { stage: { type: String, required: true } },
  setup(props) {
    const el = ref<HTMLElement | null>(null);
    const width = ref(0);
    onMounted(() => {
      const list = el.value;
      if (!list) return;
      width.value = list.offsetWidth;
      const r = list.getBoundingClientRect();
      seen.value = {
        ...seen.value,
        [props.stage]: `${list.offsetWidth},${Math.round(r.height)},${list.scrollHeight}`,
      };
    });
    return () =>
      h(
        "div",
        {
          id: `list-${props.stage}`,
          ref: el,
          style: { display: "flex", flexDirection: "column", padding: "8px", gap: "4px", background: "#202830" },
        },
        [
          // 60% of the measured width: `0px` when the read lied.
          h(
            "div",
            { id: `bubble-${props.stage}`, style: { width: `${Math.round(width.value * 0.6)}px`, height: "24px", background: "#3a7bd5" } },
            "hi",
          ),
          h("div", { style: { height: "24px", background: "#555555" } }, "row"),
        ],
      );
  },
});

// The spec's handle: a fetch against its loopback server. The list mounts in
// the `.then` after the response, which the host delivers in a later frame.
(globalThis as { __VN_TEST_LATE__?: unknown }).__VN_TEST_LATE__ = {
  fetchMount(url: string) {
    void fetch(url)
      .then((r) => r.text())
      .then(() => show("fetch"));
  },
};

createApp({
  setup() {
    onMounted(() => {
      // Late enough that frames have been painted: not the boot wave.
      setTimeout(() => show("timeout"), 300);
    });
    return () =>
      h("div", { id: "root" }, [
        h("div", { style: { display: "flex", gap: "8px" } }, [
          h("button", { id: "mount-click", onClick: () => show("click") }, "click"),
          h("button", { id: "mount-realclick", onClick: () => show("realclick") }, "real click"),
        ]),
        ...STAGES.map((k) => (shown.value[k] ? h(List, { key: k, stage: k }) : null)),
        // One box per stage, so only text changes across a patch.
        h(
          "div",
          { id: "late-seen" },
          STAGES.map((k) => h("div", { id: `seen-${k}` }, `${k}=${seen.value[k] ?? "?"}`)),
        ),
      ]);
  },
}).mount();
