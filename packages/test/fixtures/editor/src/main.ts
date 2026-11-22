import "./editor.css";
import { createApp, h, shallowRef } from "@skathi-native/runtime";

interface Value {
  text: string;
  marks: { type: string; from: number; to: number; data?: unknown }[];
  atoms: { id: string; at: number; type: string; text?: string; data?: unknown }[];
  lines?: { line: number; type: string }[];
}
type Editor = HTMLElement & { value: Value };

const draft = shallowRef<Value>({ text: "", marks: [], atoms: [] });
const align = shallowRef("baseline");
const state = {
  log: [] as string[],
  changes: [] as unknown[],
  selections: 0,
  inputs: 0,
  blockEnter: false,
};
(globalThis as { __SK_TEST_EDITOR__?: unknown }).__SK_TEST_EDITOR__ = {
  state,
  draft,
  align,
  ed: () => document.querySelector("#ed") as Editor,
  ed2: () => document.querySelector("#ed2") as Editor,
  reset(value?: Value) {
    state.log.length = 0;
    state.changes.length = 0;
    state.selections = 0;
    state.inputs = 0;
    state.blockEnter = false;
    align.value = "baseline";
    draft.value = value ?? { text: "", marks: [], atoms: [] };
    const ed2 = document.querySelector("#ed2") as Editor | null;
    if (ed2) ed2.value = { text: "", marks: [], atoms: [] };
  },
};

function onBeforeinput(e: InputEvent) {
  const r = e.getTargetRanges()[0];
  state.log.push(`${e.inputType}|${e.data}|${r ? `${r.startOffset}-${r.endOffset}` : "none"}`);
  if (state.blockEnter && e.inputType === "insertParagraph") {
    e.preventDefault();
    state.log.push("sent");
  }
}

createApp({
  setup() {
    return () =>
      h("div", { id: "root", style: { padding: "8px", display: "flex", flexDirection: "column", gap: "8px" } }, [
        h(
          "sk-editor",
          {
            id: "ed",
            value: draft.value,
            placeholder: "Message",
            onBeforeinput,
            onInput: (e: Event) => {
              state.inputs++;
              draft.value = (e.target as Editor).value;
            },
            onChange: (e: Event & { changes?: unknown }) => state.changes.push(e.changes),
            onSelectionchange: () => state.selections++,
          },
          draft.value.atoms.map((a) =>
            h("div", { key: a.id, atom: a.id, class: "chip", style: { verticalAlign: align.value } }),
          ),
        ),
        h("sk-editor", { id: "ed2" }),
      ]);
  },
}).mount();
