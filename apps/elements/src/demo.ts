// The recorder's handle, `globalThis.__demo`: which page shows, and the drawn cursor. Input itself
// comes from devtools (`mouse`, `type`, `key`), so the real mouse and keyboard are never touched.
import { reactive } from "vue";

export const nav = reactive({ id: "lottie", key: 0 });

export const pointer = reactive({ x: 640, y: 400, shown: false, dur: 0, pressed: false, ripple: 0, key: "" });

const sleep = (ms: number) => new Promise<void>((r) => setTimeout(r, ms));

export function installDemo(): void {
  (globalThis as Record<string, unknown>).__demo = {
    show(id: string) {
      pointer.shown = false;
      pointer.key = "";
      nav.id = id;
      nav.key++;
      return "ok";
    },
    rect(selector: string) {
      const el = document.querySelector(selector);
      if (!el) return null;
      const r = el.getBoundingClientRect();
      return JSON.stringify({ x: r.x, y: r.y, w: r.width, h: r.height });
    },
    cursor: {
      async show(x: number, y: number) {
        pointer.dur = 0;
        pointer.x = x;
        pointer.y = y;
        await sleep(32);
        pointer.shown = true;
      },
      move(x: number, y: number, ms: number) {
        pointer.dur = ms;
        pointer.x = x;
        pointer.y = y;
      },
      press(down: boolean) {
        pointer.pressed = down;
        if (down) pointer.ripple++;
      },
      key(label: string) {
        pointer.key = label;
      },
      hide() {
        pointer.shown = false;
      },
    },
  };
}
