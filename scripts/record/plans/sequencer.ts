// The landing page's sequencer clip: the sequencer template played by hand. It starts a pattern,
// paints three steps, turns two dials, switches to the second pattern and puts everything back.
import { mkdirSync, mkdtempSync, rmSync } from "node:fs";
import { basename, join, relative } from "node:path";
import { CURSOR_FILL, CURSOR_HOTSPOT, CURSOR_OUTLINE, CURSOR_VIEWBOX } from "../../../apps/elements/src/cursorShape";
import { create } from "../../../packages/cli/cli/create";
import { compileAppEffects, writeEffectFiles } from "../../../packages/cli/effects/compile";
import { resolveCompiler } from "../../../packages/cli/effects/redslangc";
import { REPO, WORK_DIR } from "../../../packages/test/src/index";
import type { Driver } from "../driver";
import type { AppPlans } from "../record";
import { sleep } from "../win32";

const SIZE = 27;
const HX = (CURSOR_HOTSPOT.x / CURSOR_VIEWBOX) * SIZE;
const HY = (CURSOR_HOTSPOT.y / CURSOR_VIEWBOX) * SIZE;

// The template has no recorder's handle of its own. This gives it `__demo` with a drawn cursor,
// and sends what would reach the speakers through a silent gain: the analyser before it still
// hears the mix, so the meter moves.
const HANDLE = `(() => {
  const contexts = [];
  globalThis.AudioContext = class extends AudioContext {
    constructor(...options) {
      super(...options);
      contexts.push(this);
    }
  };
  const connect = AudioNode.prototype.connect;
  AudioNode.prototype.connect = function (to, ...rest) {
    const context = this.__silent ? undefined : contexts.find((c) => c.destination === to);
    if (!context) return connect.call(this, to, ...rest);
    const silent = context.createGain();
    silent.gain.value = 0;
    silent.__silent = true;
    connect.call(silent, to);
    connect.call(this, silent);
    return to;
  };

  const cursor = document.createElement("div");
  cursor.style.cssText =
    "position:fixed;left:0;top:0;width:${SIZE}px;height:${SIZE}px;z-index:1000;pointer-events:none;opacity:0;" +
    "transition-property:transform,opacity;transition-timing-function:cubic-bezier(0.32,0,0.18,1),ease";
  const NS = "http://www.w3.org/2000/svg";
  const arrow = document.createElementNS(NS, "svg");
  arrow.setAttribute("width", "${SIZE}");
  arrow.setAttribute("height", "${SIZE}");
  arrow.setAttribute("viewBox", "0 0 ${CURSOR_VIEWBOX} ${CURSOR_VIEWBOX}");
  arrow.style.cssText = "transition:transform 110ms ease;transform-origin:4.7px 4px";
  for (const [d, fill] of [["${CURSOR_FILL}", "#f7eafc"], ["${CURSOR_OUTLINE}", "#8a44a3"]]) {
    const path = document.createElementNS(NS, "path");
    path.setAttribute("d", d);
    path.setAttribute("fill", fill);
    path.setAttribute("fill-rule", "evenodd");
    arrow.append(path);
  }
  cursor.append(arrow);
  document.body.append(cursor);
  const at = (x, y, ms) => {
    cursor.style.transitionDuration = ms + "ms, 200ms";
    cursor.style.transform = "translate(" + (x - ${HX}) + "px, " + (y - ${HY}) + "px)";
  };
  globalThis.__demo = {
    rect(selector) {
      const el = document.querySelector(selector);
      if (!el) return null;
      const r = el.getBoundingClientRect();
      return JSON.stringify({ x: r.x, y: r.y, w: r.width, h: r.height });
    },
    cursor: {
      show(x, y) {
        at(x, y, 0);
        setTimeout(() => (cursor.style.opacity = "1"), 32);
      },
      hide: () => (cursor.style.opacity = "0"),
      move: at,
      press: (down) => (arrow.style.transform = down ? "scale(0.86)" : "scale(1)"),
      key() {},
    },
  };
  return "ok";
})()`;

const step = (row: number, index: number) => `.row:nth-child(${row}) .step:nth-child(${index})`;

/** Drags from the middle of one element to the middle of another, or by `dy` from the first. */
async function drag(d: Driver, from: string, to: string | number, ms: number) {
  await d.move(await d.center(from));
  await sleep(140);
  await d.down();
  await sleep(120);
  await d.move(typeof to === "number" ? { x: d.pos.x, y: d.pos.y + to } : await d.center(to), ms);
  await sleep(160);
  await d.up();
  await sleep(200);
}

export const plans: AppPlans = {
  app: "sequencer",
  group: "",
  window: [1280, 820],
  backdrop: "opaque",
  async prepare() {
    mkdirSync(WORK_DIR, { recursive: true });
    const dir = mkdtempSync(join(WORK_DIR, "record-sequencer-"));
    create(dir, { name: "Sequencer", template: "sequencer" });
    const effects = join(dir, "effects");
    writeEffectFiles(effects, (await compileAppEffects(dir, (await resolveCompiler()).path)).files);
    return {
      app: relative(REPO, join(dir, "src/main.ts")).replaceAll("\\", "/"),
      env: { SK_APP_NAME: `sk-test-${basename(dir)}`, SK_EFFECTS_DIR: effects },
      dispose: () => rmSync(dir, { recursive: true, force: true }),
    };
  },
  clips: {
    sequencer: {
      poster: 0.62,
      async show(app) {
        await app.eval(HANDLE);
      },
      async plan(d) {
        const start = Date.now();
        await sleep(500);
        await d.show({ x: 420, y: 560 });
        await d.click(await d.center(".play"));
        await sleep(2400);

        await drag(d, step(2, 9), step(2, 11), 700);
        await sleep(1500);
        await drag(d, '[aria-label="Echo"]', -70, 800);
        await drag(d, '[aria-label="Drive"]', -70, 800);
        await sleep(1800);

        await d.click(await d.center('[aria-label="Pattern B"]'));
        await sleep(5200);
        await d.click(await d.center('[aria-label="Pattern A"]'));
        await sleep(1200);

        await drag(d, '[aria-label="Drive"]', 70, 700);
        await drag(d, '[aria-label="Echo"]', 70, 700);
        await drag(d, step(2, 9), step(2, 11), 600);
        await sleep(900);
        await d.click(await d.center(".play"));
        await d.move({ x: 420, y: 560 });
        await d.hide();
        await sleep(1400);
        return { start, end: Date.now() };
      },
    },
  },
};
