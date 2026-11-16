import { reactive, shallowRef } from "vue";

export const demo = reactive({ enabled: false, running: "" });

/** The drawn pointer, in the MAIN window's client CSS px. Each window's layer draws it at its own offset. */
export const pointer = reactive({
  x: 0,
  y: 0,
  shown: false,
  dur: 0,
  pressed: false,
  ripple: 0,
  key: "",
});

export type Point = { x: number; y: number };

export const mainLayer = shallowRef<Element | null>(null);

interface PopLayer {
  layer: Element;
  root: Element | null;
  /** Where the main window's client origin is in this window, or null while it is not open. */
  offset(): Point | null;
}
const popLayers = new Set<PopLayer>();

export function registerPopLayer(layer: PopLayer): () => void {
  popLayers.add(layer);
  return () => popLayers.delete(layer);
}

export const sleep = (ms: number) => new Promise<void>((r) => setTimeout(r, ms));

/** The center of `el` in main-window coordinates, through whichever window paints it. */
export function centerOf(el: Element, dx = 0, dy = 0): Point | null {
  const r = el.getBoundingClientRect();
  const root = el.closest(".sk-window-root");
  if (root) {
    for (const pop of popLayers) {
      if (pop.root !== root) continue;
      const off = pop.offset();
      if (!off) break;
      const lr = pop.layer.getBoundingClientRect();
      return { x: r.x - lr.x + r.width / 2 + dx - off.x, y: r.y - lr.y + r.height / 2 + dy - off.y };
    }
  }
  const base = mainLayer.value?.getBoundingClientRect() ?? { x: 0, y: 0 };
  return { x: r.x - base.x + r.width / 2 + dx, y: r.y - base.y + r.height / 2 + dy };
}

export async function showAt(p: Point): Promise<void> {
  pointer.dur = 0;
  pointer.x = p.x;
  pointer.y = p.y;
  await sleep(32);
  pointer.shown = true;
  await sleep(260);
}

export async function hide(): Promise<void> {
  pointer.shown = false;
  await sleep(220);
}

export async function moveTo(
  target: Element | Point,
  opts: { ms?: number; dx?: number; dy?: number } = {},
): Promise<void> {
  const p = "getBoundingClientRect" in target ? centerOf(target, opts.dx, opts.dy) : target;
  if (!p) return;
  if (!pointer.shown) return showAt(p);
  const dist = Math.hypot(p.x - pointer.x, p.y - pointer.y);
  const ms = opts.ms ?? Math.round(Math.min(1000, Math.max(420, 360 + dist * 0.6)));
  pointer.dur = ms;
  pointer.x = p.x;
  pointer.y = p.y;
  await sleep(ms + 60);
}

/** Moves to `el`, shows the press, then runs the real action: `el.click()` unless `action` is given. */
export async function click(el: Element, action?: () => void): Promise<void> {
  await moveTo(el);
  el.classList.add("demo-hover");
  await sleep(160);
  pointer.pressed = true;
  pointer.ripple++;
  el.classList.add("demo-press");
  await sleep(120);
  if (action) action();
  else (el as HTMLElement).click();
  pointer.pressed = false;
  await sleep(140);
  el.classList.remove("demo-press");
  el.classList.remove("demo-hover");
}

export async function pressKey(label: string, action: () => void): Promise<void> {
  pointer.key = label;
  await sleep(520);
  action();
  await sleep(380);
  pointer.key = "";
}
