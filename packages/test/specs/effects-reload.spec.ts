// `sk dev` hands a running host the app's recompiled effects (`effects-changed.json` beside the HMR
// changes): an element whose CSS named an effect that did not exist yet picks it up.

import { afterAll, expect, test } from "bun:test";
import { copyFileSync, mkdirSync, mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { type Bounds, launch, readPng } from "../src/index";

const REPO = join(import.meta.dir, "../../..");
const FIXTURE = join(REPO, "packages/test/fixtures/layerfx");
const work = mkdtempSync(join(tmpdir(), "sk-effects-reload-"));

function compile(app: string, out: string) {
  const built = Bun.spawnSync(["bun", "scripts/shaders.ts", "--app", app, "--out", out], {
    cwd: REPO,
    stdout: "pipe",
    stderr: "pipe",
  });
  if (built.exitCode !== 0) throw new Error(`compiling effects failed:\n${built.stderr.toString()}`);
}

const early = join(work, "early");
mkdirSync(join(early, "shaders"), { recursive: true });
copyFileSync(join(FIXTURE, "shaders/count.slang"), join(early, "shaders/count.slang"));
compile(early, join(work, "first"));
compile(FIXTURE, join(work, "second"));

const app = await launch({
  app: "packages/test/fixtures/layerfx/src/main.ts",
  window: [360, 200],
  label: "effects-reload",
  env: { SK_EFFECTS_DIR: join(work, "first") },
});
afterAll(async () => {
  await app.close();
  rmSync(work, { recursive: true, force: true });
});

const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms));

type Shaded = { id: number; effect: string | null };
type Rgb = [number, number, number];

const near = (rgb: Rgb, expected: Rgb, slack = 24) => rgb.every((c, i) => Math.abs(c - expected[i]!) <= slack);

async function pixel(b: Bounds, dx: number, dy: number): Promise<Rgb> {
  const shot = readPng(await app.screenshot());
  const k = shot.width / 360;
  return shot.at(Math.floor((b.x + dx + 0.5) * k), Math.floor((b.y + dy + 0.5) * k));
}

async function effectOf(selector: string): Promise<string | null> {
  const id = (await app.query(selector)).nodes[0]!.id;
  const list = (await app.send("effects", { op: "list" })) as { nodes: Shaded[] };
  return list.nodes.find((n) => n.id === id)?.effect ?? null;
}

test("an effect compiled after the CSS that names it applies once it arrives", async () => {
  expect(await effectOf("#counter")).toBe("count");
  expect(await effectOf("#swapped")).toBe(null);
  await app.eval(`document.querySelector("#appear").setAttribute("style", "--shading: swap(1)")`);
  await sleep(200);
  expect(await effectOf("#appear")).toBe(null);

  writeFileSync(join(app.slotDir, "effects-changed.json"), JSON.stringify({ dir: join(work, "second") }));
  const deadline = Date.now() + 10_000;
  let effect = await effectOf("#swapped");
  while (effect !== "swap" && Date.now() < deadline) {
    await sleep(100);
    effect = await effectOf("#swapped");
  }
  expect(effect).toBe("swap");
  expect(await effectOf("#appear")).toBe("swap");
  expect(await effectOf("#counter")).toBe("count");

  const b = (await app.query("#swapped")).nodes[0]!.bounds!;
  let rgb = await pixel(b, 8, 8);
  while (!near(rgb, [0, 0, 255]) && Date.now() < deadline + 5_000) {
    await sleep(100);
    rgb = await pixel(b, 8, 8);
  }
  expect(rgb).toEqual([0, 0, 255]);
}, 30_000);
