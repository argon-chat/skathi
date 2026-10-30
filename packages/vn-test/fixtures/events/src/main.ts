// A fixture for packages/vn-test/specs/events.spec.ts (m26 fixes,
// docs/RUNTIME.md §5.6): the events the Argon client binds that the host never
// delivered.
//
// - `#trigger` is a reka-style context-menu trigger: `contextmenu` is cancelled
//   and opens `#menu` at the pointer. It also listens for `mousedown`,
//   `mouseup` and `click`, and so does `#inner` inside it (press and release
//   only), which is what pins ONE event per press and the DOM order.
// - `#plain` leaves its `contextmenu` uncancelled (the E-1 advisory).
// - `#pad` and `#pad2` are focusable: the Menu key and Shift+F10 fire
//   `contextmenu` at them. `#pad2` cancels the Menu key's `keyup`, which
//   cancels the `contextmenu` too (Chromium).
// - `#key` takes a keyboard click (Enter): `detail` 0.
// - `#ok` (a data: PNG), `#bad` (a file that is not there) and `#blob` (a
//   blob: URL over bytes in memory) answer `load` / `error`; `#ok`'s
//   `onload` handler attribute is set from `onMounted`.
//
// `__VN_TEST_EVENTS__.events` is one line per event a handler saw:
// `<type>@<listener>|<what it read>`.
import { createApp, h, onMounted, ref } from "vue-native";

const events: string[] = [];
const menu = ref<{ x: number; y: number } | null>(null);
(globalThis as unknown as { __VN_TEST_EVENTS__: unknown }).__VN_TEST_EVENTS__ = { events };

/** A mirror node's `id` PROPERTY is its host id; the attribute is the name. */
const idOf = (t: EventTarget | null) => (t as { getAttribute?(n: string): string | null } | null)?.getAttribute?.("id") ?? "?";

const mouse = (where: string, cancel = false) => (e: MouseEvent) => {
  events.push(
    `${e.type}@${where}|t=${idOf(e.target)} b=${e.button} bs=${e.buttons} d=${e.detail} ` +
      `x=${Math.round(e.clientX)} y=${Math.round(e.clientY)} c=${e.cancelable}`,
  );
  if (cancel) e.preventDefault();
};

const image = (where: string) => (e: Event) => {
  const img = e.target as HTMLImageElement;
  events.push(`${e.type}@${where}|bubbles=${e.bubbles} complete=${img.complete} w=${img.naturalWidth} h=${img.naturalHeight}`);
};

// An 8×6 PNG (one colour), and the same bytes behind a blob: URL.
const PNG_8x6 = "iVBORw0KGgoAAAANSUhEUgAAAAgAAAAGCAIAAABxZ0isAAAAEUlEQVR4nGNQmPAAK2IYSAkA1tFLAUEgC3gAAAAASUVORK5CYII=";
const bytes = Uint8Array.from(atob(PNG_8x6), (c) => c.charCodeAt(0));
const blobUrl = URL.createObjectURL(new Blob([bytes], { type: "image/png" }));

const box = { width: "120px", height: "48px", background: "#e8e8e8", padding: "4px" };

createApp({
  setup() {
    const ok = ref<HTMLImageElement | null>(null);
    onMounted(() => {
      // The handler attribute: the answer comes from the frame tick, after
      // this wave, so it is set in time.
      ok.value!.onload = () => events.push(`onload@ok|w=${ok.value!.naturalWidth}`);
    });
    return () =>
      h("div", { id: "root", style: { padding: "12px", display: "flex", flexWrap: "wrap", gap: "10px", background: "#ffffff" } }, [
        h(
          "div",
          {
            id: "trigger",
            style: box,
            onMousedown: mouse("trigger"),
            onMouseup: mouse("trigger"),
            onClick: mouse("trigger"),
            onContextmenu: (e: MouseEvent) => {
              mouse("trigger", true)(e);
              menu.value = { x: e.clientX, y: e.clientY };
            },
          },
          [h("span", { id: "inner", style: { background: "#c0c0ff", padding: "6px" }, onMousedown: mouse("inner"), onMouseup: mouse("inner") }, "inner")],
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
        // A <button>: only an element with an activation behaviour clicks from
        // a key (docs/RUNTIME.md §5.11).
        h("button", { id: "key", tabindex: 0, style: box, onClick: mouse("key") }, "key"),
        // Drawn in a corner rather than at the pointer, so it never covers a
        // box the spec aims at next; it says where the pointer was.
        menu.value
          ? h(
              "div",
              { id: "menu", style: { position: "absolute", right: "8px", bottom: "8px", background: "#333", color: "#fff", padding: "4px" } },
              `menu at ${Math.round(menu.value.x)},${Math.round(menu.value.y)}`,
            )
          : null,
        h("img", { id: "ok", ref: ok, src: `data:image/png;base64,${PNG_8x6}`, alt: "ok", onLoad: image("ok"), onError: image("ok") }),
        h("img", { id: "bad", src: "no-such-picture.png", alt: "bad", onLoad: image("bad"), onError: image("bad") }),
        h("img", { id: "blob", src: blobUrl, alt: "blob", onLoad: image("blob"), onError: image("blob") }),
      ]);
  },
}).mount();
