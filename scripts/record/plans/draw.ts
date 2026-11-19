// The landing page's draw clip: the draw template replaying an artist's recording
// (scripts/record/draw/girl.skev), with the editor's own controls following the recorded tools.
import { mkdirSync, mkdtempSync, readFileSync, rmSync } from "node:fs";
import { basename, join, relative } from "node:path";
import { create } from "../../../packages/cli/cli/create";
import { REPO, WORK_DIR, type SkApp } from "../../../packages/test/src/index";
import type { AppPlans } from "../record";
import { sleep } from "../win32";

const RECORDING = join(import.meta.dir, "..", "draw", "girl.skev");
const PLAY = JSON.stringify({ chrome: "editor", camera: "fit", fitTo: 28, compressPausesTo: 1, showUndone: false });
const HOLD = 2000;

/** Runs `body` (async JS) in the app and waits for it: `eval` does not await promises. */
async function run(app: SkApp, body: string, timeout: number): Promise<number> {
  await app.eval(
    `window.__clip = null; (async () => { ${body} })().then(() => (window.__clip = { ok: true }), (e) => (window.__clip = { error: String(e) })); 0`,
  );
  const start = Date.now();
  while (Date.now() - start < timeout) {
    await sleep(100);
    const raw = String(await app.eval("JSON.stringify(window.__clip)"));
    const result = JSON.parse(raw) as { ok: true } | { error: string } | null;
    if (result && "error" in result) throw new Error(result.error);
    if (result) return Date.now();
  }
  throw new Error(`the page did not finish in ${timeout} ms`);
}

export const plans: AppPlans = {
  app: "draw",
  group: "",
  window: [1280, 820],
  backdrop: "opaque",
  prepare() {
    mkdirSync(WORK_DIR, { recursive: true });
    const dir = mkdtempSync(join(WORK_DIR, "record-draw-"));
    create(dir, { name: "Draw", template: "draw" });
    return {
      app: relative(REPO, join(dir, "src/main.ts")).replaceAll("\\", "/"),
      env: { SK_APP_NAME: `sk-test-${basename(dir)}` },
      dispose: () => rmSync(dir, { recursive: true, force: true }),
    };
  },
  clips: {
    draw: {
      poster: 0.98,
      async show(app) {
        const bytes = readFileSync(RECORDING).toString("base64");
        await run(
          app,
          `const s = atob(${JSON.stringify(bytes)});
          const b = new Uint8Array(s.length);
          for (let i = 0; i < s.length; i++) b[i] = s.charCodeAt(i);
          await __draw.open(b);
          const played = __draw.play(${PLAY});
          __draw.pause();
          await played;
          await __draw.seek(0);`,
          30_000,
        );
      },
      async plan(d) {
        const start = Date.now();
        const done = await run(d.app, `await __draw.play(${PLAY});`, 120_000);
        await sleep(HOLD);
        return { start, end: done + HOLD };
      },
    },
  },
};
