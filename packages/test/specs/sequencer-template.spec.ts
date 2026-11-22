// `sk create --template sequencer`: its dials, keys, panel and meter are the template's own effects, a press toggles a step,
// and playing moves the playhead through the pattern.

import { mkdirSync, rmSync } from "node:fs";
import { join, relative } from "node:path";
import { afterAll, describe, expect as bunExpect, test } from "bun:test";
import { create } from "../../cli/cli/create";
import { compileAppEffects, writeEffectFiles } from "../../cli/effects/compile";
import { resolveCompiler } from "../../cli/effects/redslangc";
import { expect, launch, REPO, type SkApp, WORK_DIR } from "../src/index";

const T = 60_000;
const dir = join(WORK_DIR, `sequencer-template-${process.pid}`);
rmSync(dir, { recursive: true, force: true });
mkdirSync(WORK_DIR, { recursive: true });
create(dir, { name: "sequencer-template", template: "sequencer", platforms: ["windows"] });
const entry = relative(REPO, join(dir, "src/main.ts")).split("\\").join("/");
const compiler = (await resolveCompiler().catch(() => null))?.path ?? null;

let app: SkApp | null = null;
if (compiler) {
  const effectsDir = join(dir, "effects");
  writeEffectFiles(effectsDir, (await compileAppEffects(dir, compiler)).files);
  app = await launch({
    app: entry,
    window: [1200, 820],
    label: "sequencer-template",
    env: { SK_EFFECTS_DIR: effectsDir },
  });
}
afterAll(async () => {
  await app?.close();
  rmSync(dir, { recursive: true, force: true });
});

const shot = async (name: string) => {
  if (process.env.SK_SHOTS) await app!.screenshot(join(process.env.SK_SHOTS, name));
};

describe.skipIf(!compiler)("the sequencer template", () => {
  test(
    "the meter is painted by the template's effect, and the starting pattern is on the grid",
    async () => {
      await expect(app!.locator(".row")).toHaveCount(6, { timeout: 30_000 });
      const nodes = ((await app!.send("effects", { op: "list" })) as { nodes: { classes: string[]; effect: string }[] })
        .nodes;
      bunExpect([...new Set(nodes.map((n) => n.effect))].sort()).toEqual(["knob", "meter", "pad", "plate"]);
      await expect(app!.locator(".row:nth-child(1) .step.on")).toHaveCount(5);
      await shot("sequencer-idle.png");
    },
    T,
  );

  test(
    "a press turns a step on and another press turns it off",
    async () => {
      const second = app!.locator(".row:nth-child(2) .step").nth(1);
      await second.click();
      await expect(app!.locator(".row:nth-child(2) .step.on")).toHaveCount(3);
      await second.click();
      await expect(app!.locator(".row:nth-child(2) .step.on")).toHaveCount(2);
    },
    T,
  );

  test(
    "Play moves the playhead, and Stop takes it away",
    async () => {
      await app!.locator(".play").click();
      const seen = new Set<number>();
      for (const end = Date.now() + 6000; seen.size < 4 && Date.now() < end; await Bun.sleep(40)) {
        const at = Number(
          await app!.eval(
            `[...document.querySelectorAll(".row:nth-child(1) .step")].findIndex(s => s.classList.contains("now"))`,
          ),
        );
        if (at >= 0) seen.add(at);
      }
      bunExpect(seen.size).toBeGreaterThanOrEqual(4);
      await shot("sequencer-playing.png");
      await app!.locator(".play").click();
      await expect(app!.locator(".step.now")).toHaveCount(0);
    },
    T,
  );
});
