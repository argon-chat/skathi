import { mkdirSync, rmSync, writeFileSync } from "node:fs";
import { join, relative } from "node:path";
import { afterAll, describe, expect as bunExpect, test } from "bun:test";
import { create } from "../../cli/cli/create";
import { compileAppEffects, writeEffectFiles } from "../../cli/effects/compile";
import { resolveCompiler } from "../../cli/effects/redslangc";
import { expect, launch, REPO, WORK_DIR } from "../src/index";

const T = 60_000;
const dir = join(WORK_DIR, `player-template-${process.pid}`);
rmSync(dir, { recursive: true, force: true });
mkdirSync(WORK_DIR, { recursive: true });
create(dir, { name: "player-template", template: "player", platforms: ["windows"] });
const entry = relative(REPO, join(dir, "src/main.ts")).split("\\").join("/");

const compiler = (await resolveCompiler().catch(() => null))?.path ?? null;
const effectsDir = join(dir, ".effects");
if (compiler) writeEffectFiles(effectsDir, (await compileAppEffects(dir, compiler)).files);

const app = await launch({
  app: entry,
  window: [1100, 760],
  label: "player-template",
  ...(compiler ? { env: { SK_EFFECTS_DIR: effectsDir } } : {}),
});
afterAll(async () => {
  await app.close();
  rmSync(dir, { recursive: true, force: true });
});

const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms));

async function until<T>(read: () => Promise<T>, ok: (value: T) => boolean, what: string, ms = 10_000): Promise<T> {
  const deadline = Date.now() + ms;
  let value = await read();
  while (!ok(value)) {
    if (Date.now() > deadline) throw new Error(`timed out waiting for ${what}; last: ${JSON.stringify(value)}`);
    await sleep(100);
    value = await read();
  }
  return value;
}

const shading = async (selector: string) =>
  String(await app.eval(`document.querySelector(${JSON.stringify(selector)}).style.getPropertyValue('--shading')`));

function tone(): Uint8Array {
  const rate = 22050;
  const samples = rate;
  const out = new DataView(new ArrayBuffer(44 + samples * 2));
  const text = (at: number, s: string) => [...s].forEach((c, i) => out.setUint8(at + i, c.charCodeAt(0)));
  text(0, "RIFF");
  out.setUint32(4, 36 + samples * 2, true);
  text(8, "WAVEfmt ");
  out.setUint32(16, 16, true);
  out.setUint16(20, 1, true);
  out.setUint16(22, 1, true);
  out.setUint32(24, rate, true);
  out.setUint32(28, rate * 2, true);
  out.setUint16(32, 2, true);
  out.setUint16(34, 16, true);
  text(36, "data");
  out.setUint32(40, samples * 2, true);
  for (let i = 0; i < samples; i++)
    out.setInt16(44 + i * 2, Math.round(Math.sin((i / rate) * 2 * Math.PI * 440) * 12000), true);
  return new Uint8Array(out.buffer);
}

describe("the player template", () => {
  test(
    "mounts with the demo playlist on the first track",
    async () => {
      // Likes and volume persist per app; start from none, as a fresh install does.
      await app.eval("localStorage.clear(); location.reload()");
      await expect(app.locator(".title")).toHaveText("Neon Petals!!");
      await expect(app.locator(".row")).toHaveCount(6);
      await expect(app.locator(".row.current .name")).toHaveText("Neon Petals!!");
      await expect(app.locator(".brand .name")).toHaveText("Player Template");
    },
    T,
  );

  test(
    "plays a demo track: decodes it, runs the clock and feeds the analyser's spectrum to the ring",
    async () => {
      await app.locator(".volume .tool").click();
      await until(
        () => app.eval("document.querySelector('.volume .tool').title"),
        (t) => t === "Unmute",
        "mute",
      );
      await app.locator(".play").click();
      await expect(app.locator(".play")).toHaveClass("playing", { timeout: 15_000 });
      await until(
        () => app.locator(".time").nth(0).textContent(),
        (t) => t !== "0:00",
        "the clock to run",
      );
      const levels = await until(
        () => shading(".disc"),
        (v) =>
          /^spectrum\(/.test(v) &&
          v
            .split(",")
            .slice(0, 24)
            .some((n) => Number.parseFloat(n.replace("spectrum(", "")) > 0),
        "non-zero spectrum levels",
      );
      bunExpect(levels).toMatch(/126px, 96px, [\d.]+, -?[\d.]+, #[0-9a-f]{8}\)$/);
    },
    T,
  );

  test(
    "a playlist row switches the track and dresses the app in its colours",
    async () => {
      await app.locator(".row").nth(5).click();
      await expect(app.locator(".title")).toHaveText("Demons...", { timeout: 5_000 });
      await expect(app.locator(".row.current .name")).toHaveText("Demons...");
      await until(
        () => shading(".app"),
        (v) => v.includes("#ff3b1f") && v.includes("#9e1020"),
        "the track's backdrop",
      );
    },
    T,
  );

  test(
    "liking a track sends hearts and is remembered",
    async () => {
      await app.locator(".like").click();
      await expect(app.locator(".like")).toHaveClass("liked");
      const born = Number((await shading(".hearts")).split(",")[2]?.replace(")", ""));
      bunExpect(born).toBeGreaterThan(0);
      bunExpect(String(await app.eval("localStorage.getItem('player.settings')"))).toContain('"demons"');
    },
    T,
  );

  test(
    "Add your music opens a picker and plays the file it returns",
    async () => {
      const file = join(dir, "a tone.wav");
      writeFileSync(file, tone());
      await app.locator(".add").click();
      const dialog = await app.waitForDialog();
      bunExpect(dialog.multiple).toBe(true);
      await app.answerDialog(dialog.id, file);
      await expect(app.locator(".row")).toHaveCount(7);
      await expect(app.locator(".title")).toHaveText("a tone", { timeout: 5_000 });
      await expect(app.locator(".row.current .length")).toHaveText("0:01", { timeout: 10_000 });
    },
    T,
  );

  test(
    "nothing threw",
    async () => {
      await app.expectNoErrors();
    },
    T,
  );
});
