// A fixture for packages/vn-test/specs/beforeinput.spec.ts (m26 fixes,
// docs/RUNTIME.md §5.25): `beforeinput`, `input` and the composition events of
// a text field.
//
// - `#ta`, a `<textarea>`, logs `keydown`, `beforeinput`, `input`, the three
//   composition events and `paste` / `cut`. Its `beforeinput`
//   handler cancels the edit `__VN_TEST_BI__.cancel` names (an inputType, and
//   optionally the data).
// - `#vm`, an `<input v-model>`, logs `input` and the composition events;
//   `#model` shows the model, which runtime-dom leaves alone while an IME
//   composes.
//
// `__VN_TEST_BI__.events` is one line per event a handler saw:
// `<type>@<where>|<what it read>`. `assign(sel, value)` sets a field's value
// from script.
import { createApp, h, ref, vModelText, withDirectives } from "vue-native";

type Edit = Event & { inputType: string; data: string | null; isComposing: boolean };

const events: string[] = [];
const cancel: { type: string | null; data?: string | null } = { type: null };
const model = ref("");

const value = (e: Event) => JSON.stringify((e.target as HTMLTextAreaElement).value);

const log = (where: string) => (e: Event) => {
  switch (e.type) {
    case "keydown":
    case "keyup":
      events.push(`${e.type}@${where}|${(e as KeyboardEvent).key}`);
      break;
    case "beforeinput":
    case "input": {
      const ie = e as Edit;
      events.push(
        `${e.type}@${where}|${ie.inputType} data=${JSON.stringify(ie.data)} composing=${ie.isComposing} ` +
          `cancelable=${e.cancelable} v=${value(e)}`,
      );
      if (e.type === "beforeinput" && cancel.type === ie.inputType && (cancel.data === undefined || cancel.data === ie.data))
        e.preventDefault();
      break;
    }
    case "paste":
    case "cut":
      events.push(`${e.type}@${where}`);
      break;
    default:
      events.push(`${e.type}@${where}|data=${JSON.stringify((e as CompositionEvent).data)} v=${value(e)}`);
  }
};

// No `keyup`: devtools delivers a key's keyup in the same gpui update as its
// keydown, so it lands before the `input` the widget defers to the end of that
// update. A physical key's keyup is a later platform message, after `input`.
const all = (where: string) => {
  const on = log(where);
  return {
    onKeydown: on,
    onBeforeinput: on,
    onInput: on,
    onCompositionstart: on,
    onCompositionupdate: on,
    onCompositionend: on,
    onPaste: on,
    onCut: on,
  };
};

(globalThis as unknown as { __VN_TEST_BI__: unknown }).__VN_TEST_BI__ = {
  events,
  cancel,
  model,
  assign(sel: string, v: string) {
    (document.querySelector(sel) as HTMLTextAreaElement).value = v;
  },
};

createApp({
  setup() {
    return () =>
      h("div", { id: "root", style: { padding: "12px", display: "flex", flexDirection: "column", gap: "8px", background: "#ffffff" } }, [
        h("textarea", { id: "ta", rows: 2, style: { width: "300px", fontSize: "14px" }, ...all("ta") }),
        withDirectives(
          h("input", {
            id: "vm",
            style: { width: "300px", fontSize: "14px" },
            "onUpdate:modelValue": (v: string) => (model.value = v),
            onInput: log("vm"),
            onCompositionstart: log("vm"),
            onCompositionend: log("vm"),
          }),
          [[vModelText, model.value]],
        ),
        h("div", { id: "model" }, `model=${model.value}`),
      ]);
  },
}).mount();
