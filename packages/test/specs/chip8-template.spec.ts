// `sk create --template chip8`: the bundled programs assemble and run, the debugger follows the
// CPU, and a breakpoint stops it.

import { mkdirSync, rmSync } from "node:fs";
import { join, relative } from "node:path";
import { afterAll, describe, expect as bunExpect, test } from "bun:test";
import { create } from "../../cli/cli/create";
import { expect, launch, REPO, type SkApp, WORK_DIR } from "../src/index";

const T = 60_000;
const dir = join(WORK_DIR, `chip8-template-${process.pid}`);
rmSync(dir, { recursive: true, force: true });
mkdirSync(WORK_DIR, { recursive: true });
create(dir, { name: "chip8-template", template: "chip8", platforms: ["windows"] });
const entry = relative(REPO, join(dir, "src/main.ts")).split("\\").join("/");

const app: SkApp = await launch({ app: entry, window: [1120, 800], label: "chip8-template" });
afterAll(async () => {
  await app.close();
  rmSync(dir, { recursive: true, force: true });
});

const lit = async (): Promise<number> =>
  Number(
    await app.eval(`(() => {
      const c = document.querySelector(".screen");
      const d = c.getContext("2d").getImageData(0, 0, c.width, c.height).data;
      let n = 0;
      for (let i = 1; i < d.length; i += 4) if (d[i] > 200) n++;
      return Math.round(n / 100);
    })()`),
  );

async function until<V>(read: () => Promise<V>, done: (value: V) => boolean): Promise<V> {
  let value = await read();
  for (const end = Date.now() + 8000; !done(value) && Date.now() < end; value = await read()) await Bun.sleep(50);
  return value;
}

const button = (index: number) => app.locator("button.control").nth(index);
const RUN = 2;
const STEP = 3;

describe("the chip8 template", () => {
  test(
    "Bounce runs: its ball is the four lit pixels, and it moves",
    async () => {
      await expect(app.locator(".status")).toHaveText("Running", { timeout: 30_000 });
      const pc = app.locator(".registers.special .register").first();
      const before = await pc.textContent();
      bunExpect(await until(lit, (n) => n === 4)).toBe(4);
      bunExpect(
        await until(
          () => pc.textContent(),
          (text) => text !== before,
        ),
      ).not.toBe(before);
      if (process.env.SK_SHOTS) await app.screenshot(join(process.env.SK_SHOTS, "chip8-bounce.png"));
    },
    T,
  );

  test(
    "Catch draws its score and paddle, and a key moves the paddle",
    async () => {
      await app.locator(".program").nth(1).click();
      // Two digits of 0, the paddle and the drop.
      bunExpect(await until(lit, (n) => n === 40)).toBe(14 + 14 + 8 + 4);
      const paddle = async (): Promise<number> =>
        await app.eval(`(() => {
          const c = document.querySelector(".screen");
          const d = c.getContext("2d").getImageData(0, 305, c.width, 1).data;
          for (let x = 0; x < c.width; x++) if (d[x * 4 + 1] > 200) return x / 10;
          return -1;
        })()`);
      bunExpect(await paddle()).toBe(28);
      await app.eval(`window.dispatchEvent(new KeyboardEvent("keydown", { code: "KeyQ" })); 1`);
      bunExpect(await until(paddle, (x) => x < 28)).toBeLessThan(28);
      await app.eval(`window.dispatchEvent(new KeyboardEvent("keyup", { code: "KeyQ" })); 1`);
      if (process.env.SK_SHOTS) await app.screenshot(join(process.env.SK_SHOTS, "chip8-catch.png"));
    },
    T,
  );

  test(
    "a breakpoint stops the program on that line, and Step moves one instruction",
    async () => {
      await button(RUN).click();
      await expect(app.locator(".status")).toHaveText("Paused");
      const lines = app.locator(".listing li");
      const target = await app.locator(".listing li:nth-child(7) .address").textContent();
      await lines.nth(6).click();
      await button(RUN).click();
      await expect(app.locator(".status")).toHaveText(`Stopped at a breakpoint, ${target}`, { timeout: 10_000 });
      await expect(app.locator(".listing li.current .address")).toHaveText(target);
      await button(STEP).click();
      const current = () => app.locator(".listing li.current .address").textContent();
      bunExpect(await until(current, (text) => text !== target)).not.toBe(target);
      if (process.env.SK_SHOTS) await app.screenshot(join(process.env.SK_SHOTS, "chip8-breakpoint.png"));
    },
    T,
  );
});
