// `sk create --template rv32`: the RV32 machine boots the guest image, the guest draws frames, a key
// reaches it, and the workbench stops and steps it.

import { copyFileSync, existsSync, mkdirSync, rmSync } from "node:fs";
import { join, relative } from "node:path";
import { afterAll, describe, expect as bunExpect, test } from "bun:test";
import { create } from "../../cli/cli/create";
import { compileAppEffects, writeEffectFiles } from "../../cli/effects/compile";
import { resolveCompiler } from "../../cli/effects/redslangc";
import { launch, REPO, type SkApp, WORK_DIR } from "../src/index";

const T = 120_000;
const dir = join(WORK_DIR, `rv32-template-${process.pid}`);
rmSync(dir, { recursive: true, force: true });
mkdirSync(WORK_DIR, { recursive: true });
create(dir, { name: "rv32-template", template: "rv32", platforms: ["windows"] });
// The game data is what an install fetches; one download serves every run.
const wad = join(dir, "assets", "doom1.wad");
const kept = join(WORK_DIR, "doom1.wad");
if (existsSync(kept)) copyFileSync(kept, wad);
Bun.spawnSync(["bun", "scripts/fetch-wad.mjs"], { cwd: dir, stdout: "inherit", stderr: "inherit" });
const fetched = existsSync(wad);
if (fetched) copyFileSync(wad, kept);
else console.warn("[rv32-template.spec] skipped: doom1.wad could not be fetched");
const entry = relative(REPO, join(dir, "src/main.ts")).split("\\").join("/");

// The glass over the screen is the template's own effect; without the shader compiler the canvas shows bare.
const compiler = (await resolveCompiler().catch(() => null))?.path ?? null;
const env: Record<string, string> = {};
if (compiler) {
  env.SK_EFFECTS_DIR = join(dir, "effects");
  writeEffectFiles(env.SK_EFFECTS_DIR, (await compileAppEffects(dir, compiler)).files);
}

const app: SkApp = await launch({ app: entry, window: [1200, 880], label: "rv32-template", env });
afterAll(async () => {
  await app.close();
  rmSync(dir, { recursive: true, force: true });
});

const counter = async (name: string): Promise<number> =>
  Number(((await app.locator(`.counter.${name}`).textContent()) ?? "").replace(/\D/g, ""));
const pc = () => app.locator(".counter.pc").textContent();

/** How many different colours the framebuffer holds: one while nothing has been drawn. */
const colours = async (): Promise<number> =>
  Number(
    await app.eval(`(() => {
      const c = document.querySelector(".screen");
      const d = new Uint32Array(c.getContext("2d").getImageData(0, 0, c.width, c.height).data.buffer);
      return new Set(d).size;
    })()`),
  );

async function until<V>(read: () => Promise<V>, done: (value: V) => boolean, ms = 8000): Promise<V> {
  let value = await read();
  for (const end = Date.now() + ms; !done(value) && Date.now() < end; value = await read()) await Bun.sleep(50);
  return value;
}

const shot = async (name: string) => {
  if (process.env.SK_SHOTS) await app.screenshot(join(process.env.SK_SHOTS, name));
};
const button = (index: number) => app.locator(".bar .control").nth(index);
const RUN = 0;
const STEP = 1;
const FRAME = 2;

describe.skipIf(!fetched)("the rv32 template", () => {
  test(
    "the machine boots the guest, and the guest draws a picture and keeps drawing",
    async () => {
      const first = await until(
        () => counter("frames"),
        (n) => n > 0,
        90_000,
      );
      bunExpect(first).toBeGreaterThan(0);
      bunExpect(await until(colours, (n) => n > 16)).toBeGreaterThan(16);
      const later = await until(
        () => counter("frames"),
        (n) => n > first + 5,
      );
      bunExpect(later).toBeGreaterThan(first + 5);
      await shot("rv32-running.png");
    },
    T,
  );

  test(
    "a key press reaches the guest",
    async () => {
      const before = await counter("keys");
      await app.eval(`window.dispatchEvent(new KeyboardEvent("keydown", { code: "Escape" })); 1`);
      await app.eval(`window.dispatchEvent(new KeyboardEvent("keyup", { code: "Escape" })); 1`);
      bunExpect(
        await until(
          () => counter("keys"),
          (n) => n >= before + 2,
        ),
      ).toBe(before + 2);
      await shot("rv32-menu.png");
    },
    T,
  );

  test(
    "Pause stops the machine, Step runs one instruction and Frame runs to the next frame",
    async () => {
      await button(RUN).click();
      const stopped = await until(
        () => counter("instructions"),
        () => false,
        400,
      );
      bunExpect(await counter("instructions")).toBe(stopped);

      const at = await pc();
      await button(STEP).click();
      bunExpect(await counter("instructions")).toBe(stopped + 1);
      bunExpect(await pc()).not.toBe(at);
      await shot("rv32-stepped.png");

      const frames = await counter("frames");
      await button(FRAME).click();
      bunExpect(await counter("frames")).toBe(frames + 1);
    },
    T,
  );

  test(
    "Reset boots the machine again",
    async () => {
      await button(RUN).click();
      const frames = await until(
        () => counter("frames"),
        (n) => n > 0,
      );
      await app.locator(".bar .control").nth(3).click();
      bunExpect(
        await until(
          () => counter("frames"),
          (n) => n < frames,
          2000,
        ),
      ).toBeLessThan(frames);
      bunExpect(
        await until(
          () => counter("frames"),
          (n) => n > 0,
          90_000,
        ),
      ).toBeGreaterThan(0);
    },
    T,
  );
});
