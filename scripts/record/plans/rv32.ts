// The landing page's rv32 clip: the workbench boots its guest, a game is started and played, then
// the machine is paused, stepped an instruction and a frame at a time, and reset. The guest runs
// Freedoom here, so the clip shows free game data only.
import { copyFileSync, existsSync, mkdirSync, mkdtempSync, rmSync } from "node:fs";
import { basename, join, relative } from "node:path";
import { create } from "../../../packages/cli/cli/create";
import { compileAppEffects, writeEffectFiles } from "../../../packages/cli/effects/compile";
import { resolveCompiler } from "../../../packages/cli/effects/redslangc";
import { REPO, WORK_DIR } from "../../../packages/test/src/index";
import type { Driver } from "../driver";
import { DEMO_HANDLE } from "../handle";
import type { AppPlans } from "../record";
import { sleep } from "../win32";

const FREEDOOM = "https://github.com/freedoom/freedoom/releases/download/v0.13.0/freedoom-0.13.0.zip";

/** Freedoom: Phase 1, downloaded once. */
async function freedoom(): Promise<string> {
  const kept = join(WORK_DIR, "freedoom1.wad");
  if (existsSync(kept)) return kept;
  const dir = mkdtempSync(join(WORK_DIR, "freedoom-"));
  const zip = join(dir, "freedoom.zip");
  const response = await fetch(FREEDOOM);
  if (!response.ok) throw new Error(`${FREEDOOM}: HTTP ${response.status}`);
  await Bun.write(zip, response);
  // The system tar reads zip; the one Git ships does not.
  const tar = join(process.env.SystemRoot ?? "C:/Windows", "System32", "tar.exe");
  const unpacked = Bun.spawnSync([tar, "-xf", zip, "-C", dir]);
  const wad = join(dir, "freedoom-0.13.0", "freedoom1.wad");
  if (unpacked.exitCode !== 0 || !existsSync(wad)) throw new Error(`freedoom1.wad is not in ${FREEDOOM}`);
  copyFileSync(wad, kept);
  rmSync(dir, { recursive: true, force: true });
  return kept;
}

const control = (index: number) => `.bar .control:nth-child(${index})`;
const PAUSE = control(1);
const STEP = control(2);
const FRAME = control(3);
const RESET = control(4);

async function hold(d: Driver, code: string, ms: number) {
  const send = (type: string) =>
    d.js(`window.dispatchEvent(new KeyboardEvent(${JSON.stringify(type)}, { code: ${JSON.stringify(code)} })); "ok"`);
  await send("keydown");
  await sleep(ms);
  await send("keyup");
}

async function tap(d: Driver, code: string, after: number) {
  await hold(d, code, 70);
  await sleep(after);
}

const REST = { x: 680, y: 740 };
/** From a reset to the title picture standing still. */
const BOOT = 2600;

export const plans: AppPlans = {
  app: "rv32",
  group: "",
  window: [1280, 820],
  backdrop: "opaque",
  async prepare() {
    mkdirSync(WORK_DIR, { recursive: true });
    const wad = await freedoom();
    const dir = mkdtempSync(join(WORK_DIR, "record-rv32-"));
    create(dir, { name: "RV32 Workbench", template: "rv32" });
    copyFileSync(wad, join(dir, "assets", "freedoom1.wad"));
    const effects = join(dir, "effects");
    writeEffectFiles(effects, (await compileAppEffects(dir, (await resolveCompiler()).path)).files);
    return {
      app: relative(REPO, join(dir, "src/main.ts")).replaceAll("\\", "/"),
      env: { SK_APP_NAME: `sk-test-${basename(dir)}`, SK_EFFECTS_DIR: effects },
      dispose: () => rmSync(dir, { recursive: true, force: true }),
    };
  },
  clips: {
    rv32: {
      poster: 0.34,
      settle: 4000,
      async show(app) {
        await app.eval(`(() => {\n${DEMO_HANDLE}\n  return "ok";\n})()`);
      },
      async plan(d) {
        await d.show(REST);
        await d.click(await d.center(RESET));
        await d.move(REST);
        await sleep(BOOT);
        const start = Date.now();

        // The menu, a new game, the first episode, the default skill.
        for (let i = 0; i < 4; i++) await tap(d, "Enter", 520);
        await sleep(900);
        await hold(d, "ArrowUp", 1300);
        await hold(d, "ArrowLeft", 420);
        await tap(d, "ControlLeft", 260);
        await tap(d, "ControlLeft", 260);
        await hold(d, "ArrowUp", 1100);
        await hold(d, "ArrowRight", 620);
        await tap(d, "ControlLeft", 300);
        await hold(d, "ArrowUp", 900);

        await d.click(await d.center(PAUSE));
        await sleep(700);
        for (let i = 0; i < 4; i++) {
          await d.click(await d.center(STEP));
          await sleep(420);
        }
        for (let i = 0; i < 2; i++) {
          await d.click(await d.center(FRAME));
          await sleep(520);
        }
        await d.click(await d.center(PAUSE));
        await hold(d, "ArrowLeft", 500);
        await hold(d, "ArrowUp", 1200);
        await tap(d, "ControlLeft", 500);

        await d.click(await d.center(RESET));
        await d.move(REST);
        await sleep(BOOT);
        return { start, end: Date.now() };
      },
    },
  },
};
