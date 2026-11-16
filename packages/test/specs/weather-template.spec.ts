// `sk create --template weather`: its shaders compile into the app and paint the scene, the panels and
// the map, and a preview turns every layer to another weather. Offline it falls back to its sample
// forecast, so nothing here depends on the network.

import { mkdirSync, rmSync } from "node:fs";
import { join, relative } from "node:path";
import { afterAll, describe, expect as bunExpect, test } from "bun:test";
import { create } from "../../cli/cli/create";
import { compileAppEffects, writeEffectFiles } from "../../cli/effects/compile";
import { resolveCompiler } from "../../cli/effects/redslangc";
import { expect, launch, REPO, type SkApp, WORK_DIR } from "../src/index";

const T = 60_000;
// After the four painted skies, whose sizes take two slots each, and daylight, sun, cloud and fog.
const STORM = 12;
const dir = join(WORK_DIR, `weather-template-${process.pid}`);
rmSync(dir, { recursive: true, force: true });
mkdirSync(WORK_DIR, { recursive: true });
create(dir, { name: "weather-template", template: "weather", platforms: ["windows"] });
const entry = relative(REPO, join(dir, "src/main.ts")).split("\\").join("/");
const compiler = (await resolveCompiler().catch(() => null))?.path ?? null;

let app: SkApp | null = null;
if (compiler) {
  const effectsDir = join(dir, "effects");
  writeEffectFiles(effectsDir, (await compileAppEffects(dir, compiler)).files);
  app = await launch({
    app: entry,
    window: [1180, 820],
    label: "weather-template",
    env: { SK_EFFECTS_DIR: effectsDir },
  });
}
afterAll(async () => {
  await app?.close();
  rmSync(dir, { recursive: true, force: true });
});

type Shaded = { classes: string[]; effect: string | null; params: number[] };
const shaded = async (): Promise<Shaded[]> =>
  ((await app!.send("effects", { op: "list" })) as { nodes: Shaded[] }).nodes;
const paramsOf = async (cls: string): Promise<number[]> =>
  (await shaded()).find((n) => n.classes.includes(cls))?.params ?? [];

describe.skipIf(!compiler)("the weather template", () => {
  test(
    "the sky, the landscape, the rain, every panel and the map are the template's own effects",
    async () => {
      await expect(app!.locator(".temp-row .temp")).toHaveText(/°$/, { timeout: 30_000 });
      const nodes = await shaded();
      const effectOf = (cls: string) => nodes.filter((n) => n.classes.includes(cls)).map((n) => n.effect);
      bunExpect(effectOf("sky")).toEqual(["sky"]);
      bunExpect(effectOf("land")).toEqual(["land"]);
      bunExpect(effectOf("precip")).toEqual(["precip"]);
      bunExpect(effectOf("base")).toEqual(["nightmap"]);
      bunExpect(effectOf("rain")).toEqual(["radar"]);
      const panes = effectOf("glass");
      bunExpect(panes.length).toBeGreaterThanOrEqual(4);
      bunExpect(new Set(panes)).toEqual(new Set(["pane"]));
    },
    T,
  );

  test(
    "a previewed thunderstorm reaches the sky, the rain and the glass",
    async () => {
      await app!.locator(".round.picker").click();
      await app!.locator(".picker .chip.storm").click();
      await expect(app!.locator(".condition")).toHaveText("Thunderstorm");
      const deadline = Date.now() + 10_000;
      let sky: number[] = [];
      let pane: number[] = [];
      let precip: number[] = [];
      while (Date.now() < deadline) {
        [sky, pane, precip] = await Promise.all([paramsOf("sky"), paramsOf("details"), paramsOf("precip")]);
        if (sky[STORM] === 1 && pane[3] === 1 && precip[3] === 1 && (precip[0] ?? 0) > 0) break;
        await new Promise((r) => setTimeout(r, 200));
      }
      bunExpect(sky[STORM]).toBe(1);
      bunExpect(pane[3]).toBe(1);
      bunExpect(precip[3]).toBe(1);
      bunExpect(precip[0]).toBeGreaterThan(0);
    },
    T,
  );

  test(
    "nothing threw",
    async () => {
      await app!.expectNoErrors();
    },
    T,
  );
});
