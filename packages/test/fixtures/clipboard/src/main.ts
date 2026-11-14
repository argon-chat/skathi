import { createApp, h } from "@skathi-native/runtime";

const dom = document;

type Store = {
  types: readonly string[];
  setData(format: string, data: string): void;
  getData(format: string): string;
};
type ClipEvent = Event & { clipboardData: Store | null };

const events: string[] = [];
(globalThis as unknown as { __SK_TEST_CLIP__: unknown }).__SK_TEST_CLIP__ = { events };

const DT = (globalThis as unknown as { DataTransfer: new () => Store }).DataTransfer;
const CE = (globalThis as unknown as { ClipboardEvent: new (t: string) => ClipEvent }).ClipboardEvent;

function describe(e: ClipEvent): string {
  const d = e.clipboardData;
  if (!d) return "null";
  d.setData("application/x-sk-test", "1");
  const writable = d.getData("application/x-sk-test") === "1";
  const kept = d;
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

dom.addEventListener("paste", (e) => {
  const t = (e.target as { getAttribute?(n: string): string | null } | null)?.getAttribute?.("id") ?? "?";
  events.push(`paste@document|target=${t}`);
});

createApp({
  setup() {
    return () =>
      h(
        "div",
        {
          id: "root",
          style: { padding: "12px", display: "flex", flexDirection: "column", gap: "12px", background: "#ffffff" },
        },
        [
          h("input", {
            id: "field",
            style: { width: "240px" },
            onPaste: record("field", true),
            onCopy: record("field", false),
            onCut: record("field", false),
            onKeydown: (e: KeyboardEvent) => {
              if (e.ctrlKey && e.key === "x") {
                events.push("keydown@field|cancelled ctrl+x");
                e.preventDefault();
              }
            },
          }),
          h(
            "div",
            {
              id: "pad",
              tabindex: 0,
              style: { width: "240px", height: "40px", background: "#eeeeee" },
              onPaste: record("pad", true),
              onCopy: record("pad", false),
            },
            "pad",
          ),
        ],
      );
  },
}).mount();
