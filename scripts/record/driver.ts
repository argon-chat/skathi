// Drives a demo app for a clip: synthetic input through devtools (`mouse`, `type`, `key`) and the
// pointer the app draws itself, so the real mouse and keyboard are never touched.
//
// The app provides `globalThis.__demo` (apps/elements/src/demo.ts):
//   show(id)          mount the clip's page
//   rect(selector)    JSON {x, y, w, h} in window CSS px
//   cursor.show(x, y) / move(x, y, ms) / press(down) / key(label) / hide()
import type { SkApp } from "../../packages/test/src/index";
import { sleep } from "./win32";

export type Box = { x: number; y: number; w: number; h: number };
export type Pt = { x: number; y: number };

/** CSS cubic-bezier(0.32, 0, 0.18, 1), the drawn pointer's transition, so hover follows it. */
function ease(t: number): number {
  const [x1, y1, x2, y2] = [0.32, 0, 0.18, 1];
  const bx = (s: number) => 3 * (1 - s) * (1 - s) * s * x1 + 3 * (1 - s) * s * s * x2 + s * s * s;
  const by = (s: number) => 3 * (1 - s) * (1 - s) * s * y1 + 3 * (1 - s) * s * s * y2 + s * s * s;
  let lo = 0;
  let hi = 1;
  for (let i = 0; i < 30; i++) {
    const mid = (lo + hi) / 2;
    if (bx(mid) < t) lo = mid;
    else hi = mid;
  }
  return by((lo + hi) / 2);
}

/** Waits until `t` ms after `from`. */
export async function until(from: number, t: number): Promise<void> {
  const wait = from + t - Date.now();
  if (wait > 0) await sleep(wait);
}

export class Driver {
  pos: Pt = { x: 0, y: 0 };
  private held = false;
  constructor(readonly app: SkApp) {}

  js(code: string): Promise<unknown> {
    return this.app.eval(code);
  }
  async box(selector: string): Promise<Box> {
    const raw = await this.js(`__demo.rect(${JSON.stringify(selector)})`);
    if (!raw) throw new Error(`no element ${selector}`);
    return JSON.parse(String(raw)) as Box;
  }
  async center(selector: string, dx = 0, dy = 0): Promise<Pt> {
    const b = await this.box(selector);
    return { x: Math.round(b.x + b.w / 2 + dx), y: Math.round(b.y + b.h / 2 + dy) };
  }
  mouse(kind: "move" | "down" | "up", p: Pt): Promise<unknown> {
    return this.app.send("mouse", { x: p.x, y: p.y, kind, held: this.held });
  }
  /** Puts the pointer at `p` without the transition, as during a manual sweep. */
  async jump(p: Pt) {
    this.pos = p;
    await this.js(`__demo.cursor.move(${p.x}, ${p.y}, 0); "ok"`);
    await this.mouse("move", p);
  }
  async show(p: Pt) {
    this.pos = p;
    await this.mouse("move", p);
    await this.js(`__demo.cursor.show(${p.x}, ${p.y}); "ok"`);
    await sleep(260);
  }
  async hide() {
    await this.js(`__demo.cursor.hide(); "ok"`);
    await sleep(220);
  }
  async move(to: Pt, ms?: number) {
    const from = this.pos;
    const dist = Math.hypot(to.x - from.x, to.y - from.y);
    const dur = ms ?? Math.round(Math.min(900, Math.max(380, 320 + dist * 0.55)));
    await this.js(`__demo.cursor.move(${to.x}, ${to.y}, ${dur}); "ok"`);
    const start = Date.now();
    for (;;) {
      const t = Math.min(1, (Date.now() - start) / dur);
      const e = ease(t);
      await this.mouse("move", {
        x: Math.round(from.x + (to.x - from.x) * e),
        y: Math.round(from.y + (to.y - from.y) * e),
      });
      if (t >= 1) break;
      await sleep(12);
    }
    this.pos = to;
    await sleep(40);
  }
  async down() {
    await this.js(`__demo.cursor.press(true); "ok"`);
    this.held = true;
    await this.mouse("down", this.pos);
  }
  async up() {
    await this.mouse("up", this.pos);
    this.held = false;
    await this.js(`__demo.cursor.press(false); "ok"`);
  }
  async click(at?: Pt) {
    if (at) await this.move(at);
    await sleep(120);
    await this.down();
    await sleep(90);
    await this.up();
    await sleep(140);
  }
  /** Shows `label` beside the pointer while the key goes to the focused element. */
  async key(key: string, label = key, hold = 420) {
    await this.badge(label);
    await sleep(260);
    await this.app.send("key", { key });
    await sleep(hold);
    await this.badge("");
  }
  badge(label: string) {
    return this.js(`__demo.cursor.key(${JSON.stringify(label)}); "ok"`);
  }
  async type(selector: string, text: string, perChar = 60) {
    for (const ch of text) {
      await this.app.send("type", { selector, text: ch });
      await sleep(perChar);
    }
  }
}
