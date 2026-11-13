import { createApp, h, onMounted, ref } from "vue-native";

const events: string[] = [];
const menu = ref<{ x: number; y: number } | null>(null);
(globalThis as unknown as { __VN_TEST_EVENTS__: unknown }).__VN_TEST_EVENTS__ = { events };

const idOf = (t: unknown) => (t as { getAttribute?(n: string): string | null } | null)?.getAttribute?.("id") ?? "?";

const mouse =
  (where: string, cancel = false) =>
  (e: MouseEvent) => {
    events.push(
      `${e.type}@${where}|t=${idOf(e.target)} b=${e.button} bs=${e.buttons} d=${e.detail} ` +
        `x=${Math.round(e.clientX)} y=${Math.round(e.clientY)} c=${e.cancelable}`,
    );
    if (cancel) e.preventDefault();
  };

const image = (where: string) => (e: Event) => {
  const img = e.target as HTMLImageElement;
  events.push(
    `${e.type}@${where}|bubbles=${e.bubbles} complete=${img.complete} w=${img.naturalWidth} h=${img.naturalHeight}`,
  );
};

const PNG_8x6 = "iVBORw0KGgoAAAANSUhEUgAAAAgAAAAGCAIAAABxZ0isAAAAEUlEQVR4nGNQmPAAK2IYSAkA1tFLAUEgC3gAAAAASUVORK5CYII=";
const bytes = Uint8Array.from(atob(PNG_8x6), (c) => c.charCodeAt(0));
const blobUrl = URL.createObjectURL(new Blob([bytes], { type: "image/png" }));

const box = { width: "120px", height: "48px", background: "#e8e8e8", padding: "4px" };

createApp({
  setup() {
    const ok = ref<HTMLImageElement | null>(null);
    onMounted(() => {
      // The handler attribute: the answer comes from the frame tick, after this wave, so it is set
      // in time.
      ok.value!.onload = () => events.push(`onload@ok|w=${ok.value!.naturalWidth}`);
    });
    return () =>
      h(
        "div",
        {
          id: "root",
          style: { padding: "12px", display: "flex", flexWrap: "wrap", gap: "10px", background: "#ffffff" },
        },
        [
          h(
            "div",
            {
              id: "trigger",
              style: box,
              onMousedown: mouse("trigger"),
              onMouseup: mouse("trigger"),
              onClick: mouse("trigger"),
              onAuxclick: mouse("trigger"),
              onContextmenu: (e: MouseEvent) => {
                mouse("trigger", true)(e);
                menu.value = { x: e.clientX, y: e.clientY };
              },
            },
            [
              h(
                "span",
                {
                  id: "inner",
                  style: { background: "#c0c0ff", padding: "6px" },
                  onMousedown: mouse("inner"),
                  onMouseup: mouse("inner"),
                },
                "inner",
              ),
            ],
          ),
          h("div", { id: "plain", style: box, onContextmenu: mouse("plain") }, "plain"),
          h(
            "div",
            {
              id: "pad",
              tabindex: 0,
              style: box,
              onContextmenu: mouse("pad", true),
              onKeydown: (e: KeyboardEvent) => events.push(`keydown@pad|k=${e.key}`),
            },
            "pad",
          ),
          h(
            "div",
            {
              id: "pad2",
              tabindex: 0,
              style: box,
              onContextmenu: mouse("pad2", true),
              onKeyup: (e: KeyboardEvent) => {
                if (e.key === "ContextMenu") {
                  events.push("keyup@pad2|cancelled");
                  e.preventDefault();
                }
              },
            },
            "pad2",
          ),
          h("button", { id: "key", tabindex: 0, style: box, onClick: mouse("key") }, "key"),
          // Drawn in a corner rather than at the pointer, so it never covers a box the spec aims at
          // next; it says where the pointer was.
          menu.value
            ? h(
                "div",
                {
                  id: "menu",
                  style: {
                    position: "absolute",
                    right: "8px",
                    bottom: "8px",
                    background: "#333",
                    color: "#fff",
                    padding: "4px",
                  },
                },
                `menu at ${Math.round(menu.value.x)},${Math.round(menu.value.y)}`,
              )
            : null,
          h("img", {
            id: "ok",
            ref: ok,
            src: `data:image/png;base64,${PNG_8x6}`,
            alt: "ok",
            onLoad: image("ok"),
            onError: image("ok"),
          }),
          h("img", { id: "bad", src: "no-such-picture.png", alt: "bad", onLoad: image("bad"), onError: image("bad") }),
          h("img", { id: "blob", src: blobUrl, alt: "blob", onLoad: image("blob"), onError: image("blob") }),
        ],
      );
  },
}).mount();
