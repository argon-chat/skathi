import { afterAll, describe, expect, test } from "bun:test";
import { launch, readPng } from "../src/index";

const WIDTH = 420;
const app = await launch({
  app: "packages/vn-test/fixtures/font-subsets/src/main.ts",
  window: [WIDTH, 440],
  label: "font-subsets",
});
afterAll(() => app.close());

const T = 30_000;

const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms));

async function until<V>(read: () => Promise<V>, ok: (v: V) => boolean, ms = 10_000): Promise<V> {
  const deadline = Date.now() + ms;
  let v = await read();
  while (!ok(v) && Date.now() < deadline) {
    await sleep(50);
    v = await read();
  }
  return v;
}

type Box = { x: number; y: number; w: number; h: number };
const box = async (id: string): Promise<Box> => (await app.query(`#${id}`)).nodes[0]!.bounds!;
const width = async (id: string) => (await app.query(`#${id}`)).nodes[0]?.bounds?.w ?? -1;
const faces = async (): Promise<string[]> =>
  JSON.parse(String(await app.eval("JSON.stringify(__VN_TEST_SUBSETS__.faces())")));

async function settled() {
  await until(faces, (f) => f.filter((x) => x.includes(":loaded:")).length >= 6);
  await until(
    async (): Promise<[number, number, number]> => [await width("lat"), await width("lat-ref"), await width("plain")],
    ([a, b, p]) => a > 0 && Math.abs(a - b) < 0.01 && Math.abs(a - p) > 1,
  );
  await sleep(300);
}

async function compare(a: string, b: string, w?: number) {
  const shot = readPng(await app.screenshot());
  const k = shot.width / WIDTH;
  const [ba, bb] = [await box(a), await box(b)];
  const cols = Math.floor((w ?? ba.w) * k);
  const rows = Math.floor(Math.min(ba.h, bb.h) * k);
  const [ax, ay, bx, by] = [ba.x, ba.y, bb.x, bb.y].map((v) => Math.round(v * k));
  let ink = 0;
  let diff = 0;
  for (let y = 0; y < rows; y++) {
    for (let x = 0; x < cols; x++) {
      const p = shot.at(ax! + x, ay! + y);
      const q = shot.at(bx! + x, by! + y);
      if ((p[0] + p[1] + p[2]) / 3 < 160) ink++;
      if (Math.abs(p[0] - q[0]) + Math.abs(p[1] - q[1]) + Math.abs(p[2] - q[2]) > 60) diff++;
    }
  }
  return { ink, diff };
}

async function expectSamePixels(a: string, b: string, w?: number) {
  const { ink, diff } = await compare(a, b, w);
  expect(ink).toBeGreaterThan(40);
  expect(diff).toBeLessThanOrEqual(Math.max(4, Math.round(ink * 0.02)));
}

describe("a family cut into unicode-range subsets", () => {
  test(
    "only the subsets the text needs load",
    async () => {
      await settled();
      const f = await faces();
      expect(f).toContain("Mix:loaded:U+301");
      expect(f).toContain("Mix:loaded:U+0-FF");
      expect(f).toContain("Mix:unloaded:U+370-377");
    },
    T,
  );

  test(
    "Latin text is drawn with the latin subset's glyphs",
    async () => {
      await settled();
      expect(await width("lat")).toBeCloseTo(await width("lat-ref"), 1);
      await expectSamePixels("lat", "lat-ref");
    },
    T,
  );

  test(
    "bold Latin and digits are the latin subset's 700",
    async () => {
      await settled();
      expect(await width("bold")).toBeCloseTo(await width("bold-ref"), 1);
      await expectSamePixels("bold", "bold-ref");
    },
    T,
  );

  test(
    "Cyrillic text is drawn with the cyrillic subset's glyphs",
    async () => {
      await settled();
      expect(await width("cyr")).toBeCloseTo(await width("cyr-ref"), 1);
      await expectSamePixels("cyr", "cyr-ref");
    },
    T,
  );

  test(
    "one run of Latin, Cyrillic and digits takes each character from its subset",
    async () => {
      await settled();
      const pieces = (await width("mixed-lat")) + (await width("mixed-cyr")) + (await width("mixed-num"));
      expect(Math.abs((await width("mixed")) - pieces)).toBeLessThanOrEqual(3);
      await expectSamePixels("mixed", "mixed-lat", (await width("mixed-lat")) - 6);
    },
    T,
  );

  test(
    "a PUA-only family that leads the stack draws its own glyphs, and the next family the rest",
    async () => {
      await settled();
      const pieces = (await width("stack-lat")) + (await width("stack-pua"));
      expect(Math.abs((await width("stack")) - pieces)).toBeLessThanOrEqual(2);
      await expectSamePixels("stack", "stack-lat", (await width("stack-lat")) - 6);
    },
    T,
  );

  test(
    "the app logged nothing broken",
    async () => {
      const { entries } = await app.logs(0);
      expect(entries.filter((e) => e.text.includes("missing-font"))).toEqual([]);
      // The greek subset's file is missing on purpose, and nothing asks for it.
      await app.expectNoErrors();
    },
    T,
  );
});
