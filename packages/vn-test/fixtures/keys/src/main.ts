// A fixture for packages/vn-test/specs/keys.spec.ts (m26, docs/RUNTIME.md
// §5.19): one `keydown` (and one `keyup`) per key, AT the focused element,
// bubbling from there to `document`. gpui hands a key event to every listener
// on the focus path, and a text field's editor sees it before all of them;
// each used to dispatch a DOM event of its own, so `document` heard one key
// two or three times.
//
// * `#inner` is focusable and listens for nothing; `#box` and `#wrap` around
//   it listen. A key on `#inner` is ONE event with `target === #inner`.
// * `#field` closes itself on Escape (its own `@keydown`, as a dialog's search
//   box would); `document` must still hear that Escape once, not a second
//   time from the root after the field is gone.
// * `#plain` is a field with no listener of its own.
//
// `__VN_TEST_KEYS__.log` records `<listener>:<event>:<key> t=<target id>`.
import { createApp, h, ref } from "vue-native";

type Log = { log: string[] };
const out: Log = { log: [] };
(globalThis as unknown as { __VN_TEST_KEYS__: Log }).__VN_TEST_KEYS__ = out;

const idOf = (e: Event) => ((e.target as { id?: string } | null)?.id || (e.target === document ? "document" : "?"));
const note = (who: string) => (e: KeyboardEvent) => out.log.push(`${who}:${e.type}:${e.key} t=${idOf(e)}`);

document.addEventListener("keydown", note("document"));
document.addEventListener("keyup", note("document"));

const fieldOpen = ref(true);

const App = {
  render: () =>
    h("div", { id: "page", style: { display: "flex", flexDirection: "column", gap: "12px", padding: "16px", background: "#fff", height: "100%" } }, [
      h("div", { id: "wrap", onKeydown: note("wrap"), style: { padding: "8px", background: "#eef" } }, [
        h("div", { id: "box", onKeydown: note("box"), style: { padding: "8px", background: "#dde" } }, [
          h("div", { id: "inner", tabindex: 0, style: { padding: "8px", background: "#ccd" } }, "inner"),
        ]),
      ]),
      fieldOpen.value
        ? h("input", {
            id: "field",
            onKeydown: (e: KeyboardEvent) => {
              note("field")(e);
              if (e.key === "Escape") fieldOpen.value = false;
            },
            style: { width: "200px" },
          })
        : h("button", { id: "reopen", onClick: () => (fieldOpen.value = true) }, "reopen"),
      h("input", { id: "plain", style: { width: "200px" } }),
      // An emoji suggestion list driven from the field: the arrow keys move the
      // pick, and the handler asks where the focus is and keeps it on the field.
      // It runs inside the field editor's own update, which a read of that
      // editor used to answer with a native panic.
      h("input", {
        id: "suggest",
        onKeydown: (e: KeyboardEvent) => {
          if (e.key !== "ArrowDown" && e.key !== "ArrowUp" && e.key !== "Enter") return;
          e.preventDefault();
          const active = (document.activeElement as { id?: string } | null)?.id ?? "none";
          (e.target as HTMLInputElement).focus();
          out.log.push(`suggest:keydown:${e.key} active=${active}`);
        },
        style: { width: "200px" },
      }),
    ]),
};

createApp(App).mount("#app");
