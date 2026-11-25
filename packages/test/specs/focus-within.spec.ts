import { afterAll, describe, expect as bunExpect, test } from "bun:test";
import { launch, readPng, SCREENSHOTS } from "../src/index";

const app = await launch({
  app: "packages/test/fixtures/focus-within/src/main.ts",
  window: [360, 240],
  label: "focus-within",
  env: { SK_WINDOW_FOCUS: "never" },
});
afterAll(() => app.close());

const T = 30_000;
const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms));

const RINGS: [string, number][] = [
  ["l", 0.5],
  ["box", 1.5],
  ["solo", 1.5],
];

async function painted(): Promise<Record<string, boolean>> {
  const shot = readPng(await app.screenshot());
  const k = shot.width / 360;
  const out: Record<string, boolean> = {};
  for (const [id, inset] of RINGS) {
    const box = await app.locator(`#${id}`).boundingBox();
    const [r, , b] = shot.at(Math.floor((box.x - inset) * k), Math.floor((box.y + box.h / 2) * k));
    out[id] = b - r > 60;
  }
  return out;
}

async function matching(): Promise<Record<string, boolean>> {
  return JSON.parse(
    String(
      await app.eval(
        'JSON.stringify({ l: document.getElementById("l").matches(":focus-within"), ' +
          'box: document.getElementById("box").matches(":focus-within"), ' +
          'solo: document.getElementById("solo").matches(":focus") })',
      ),
    ),
  ) as Record<string, boolean>;
}

async function settle(want: Record<string, boolean>): Promise<Record<string, boolean>> {
  const deadline = Date.now() + 3_000;
  let got = await painted();
  while (JSON.stringify(got) !== JSON.stringify(want) && Date.now() < deadline) {
    await sleep(100);
    got = await painted();
  }
  return got;
}

describe(":focus-within on the element itself", () => {
  test.skipIf(!SCREENSHOTS)(
    "paints when focus enters its subtree, native input or focusable div, and clears when it leaves",
    async () => {
      await sleep(300);
      const none = { l: false, box: false, solo: false };
      bunExpect(await settle(none)).toEqual(none);

      await app.eval('document.getElementById("i").focus()');
      const label = { l: true, box: false, solo: false };
      bunExpect(await settle(label)).toEqual(label);
      bunExpect(await matching()).toEqual(label);

      await app.eval('document.getElementById("inner").focus()');
      const box = { l: false, box: true, solo: false };
      bunExpect(await settle(box)).toEqual(box);
      bunExpect(await matching()).toEqual(box);

      await app.eval('document.getElementById("solo").focus()');
      const solo = { l: false, box: false, solo: true };
      bunExpect(await settle(solo)).toEqual(solo);
      bunExpect(await matching()).toEqual(solo);

      await app.eval('document.getElementById("ta").focus()');
      bunExpect(await settle(box)).toEqual(box);
      bunExpect(await matching()).toEqual(box);

      await app.eval('document.getElementById("i").focus()');
      bunExpect(await settle(label)).toEqual(label);
      await app.eval("document.activeElement.blur()");
      bunExpect(await settle(none)).toEqual(none);
      bunExpect(await matching()).toEqual(none);
    },
    T,
  );
});
