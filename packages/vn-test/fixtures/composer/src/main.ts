// A chat composer's key handling against a text field's default action (m26
// client sweep #8, docs/RUNTIME.md §5.8).
//
// A keydown handler's Vue patch is a microtask. A browser runs it at the
// checkpoint after the keydown event, BEFORE the field inserts the key. The
// host now leaves a listener's checkpoint for after gpui's dispatch, except
// in a text field, where it is settled before the widget acts. `mode` picks
// what the keydown handler does; `__VN_TEST_COMPOSER__.sent` records sends.
import { createApp, h, ref } from "vue-native";

const text = ref("");
const showVictim = ref(true);
const state = {
  mode: "send" as "send" | "clear" | "upper",
  sent: [] as string[],
  /** focus / blur on #other and #victim, and what `document.activeElement` read in the mousedown. */
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
    // Enter-to-send: cancel the newline, send, clear.
    e.preventDefault();
    state.sent.push(text.value);
    text.value = "";
  } else if (state.mode === "clear" && e.key === "Enter") {
    // Clear WITHOUT cancelling: a browser clears, then inserts the newline.
    text.value = "";
  } else if (state.mode === "upper" && e.key === "x") {
    // Rewrite the value, and let the key through: "ABC" then "x".
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
        // `@mousedown.prevent`: the press keeps the caret where it is (a mention
        // list, a toolbar button). One focusable, one not.
        h("button", { id: "keep", tabindex: 0, onMousedown: (e: MouseEvent) => e.preventDefault() }, "keep"),
        h("div", { id: "keep-plain", onMousedown: (e: MouseEvent) => e.preventDefault() }, "keep-plain"),
        // A press whose own handler hides the target from a microtask (a reka
        // DismissableLayer closing on pointerdown-outside has this shape).
        showVictim.value
          ? h(
              "button",
              {
                id: "victim",
                tabindex: 0,
                onFocus: logFocus,
                onBlur: logFocus,
                onMousedown: () => {
                  state.focusLog.push(`mousedown:active-${document.activeElement?.id || "none"}`);
                  queueMicrotask(() => (showVictim.value = false));
                },
              },
              "victim",
            )
          : null,
      ]);
  },
}).mount();
