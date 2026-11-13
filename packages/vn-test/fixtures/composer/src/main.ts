// A browser runs it at the checkpoint after the keydown event, BEFORE the field inserts the key.
import { createApp, h, ref } from "vue-native";

const dom = document as unknown as VnDomDocument;

const text = ref("");
const showVictim = ref(true);
const state = {
  mode: "send" as "send" | "clear" | "upper",
  sent: [] as string[],
  focusLog: [] as string[],
};
(globalThis as { __VN_TEST_COMPOSER__?: unknown }).__VN_TEST_COMPOSER__ = {
  state,
  text,
  reset(mode: typeof state.mode) {
    state.mode = mode;
    state.sent.length = 0;
    state.focusLog.length = 0;
    text.value = "";
    showVictim.value = true;
  },
};
const logFocus = (e: FocusEvent) => state.focusLog.push(`${e.type}-${(e.target as HTMLElement).id}`);

function onKeydown(e: KeyboardEvent) {
  if (state.mode === "send" && e.key === "Enter") {
    e.preventDefault();
    state.sent.push(text.value);
    text.value = "";
  } else if (state.mode === "clear" && e.key === "Enter") {
    text.value = "";
  } else if (state.mode === "upper" && e.key === "x") {
    text.value = text.value.toUpperCase();
  }
}

createApp({
  setup() {
    return () =>
      h("div", { id: "root", style: { padding: "8px", display: "flex", flexDirection: "column", gap: "8px" } }, [
        h("textarea", {
          id: "composer",
          rows: 3,
          value: text.value,
          onInput: (e: Event) => (text.value = (e.target as HTMLTextAreaElement).value),
          onKeydown,
        }),
        h("button", { id: "other", tabindex: 0, onFocus: logFocus, onBlur: logFocus }, "other"),
        h("button", { id: "keep", tabindex: 0, onMousedown: (e: MouseEvent) => e.preventDefault() }, "keep"),
        h("div", { id: "keep-plain", onMousedown: (e: MouseEvent) => e.preventDefault() }, "keep-plain"),
        showVictim.value
          ? h(
              "button",
              {
                id: "victim",
                tabindex: 0,
                onFocus: logFocus,
                onBlur: logFocus,
                onMousedown: () => {
                  state.focusLog.push(`mousedown:active-${dom.activeElement?.id || "none"}`);
                  queueMicrotask(() => (showVictim.value = false));
                },
              },
              "victim",
            )
          : null,
      ]);
  },
}).mount();
