// `measure-on-mount` pins the reads made before the first frame. Before #8 every one of those
// reads answered 0.
import { createApp, defineComponent, h, onMounted, ref } from "vue-native";

const STAGES = ["timeout", "fetch", "click", "realclick", "mousedown", "keydown"] as const;
type Stage = (typeof STAGES)[number];

const shown = ref<Partial<Record<Stage, boolean>>>({});
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
          h(
            "div",
            {
              id: `bubble-${props.stage}`,
              style: { width: `${Math.round(width.value * 0.6)}px`, height: "24px", background: "#3a7bd5" },
            },
            "hi",
          ),
          h("div", { style: { height: "24px", background: "#555555" } }, "row"),
        ],
      );
  },
});

(globalThis as { __VN_TEST_LATE__?: unknown }).__VN_TEST_LATE__ = {
  fetchMount(url: string) {
    void fetch!(url)
      .then((r) => r.text!())
      .then(() => show("fetch"));
  },
};

createApp({
  setup() {
    onMounted(() => {
      setTimeout(() => show("timeout"), 300);
    });
    return () =>
      h("div", { id: "root" }, [
        h("div", { style: { display: "flex", gap: "8px" } }, [
          h("button", { id: "mount-click", onClick: () => show("click") }, "click"),
          h("button", { id: "mount-realclick", onClick: () => show("realclick") }, "real click"),
          // Dispatched under gpui's listener: the patch and `onMounted` run from the checkpoint
          // the host settles after gpui's dispatch.
          h("button", { id: "mount-down", onMousedown: () => show("mousedown") }, "down"),
          h("button", { id: "mount-key", onKeydown: () => show("keydown") }, "key"),
        ]),
        ...STAGES.map((k) => (shown.value[k] ? h(List, { key: k, stage: k }) : null)),
        h(
          "div",
          { id: "late-seen" },
          STAGES.map((k) => h("div", { id: `seen-${k}` }, `${k}=${seen.value[k] ?? "?"}`)),
        ),
      ]);
  },
}).mount();
