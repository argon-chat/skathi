// The landing page's sequencer clip: the sequencer template played by hand. It starts a pattern,
// paints three steps, turns two dials, switches to the second pattern and puts everything back.
import { mkdirSync, mkdtempSync, rmSync } from "node:fs";
import { basename, join, relative } from "node:path";
import { create } from "../../../packages/cli/cli/create";
import { compileAppEffects, writeEffectFiles } from "../../../packages/cli/effects/compile";
import { resolveCompiler } from "../../../packages/cli/effects/redslangc";
import { REPO, WORK_DIR } from "../../../packages/test/src/index";
import type { Driver } from "../driver";
import { DEMO_HANDLE } from "../handle";
import type { AppPlans } from "../record";
import { sleep } from "../win32";

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

${DEMO_HANDLE}
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
