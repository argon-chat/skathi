import { appWindow } from "skathi:app";
import { background, draft } from "../store";
import { centerOf, click, hide, mainLayer, moveTo, pressKey, showAt, sleep, type Point } from "./cursor";

const q = (name: string) => document.querySelector(`[data-demo="${name}"]`);

async function el(name: string, timeoutMs = 4000): Promise<Element> {
  const deadline = Date.now() + timeoutMs;
  for (;;) {
    const found = q(name);
    if (found && centerOf(found)) return found;
    if (Date.now() > deadline) throw new Error(`[data-demo="${name}"] did not appear`);
    await sleep(50);
  }
}

async function until(test: () => boolean, timeoutMs: number): Promise<void> {
  const deadline = Date.now() + timeoutMs;
  while (!test() && Date.now() < deadline) await sleep(50);
}

function contentPoint(fx: number, fy: number): Point {
  const r = mainLayer.value?.getBoundingClientRect();
  return { x: (r?.width ?? innerWidth) * fx, y: (r?.height ?? innerHeight) * fy };
}

async function scrollBy(list: Element, dy: number, ms: number): Promise<void> {
  const from = list.scrollTop;
  const start = performance.now();
  for (;;) {
    const t = Math.min(1, (performance.now() - start) / ms);
    const eased = t < 0.5 ? 4 * t * t * t : 1 - Math.pow(-2 * t + 2, 3) / 2;
    list.scrollTop = from + dy * eased;
    list.dispatchEvent(new Event("scroll"));
    if (t >= 1) return;
    await sleep(16);
  }
}

/** The X of a pop-out, as the OS caption would close it: the window goes, and the component stays mounted. */
function closePopout(root: Element): void {
  const close = (globalThis as { __sk_window_close?: (id: number) => void }).__sk_window_close;
  close?.((root as unknown as { __skId: number }).__skId);
}

function escapeKey(): void {
  if (__SK_WINDOW__?.escapeFullscreen?.()) return;
  void document.exitFullscreen();
}

export const scripts: Record<string, () => Promise<void>> = {
  async "windows/state-verbs"() {
    await showAt(contentPoint(0.62, 0.62));
    await sleep(300);
    await click(await el("maximize"));
    await sleep(1500);
    await click(await el("restore"));
    await sleep(1200);
    await click(await el("minimize"));
    await sleep(1700);
    appWindow.restore();
    await sleep(1500);
  },

  async "windows/always-on-top"() {
    await showAt(contentPoint(0.6, 0.6));
    await click(await el("compact"));
    await hide();
    await sleep(900);
    await showAt(centerOf(await el("note-text"))!);
    await sleep(200);
    await click(await el("reference"));
    await sleep(1500);
    await click(await el("pin"));
    await sleep(1400);
    await click(await el("reference"));
    await sleep(700);
    await click(await el("reference"));
    await sleep(2200);
  },

  async "windows/theme-backdrop"() {
    await showAt(contentPoint(0.55, 0.7));
    const first = appWindow.theme === "light" ? "dark" : "light";
    const second = first === "light" ? "dark" : "light";
    for (const name of [`theme-${first}`, `theme-${second}`, "backdrop-mica", "backdrop-none", "backdrop-acrylic"]) {
      await click(await el(name));
      await sleep(1000);
    }
    await click(await el("theme-system"));
    await sleep(900);
  },

  async "windows/fullscreen"() {
    await showAt(contentPoint(0.5, 0.75));
    await click(await el("fullscreen"));
    await until(() => appWindow.isFullscreen(), 3000);
    await sleep(1600);
    await pressKey("Esc", escapeKey);
    await until(() => !appWindow.isFullscreen(), 3000);
    await sleep(1200);
    await click(await el("fullscreen"));
    await until(() => appWindow.isFullscreen(), 3000);
    await sleep(1500);
    await click(await el("fullscreen"));
    await until(() => !appWindow.isFullscreen(), 3000);
    await sleep(1000);
  },

  async "pop-out/palette"() {
    await showAt(contentPoint(0.45, 0.8));
    await click(await el("palette-toggle"));
    await sleep(900);
    for (const i of [1, 3, 4]) {
      await click(await el(`swatch-${i}`));
      await sleep(900);
    }
    await click(await el("palette-toggle"));
    await sleep(800);
  },

  async "pop-out/follow"() {
    await showAt(contentPoint(0.7, 0.7));
    await click(await el("row-2"));
    await sleep(1000);
    const list = await el("file-list");
    await moveTo(list, { dy: 80 });
    await scrollBy(list, 150, 1300);
    await sleep(600);
    await scrollBy(list, 260, 1200);
    await sleep(900);
    await scrollBy(list, -410, 1500);
    await sleep(500);
    await click(await el("row-6"));
    await sleep(1400);
  },

  async "pop-out/overlay"() {
    await showAt(contentPoint(0.55, 0.75));
    await click(await el("break"));
    await sleep(1300);
    for (const i of [0, 1, 2]) {
      await click(await el(`task-${i}`));
      await sleep(1000);
    }
    await until(() => !q("break-running"), 9000);
    await sleep(800);
  },

  async "pop-out/reattach"() {
    await showAt(contentPoint(0.4, 0.7));
    const root = await el("inspector-root");
    await sleep(500);
    await click(await el("opacity-up"));
    await sleep(500);
    await click(await el("inspector-close"), () => closePopout(root));
    await until(() => !!q("popout-again"), 3000);
    await sleep(1400);
    await click(await el("opacity-up"));
    await sleep(700);
    await click(await el("popout-again"));
    await sleep(1600);
  },

  async "title-bar/drag-region"() {},
  async "title-bar/overlay"() {},
  async "title-bar/edge-resize"() {},

  async "closing/unsaved-changes"() {
    draft.text = `${draft.saved}\nLink the close-requested reference from the sheet sample.`;
  },

  async "closing/hide-and-return"() {
    background.keepRunning = true;
  },
};

type Targets = { scale: number; points: Record<string, Point> };

/** Screen points (CSS px) for the recorder's real mouse: content origin plus element centers. */
export function targets(id: string): Targets {
  const cb = appWindow.getContentBounds();
  const outer = appWindow.getBounds();
  const points: Record<string, Point> = {};
  const add = (name: string, p: Point | null) => {
    if (p) points[name] = { x: Math.round(cb.x + p.x), y: Math.round(cb.y + p.y) };
  };
  const at = (name: string, dx = 0, dy = 0) => {
    const found = q(name);
    return found ? centerOf(found, dx, dy) : null;
  };

  add("start", { x: cb.width * 0.62, y: cb.height * 0.66 });
  add("titlebar", at("titlebar-empty"));
  add("content", { x: cb.width * 0.5, y: cb.height * 0.55 });

  if (id === "title-bar/overlay") {
    const h = appWindow.chrome.titleBarHeight || 40;
    add("tab", at("tab-Canvas"));
    add("min", { x: cb.width - 115, y: h / 2 });
    add("max", { x: cb.width - 69, y: h / 2 });
    add("close", { x: cb.width - 23, y: h / 2 });
  } else {
    add("min", at("cap-min"));
    add("max", at("cap-max"));
    add("close", at("cap-close"));
  }
  if (id === "title-bar/edge-resize") {
    const border = outer.x + outer.width - (cb.x + cb.width);
    add("edge", { x: border > 3 ? cb.width + border / 2 : cb.width - 2, y: cb.height * 0.55 });
  }
  add("cancel", at("sheet-cancel"));
  return { scale: appWindow.scaleFactor, points };
}
