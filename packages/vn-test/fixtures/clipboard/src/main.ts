// A fixture for packages/vn-test/specs/clipboard.spec.ts (m26 fixes,
// docs/RUNTIME.md §5.4): `copy` / `cut` / `paste` from real keystrokes, on a
// text field and on a focusable element that is not one.
//
// It never writes the system clipboard, because the spec runs while somebody
// uses the machine:
// - every `paste` is cancelled, so the field never inserts what is there;
// - no `copy` / `cut` is cancelled, and the field has no selection, so the
//   field's own copy has nothing to write and a non-field copy copies nothing.
//   (A cancelled copy WRITES what its handler set: that path is covered by
//   vn-tree's `clipboard::tests`, against no real clipboard.)
//
// `__VN_TEST_CLIP__.events` is one line per event a handler saw:
// `<type>@<where>|<what it learned>`.
import { createApp, h } from "vue-native";

type Store = {
  types: readonly string[];
  setData(format: string, data: string): void;
  getData(format: string): string;
};
type ClipEvent = Event & { clipboardData: Store | null };

const events: string[] = [];
(globalThis as unknown as { __VN_TEST_CLIP__: unknown }).__VN_TEST_CLIP__ = { events };

const DT = (globalThis as unknown as { DataTransfer: new () => Store }).DataTransfer;
const CE = (globalThis as unknown as { ClipboardEvent: new (t: string) => ClipEvent }).ClipboardEvent;

/** What one event says about itself and its store, without touching the clipboard. */
function describe(e: ClipEvent): string {
  const d = e.clipboardData;
  if (!d) return "null";
  // Can a handler write it? Only copy / cut may (read/write); paste is read-only.
  d.setData("application/x-vn-test", "1");
  const writable = d.getData("application/x-vn-test") === "1";
  const kept = d;
  // After the event the store is disabled: no types, no data.
  setTimeout(() => events.push(`after:${e.type}|types=${kept.types.length}`), 0);
  return [
    `event=${e instanceof CE}`,
    `dt=${d instanceof DT}`,
    `bubbles=${e.bubbles}`,
    `cancelable=${e.cancelable}`,
    `writable=${writable}`,
  ].join(",");
}

const record = (where: string, cancel: boolean) => (e: Event) => {
  const ev = e as ClipEvent;
  events.push(`${ev.type}@${where}|${describe(ev)}`);
  if (cancel) ev.preventDefault();
};

document.addEventListener("paste", (e) => {
  // `getAttribute`: a mirror node's `id` property is its host node id.
  const t = (e.target as { getAttribute?(n: string): string | null } | null)?.getAttribute?.("id") ?? "?";
  events.push(`paste@document|target=${t}`);
});

createApp({
  setup() {
    return () =>
      h("div", { id: "root", style: { padding: "12px", display: "flex", flexDirection: "column", gap: "12px", background: "#ffffff" } }, [
        h("input", {
          id: "field",
          style: { width: "240px" },
          onPaste: record("field", true),
          onCopy: record("field", false),
          onCut: record("field", false),
          // Ctrl+X's keydown is cancelled: no `cut` may follow it.
          onKeydown: (e: KeyboardEvent) => {
            if (e.ctrlKey && e.key === "x") {
              events.push("keydown@field|cancelled ctrl+x");
              e.preventDefault();
            }
          },
        }),
        h("div", { id: "pad", tabindex: 0, style: { width: "240px", height: "40px", background: "#eeeeee" }, onPaste: record("pad", true), onCopy: record("pad", false) }, "pad"),
      ]);
  },
}).mount();
