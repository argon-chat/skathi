// A fixture for packages/vn-test/specs/lightbox.spec.ts: a media viewer in its
// own window, the way Telegram Desktop's is (docs/MULTIWINDOW.md §7.9).
//
// The main window is a strip of thumbnails, a text field that holds the focus,
// and `#control`, a big picture drawn small (what the residency sweep reduces).
// The viewer is a `<VnWindow>` created hidden — `visible` false, `cover:
// "monitor"`, `taskbar: false`, `fullResolution` — so a thumbnail's click is a
// SHOW: the window exists and has painted before anybody asks for it. Its
// content is `#lb`, the dim, with the thumbnail under the full picture
// (`#lb-thumb`, `#lb-full`) so the first frame has something to show while the
// full one loads. Esc or a click on the dim closes it, the arrows step.
//
// `#pin` is a second, small viewer-like window (no cover) that shows one big
// picture far below its size, for the spec's residency check without a
// monitor-sized window on screen.
//
// `__VN_TEST_LB__` holds the state the spec reads and writes: `items` (thumb
// and full URLs), `events` (show/hide with the host's `firstFrameMs` and this
// realm's `jsMs` since the click), `loads` (the full picture's load, ms since
// the click), `keys`, and the source of `#control` / `#pin-img`.
//
// Render functions rather than an SFC, for the reason fixtures/layout gives.
import { createApp, h, nextTick, reactive, ref, VnWindow } from "vue-native";

interface Item {
  thumb: string;
  full: string;
}

interface Shown {
  firstFrameMs: number;
  width: number;
  height: number;
}

const S = reactive({
  items: [] as Item[],
  open: false,
  shown: false,
  index: 0,
  t0: 0,
  windowId: 0,
  events: [] as { kind: string; index: number; firstFrameMs?: number; jsMs: number; width?: number; height?: number }[],
  loads: [] as { index: number; ms: number }[],
  keys: [] as string[],
  closes: [] as string[],
  controlSrc: "",
  pinSrc: "",
  pinOpen: false,
  pinWindowId: 0,
});

const lb = ref<HTMLElement | null>(null);
const viewer = ref<{ mapFromMain(el: unknown): { x: number; y: number; width: number; height: number } | null } | null>(null);
const thumbs: HTMLElement[] = [];

function open(i: number) {
  S.t0 = performance.now();
  S.index = i;
  S.open = true;
  S.shown = true;
  void nextTick(() => lb.value?.focus());
}

function close(how: string) {
  S.closes.push(how);
  S.open = false;
  S.shown = false;
}

function step(by: number) {
  const n = S.items.length;
  if (n > 0) S.index = (S.index + by + n) % n;
}

function onKeydown(e: KeyboardEvent) {
  S.keys.push(e.key);
  if (e.key === "Escape") {
    e.preventDefault();
    close("escape");
  } else if (e.key === "ArrowLeft") {
    e.preventDefault();
    step(-1);
  } else if (e.key === "ArrowRight") {
    e.preventDefault();
    step(1);
  }
}

const fill = { position: "absolute", left: "0", top: "0", width: "100%", height: "100%", objectFit: "contain" };

function lightbox() {
  const item = S.items[S.index];
  if (!S.open || !item) return null;
  const index = S.index;
  return h(
    "div",
    {
      id: "lb",
      ref: lb,
      tabindex: 0,
      style: { position: "absolute", left: "0", top: "0", right: "0", bottom: "0", background: "rgba(0, 0, 0, 0.85)", outline: "none" },
      onKeydown,
      onClick: (e: MouseEvent) => {
        if (e.target === e.currentTarget) close("dim");
      },
    },
    [
      h("div", { id: "lb-stage", style: { position: "absolute", left: "5%", top: "5%", width: "90%", height: "90%" } }, [
        h("img", { id: "lb-thumb", src: item.thumb, style: fill }),
        h("img", {
          id: "lb-full",
          key: `full-${index}`,
          src: item.full,
          style: fill,
          onLoad: () => S.loads.push({ index, ms: performance.now() - S.t0 }),
        }),
      ]),
      h("div", { id: "lb-counter", style: { position: "absolute", left: "16px", bottom: "16px", color: "#fff" } }, `${index + 1} / ${S.items.length}`),
    ],
  );
}

createApp({
  setup() {
    const onShow = (e: Shown) =>
      S.events.push({ kind: "show", index: S.index, firstFrameMs: e.firstFrameMs, jsMs: performance.now() - S.t0, width: e.width, height: e.height });
    const onHide = () => S.events.push({ kind: "hide", index: S.index, jsMs: performance.now() - S.t0 });
    return () =>
      h("div", { id: "app", style: { padding: "12px", display: "flex", flexDirection: "column", gap: "8px" } }, [
        h("input", { id: "composer", value: "draft" }),
        h(
          "div",
          { id: "thumbs", style: { display: "flex", gap: "6px", flexWrap: "wrap" } },
          S.items.map((it, i) =>
            h("img", {
              id: `thumb-${i}`,
              class: "thumb",
              src: it.thumb,
              ref: (el: unknown) => {
                if (el) thumbs[i] = el as HTMLElement;
              },
              style: { width: "40px", height: "40px", objectFit: "cover", cursor: "pointer" },
              onClick: () => open(i),
            }),
          ),
        ),
        S.controlSrc ? h("img", { id: "control", src: S.controlSrc, style: { width: "40px", height: "30px" } }) : null,
        h(
          VnWindow,
          {
            ref: viewer,
            title: "Lightbox",
            visible: S.shown,
            cover: "monitor",
            taskbar: false,
            fullResolution: true,
            activate: true,
            chrome: "hidden",
            resizable: false,
            backdrop: "transparent",
            onOpen: (id: number) => (S.windowId = id),
            onShow,
            onHide,
          },
          { default: lightbox },
        ),
        h(
          VnWindow,
          {
            title: "Pin",
            visible: S.pinOpen,
            bounds: { width: 240, height: 180 },
            taskbar: false,
            fullResolution: true,
            resizable: false,
            backdrop: "none",
            onOpen: (id: number) => (S.pinWindowId = id),
          },
          {
            default: () =>
              S.pinOpen && S.pinSrc ? h("img", { id: "pin-img", src: S.pinSrc, style: { width: "200px", height: "150px" } }) : null,
          },
        ),
      ]);
  },
}).mount();

(globalThis as Record<string, unknown>).__VN_TEST_LB__ = {
  S,
  open,
  close,
  /** Where thumbnail `i` falls in the viewer, through `mapFromMain`. */
  mapThumb: (i: number) => viewer.value?.mapFromMain(thumbs[i]) ?? null,
};
