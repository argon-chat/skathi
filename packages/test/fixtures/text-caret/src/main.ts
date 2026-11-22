import { createApp, h, ref } from "@skathi-native/runtime";

const text = ref("");
const log: string[] = [];
(globalThis as { __SK_TEST_CARET__?: unknown }).__SK_TEST_CARET__ = { text, log };

const caret = (e: Event) => {
  const el = e.target as HTMLTextAreaElement;
  return `${el.selectionStart},${el.selectionEnd}`;
};

createApp({
  setup() {
    return () =>
      h("div", { id: "root", style: { padding: "8px", display: "flex", flexDirection: "column", gap: "8px" } }, [
        h("textarea", {
          id: "ta",
          rows: 3,
          value: text.value,
          onInput: (e: InputEvent) => {
            text.value = (e.target as HTMLTextAreaElement).value;
            log.push(`input:${e.inputType}@${caret(e)}`);
          },
          onKeydown: (e: KeyboardEvent) => log.push(`keydown:${e.key}@${caret(e)}`),
          onBeforeinput: (e: InputEvent) => log.push(`beforeinput:${e.inputType}@${caret(e)}`),
          onSelect: () => log.push("select"),
        }),
        h("input", { id: "short", maxlength: 5 }),
        h("input", { id: "plain" }),
        h("input", { id: "num", type: "number" }),
      ]);
  },
}).mount();
