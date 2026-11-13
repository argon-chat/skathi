import { createApp, defineComponent, h, onMounted, ref } from "vue-native";

const dom = document as unknown as VnDomDocument;

const state = {
  opened: 0,
  measured: [] as { trigger: number[]; content: number[] }[],
  focusLog: [] as string[],
  active: [] as string[],
};
(globalThis as { __VN_TEST_POPOVER__?: unknown }).__VN_TEST_POPOVER__ = state;

const open = ref(false);
const trigger = ref<HTMLElement | null>(null);

const rect = (el: Element): [number, number, number, number] => {
  const r = el.getBoundingClientRect();
  return [Math.round(r.left), Math.round(r.top), Math.round(r.width), Math.round(r.height)];
};

const Content = defineComponent({
  setup() {
    const el = ref<HTMLElement | null>(null);
    const top = ref(0);
    const left = ref(0);
    onMounted(() => {
      const content = el.value;
      const anchor = trigger.value;
      if (!content || !anchor) return;
      state.opened++;
      content.focus();
      state.active.push(dom.activeElement?.id ?? "");
      const t = rect(anchor);
      const c = rect(content);
      state.measured.push({ trigger: t, content: c });
      top.value = t[1] + t[3] + 4;
      left.value = t[0];
    });
    return () =>
      h(
        "div",
        {
          ref: el,
          id: "content",
          role: "dialog",
          tabindex: -1,
          style: {
            position: "absolute",
            top: `${top.value}px`,
            left: `${left.value}px`,
            width: "200px",
            padding: "12px",
            background: "#2a3440",
            color: "#fff",
          },
          onFocus: () => state.focusLog.push("focus-content"),
          onBlur: () => state.focusLog.push("blur-content"),
        },
        "Aram",
      );
  },
});

createApp({
  setup() {
    return () =>
      h("div", { id: "root", style: { padding: "16px", position: "relative", height: "100%" } }, [
        h(
          "button",
          {
            ref: trigger,
            id: "member",
            style: { width: "160px", height: "36px" },
            onFocus: () => state.focusLog.push("focus-member"),
            onBlur: () => state.focusLog.push("blur-member"),
            onClick: () => {
              open.value = !open.value;
            },
          },
          "Aram",
        ),
        open.value ? h(Content) : null,
      ]);
  },
}).mount();
