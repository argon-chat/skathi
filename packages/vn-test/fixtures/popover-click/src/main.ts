// A real click that opens a reka-style popover, which focuses itself and
// measures on mount: the shape of the Argon member click (Mom → #564gtr →
// a member → the profile popover) that aborted the host with "FATAL:
// Attempting to release access but the mutator does not have access".
//
//   bun test packages/vn-test/specs/popover-click.spec.ts
//
// The trigger is a <button>, as reka's PopoverTrigger renders, so the press
// focuses it. That focus is applied after the mousedown and reported by gpui
// from the next draw. The click opens the popover from the mouse-up wave, and
// Vue's patch runs in JSC's own microtask drain at the end of that call. The
// content's onMounted does what reka's FocusScope and floating-ui do: focus
// the content, then measure the trigger and the content and place one under
// the other. Each read forces a layout, whose draw reports the trigger's
// pending focus. That `focus` dispatch therefore ran inside the running
// script, and it used to keep the engine lock past the drain's release of the
// heap access (crates/vn-runtime/src/bridge.rs `owe_drain`).
import { createApp, defineComponent, h, onMounted, ref } from "vue-native";

const state = {
  /** How many times the content mounted. */
  opened: 0,
  /** What each mount measured: the trigger's and the content's rects. */
  measured: [] as { trigger: number[]; content: number[] }[],
  /** focus / blur on the trigger and the content, in order. */
  focusLog: [] as string[],
  /** `document.activeElement`'s id after each mount. */
  active: [] as string[],
};
(globalThis as { __VN_TEST_POPOVER__?: unknown }).__VN_TEST_POPOVER__ = state;

const open = ref(false);
const trigger = ref<HTMLElement | null>(null);

const rect = (el: Element) => {
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
      // FocusScope: the content takes focus as it mounts.
      content.focus();
      state.active.push(document.activeElement?.id ?? "");
      // floating-ui: measure both, then place the content under the trigger.
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
