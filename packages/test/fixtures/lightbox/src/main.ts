import {
  createApp,
  defineComponent,
  h,
  nextTick,
  onBeforeUnmount,
  onMounted,
  reactive,
  ref,
  Teleport,
  SkWindow,
} from "@skathi-native/runtime";

const dom = document;

interface Item {
  thumb: string;
  full: string;
  messageId?: string;
  video?: string;
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
  zoom: 1,
  panX: 0,
  panY: 0,
  wheels: [] as { ctrl: boolean; x: number; y: number; dy: number }[],
  /**
   * Each zoom about a point: the point, the frame it was worked out in, and the pan before and
   * after the clamp.
   */
  zoomLog: [] as Record<string, unknown>[],
  menu: null as { x: number; y: number } | null,
  actions: [] as string[],
  dismissals: [] as string[],
  dimClicks: 0,
  readBack: "",
  inlineOn: false,
  inlineSrc: "",
  inlineEvents: [] as string[],
  inlineDetached: false,
});

const lb = ref<HTMLElement | null>(null);
const pic = ref<HTMLElement | null>(null);
const viewer = ref<{ mapFromMain(el: unknown): { x: number; y: number; width: number; height: number } | null } | null>(
  null,
);
const thumbs: HTMLElement[] = [];

function open(i: number) {
  S.t0 = performance.now();
  S.index = i;
  zoomReset();
  S.menu = null;
  S.open = true;
  S.shown = true;
  void nextTick(() => lb.value?.focus());
}

function close(how: string) {
  S.closes.push(how);
  S.menu = null;
  S.open = false;
  S.shown = false;
}

function step(by: number) {
  const n = S.items.length;
  if (n > 0) S.index = (S.index + by + n) % n;
  zoomReset();
}

function onKeydown(e: KeyboardEvent) {
  S.keys.push(e.key);
  if (e.key === "Escape") {
    e.preventDefault();
    if (S.menu) S.menu = null;
    else close("escape");
  } else if (e.key === "ArrowLeft") {
    e.preventDefault();
    step(-1);
  } else if (e.key === "ArrowRight") {
    e.preventDefault();
    step(1);
  }
}

const MAX_ZOOM = 8;
const WHEEL_RATE = 0.0015;
let drag: { x: number; y: number; px: number; py: number } | null = null;
let dragged = false;

function frame() {
  const box = pic.value?.parentElement?.getBoundingClientRect();
  const view = lb.value?.getBoundingClientRect();
  if (!box || !view || !(box.width > 0) || !(view.width > 0)) return null;
  return {
    cx: box.left + box.width / 2,
    cy: box.top + box.height / 2,
    w: box.width,
    h: box.height,
    vw: view.width,
    vh: view.height,
  };
}

function zoomReset() {
  S.zoom = 1;
  S.panX = 0;
  S.panY = 0;
  drag = null;
}

function clampPan() {
  const f = frame();
  if (!f) return;
  const axis = (pan: number, size: number, centre: number, view: number) => {
    const s = size * S.zoom;
    if (s <= view) return view / 2 - centre;
    return Math.min(s / 2 - centre, Math.max(view - centre - s / 2, pan));
  };
  S.panX = axis(S.panX, f.w, f.cx, f.vw);
  S.panY = axis(S.panY, f.h, f.cy, f.vh);
}

function zoomAt(z: number, x: number, y: number) {
  const f = frame();
  const next = Math.min(MAX_ZOOM, Math.max(1, z));
  if (!f || next === 1) return zoomReset();
  const ox = x - f.cx;
  const oy = y - f.cy;
  const k = next / S.zoom;
  S.panX = ox - (ox - S.panX) * k;
  S.panY = oy - (oy - S.panY) * k;
  const unclamped = [S.panX, S.panY];
  S.zoom = next;
  clampPan();
  S.zoomLog.push({ x, y, ...f, zoom: next, unclamped, pan: [S.panX, S.panY] });
}

function onWheel(e: WheelEvent) {
  S.wheels.push({ ctrl: e.ctrlKey, x: e.clientX, y: e.clientY, dy: e.deltaY });
  if (!e.ctrlKey) return;
  e.preventDefault();
  zoomAt(S.zoom * Math.exp(-e.deltaY * WHEEL_RATE), e.clientX, e.clientY);
}

function onPress(e: MouseEvent) {
  dragged = false;
  if (S.zoom <= 1 || e.button !== 0) return;
  e.preventDefault();
  drag = { x: e.clientX, y: e.clientY, px: S.panX, py: S.panY };
}

function onMove(e: MouseEvent) {
  if (!drag) return;
  const dx = e.clientX - drag.x;
  const dy = e.clientY - drag.y;
  if (Math.abs(dx) > 4 || Math.abs(dy) > 4) dragged = true;
  S.panX = drag.px + dx;
  S.panY = drag.py + dy;
  clampPan();
}

function onRelease() {
  drag = null;
}

/**
 * A click on the dim closes — unless it ends a drag. A click while the menu is up never gets here:
 * the menu's layer took the dim's pointer events away.
 */
function onDimClick(e: MouseEvent) {
  if (e.target !== e.currentTarget) return;
  S.dimClicks++;
  if (dragged) {
    dragged = false;
    return;
  }
  close("dim");
}

function onContextMenu(e: MouseEvent) {
  e.preventDefault();
  S.menu = { x: e.clientX, y: e.clientY };
}

function goToMessage() {
  const item = S.items[S.index];
  S.menu = null;
  S.actions.push(`goto:${item?.messageId ?? S.index}`);
  close("goto");
}

async function copyPicture() {
  const item = S.items[S.index];
  S.menu = null;
  if (!item) return;
  try {
    const blob = await (await fetch!(item.full)).blob();
    await navigator.clipboard.write([new ClipboardItem({ "image/png": blob })]);
    S.actions.push(`copy:ok:${blob.size}`);
  } catch (e) {
    S.actions.push(`copy:error:${(e as Error).name}:${(e as Error).message}`);
  }
}

async function savePicture() {
  const item = S.items[S.index];
  S.menu = null;
  if (!item) return;
  try {
    const handle = await (
      globalThis as unknown as { showSaveFilePicker(o: unknown): Promise<FileSystemFileHandle> }
    ).showSaveFilePicker({
      suggestedName: `picture-${S.index}.png`,
      types: [{ description: "PNG image", accept: { "image/png": [".png"] } }],
    });
    const blob = await (await fetch!(item.full)).blob();
    const out = await handle.createWritable();
    await out.write(blob);
    await out.close();
    S.actions.push(`save:ok:${handle.name}:${blob.size}`);
  } catch (e) {
    S.actions.push(`save:error:${(e as Error).name}:${(e as Error).message}`);
  }
}

const menuItem = (id: string, label: string, run: () => void) =>
  h(
    "div",
    {
      id,
      class: "lb-menu-item",
      style: { padding: "6px 12px", cursor: "pointer", color: "#fff" },
      onClick: (e: MouseEvent) => {
        e.stopPropagation();
        run();
      },
    },
    label,
  );

const Menu = defineComponent({
  props: { x: { type: Number, required: true }, y: { type: Number, required: true } },
  setup(props) {
    const el = ref<HTMLElement | null>(null);
    let doc: Document | null = null;
    let before = "";
    let timer: ReturnType<typeof setTimeout> | undefined;
    const onDown = (e: Event) => {
      if (el.value && !el.value.contains(e.target as Node)) {
        S.dismissals.push("outside");
        S.menu = null;
      }
    };
    onMounted(() => {
      doc = el.value!.ownerDocument;
      before = doc.body.style.pointerEvents;
      doc.body.style.pointerEvents = "none";
      timer = setTimeout(() => doc?.addEventListener("pointerdown", onDown), 0);
    });
    onBeforeUnmount(() => {
      clearTimeout(timer);
      doc?.removeEventListener("pointerdown", onDown);
      if (doc) doc.body.style.pointerEvents = before;
    });
    return () =>
      h(
        "div",
        {
          id: "lb-menu",
          ref: el,
          style: {
            position: "fixed",
            left: `${props.x}px`,
            top: `${props.y}px`,
            minWidth: "160px",
            background: "#222",
            border: "1px solid #444",
            borderRadius: "6px",
            padding: "4px 0",
            zIndex: "10000",
            pointerEvents: "auto",
          },
        },
        [
          menuItem("lb-menu-goto", "Go to message", goToMessage),
          menuItem("lb-menu-copy", "Copy", () => void copyPicture()),
          menuItem("lb-menu-save", "Save As...", () => void savePicture()),
        ],
      );
  },
});

function menu() {
  if (!S.menu) return null;
  return h(Teleport, { to: "body" }, [h(Menu, { x: S.menu.x, y: S.menu.y })]);
}

const hint = () =>
  h(Teleport, { to: "body" }, [
    h("div", { id: "lb-hint", style: { position: "fixed", right: "16px", top: "16px", color: "#fff" } }, "Esc"),
  ]);

const fill = { position: "absolute", left: "0", top: "0", width: "100%", height: "100%", objectFit: "contain" };

function lightbox() {
  const item = S.items[S.index];
  if (!S.open || !item) return null;
  const index = S.index;
  const content = item.video
    ? h("video", { id: "lb-video", src: item.video, autoplay: true, loop: true, style: fill })
    : h(
        "div",
        {
          id: "lb-pic",
          ref: pic,
          style: {
            ...fill,
            transform: `translate(${S.panX}px, ${S.panY}px) scale(${S.zoom})`,
            cursor: S.zoom > 1 ? "grab" : "auto",
          },
          onMousedown: onPress,
          onContextmenu: onContextMenu,
        },
        [
          h("img", { id: "lb-thumb", src: item.thumb, style: fill }),
          h("img", {
            id: "lb-full",
            key: `full-${index}`,
            src: item.full,
            style: fill,
            onLoad: () => S.loads.push({ index, ms: performance.now() - S.t0 }),
          }),
        ],
      );
  return h(
    "div",
    {
      id: "lb",
      ref: lb,
      tabindex: 0,
      style: {
        position: "absolute",
        left: "0",
        top: "0",
        right: "0",
        bottom: "0",
        background: "rgba(0, 0, 0, 0.85)",
        outline: "none",
        overflow: "hidden",
      },
      onKeydown,
      onClick: onDimClick,
      onMousedown: () => (dragged = false),
      onWheel,
      onMousemove: onMove,
      onMouseup: onRelease,
    },
    [
      h(
        "div",
        { id: "lb-stage", style: { position: "absolute", left: "5%", top: "5%", width: "90%", height: "90%" } },
        [content],
      ),
      h(
        "div",
        { id: "lb-counter", style: { position: "absolute", left: "16px", bottom: "16px", color: "#fff" } },
        `${index + 1} / ${S.items.length}`,
      ),
      hint(),
      menu(),
    ],
  );
}

let inlineEl: HTMLVideoElement | null = null;
let inlineParent: Node | null = null;

/**
 * Muted and played through its properties BEFORE its `src`, as Argon's VideoPlayer does in
 * `onMounted`.
 */
const InlineVideo = defineComponent({
  setup() {
    const v = ref<HTMLVideoElement | null>(null);
    onMounted(() => {
      const el = v.value!;
      inlineEl = el;
      el.muted = true;
      el.src = S.inlineSrc;
      void el.play();
    });
    const log = (name: string) => () => S.inlineEvents.push(name);
    return () =>
      h("video", {
        id: "inline-video",
        ref: v,
        loop: true,
        playsinline: true,
        style: { width: "80px", height: "45px" },
        onPlay: log("play"),
        onPause: log("pause"),
        onPlaying: log("playing"),
      });
  },
});

createApp({
  setup() {
    const onShow = (e: Shown) =>
      S.events.push({
        kind: "show",
        index: S.index,
        firstFrameMs: e.firstFrameMs,
        jsMs: performance.now() - S.t0,
        width: e.width,
        height: e.height,
      });
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
        h("div", { id: "inline-box" }, S.inlineOn ? [h(InlineVideo)] : []),
        h(
          SkWindow,
          {
            ref: viewer,
            class: "lb-window",
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
          SkWindow,
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
              S.pinOpen && S.pinSrc
                ? h("img", { id: "pin-img", src: S.pinSrc, style: { width: "200px", height: "150px" } })
                : null,
          },
        ),
      ]);
  },
}).mount();

(globalThis as Record<string, unknown>).__SK_TEST_LB__ = {
  S,
  open,
  close,
  mapThumb: (i: number) => viewer.value?.mapFromMain(thumbs[i]) ?? null,
  setZoom(z: number) {
    S.zoom = z;
    S.panX = 0;
    S.panY = 0;
    clampPan();
  },
  picRect() {
    const r = pic.value?.getBoundingClientRect();
    const v = lb.value?.getBoundingClientRect();
    return r && v ? { left: r.left, top: r.top, width: r.width, height: r.height, vw: v.width, vh: v.height } : null;
  },
  menuInfo() {
    const m = dom.getElementById("lb-menu");
    if (!m) return null;
    const r = m.getBoundingClientRect();
    const doc = m.ownerDocument;
    return {
      left: r.left,
      top: r.top,
      inWindow: !!m.closest(".sk-window-root"),
      sameDocument: doc === dom,
      docWidth: doc.documentElement.clientWidth,
      bodyIsRoot: doc.body === doc.documentElement,
      mainWidth: dom.documentElement.clientWidth,
      rootClass: doc.body.className,
      bodyPointerEvents: doc.body.style.pointerEvents,
      hintInWindow: !!dom.getElementById("lb-hint")?.closest(".sk-window-root"),
    };
  },
  detachInline() {
    if (!inlineEl) return false;
    inlineParent = inlineEl.parentNode;
    inlineEl.remove();
    S.inlineDetached = true;
    return true;
  },
  attachInline() {
    if (!inlineEl || !inlineParent) return false;
    inlineParent.appendChild(inlineEl);
    S.inlineDetached = false;
    return true;
  },
  inlineState() {
    const el = inlineEl;
    return el ? { muted: el.muted, paused: el.paused, src: el.src } : null;
  },
  /** Reads the clipboard back the way a paste would, into `S.readBack`. */
  readBack() {
    S.readBack = "";
    navigator.clipboard.read().then(
      async ([item]) => {
        const png = new Uint8Array(await (await item!.getType("image/png")).arrayBuffer());
        S.readBack = `${item!.types.join(",")}:${png.length}:${png[0]}`;
      },
      (e: Error) => (S.readBack = `error:${e.name}:${e.message}`),
    );
  },
};
