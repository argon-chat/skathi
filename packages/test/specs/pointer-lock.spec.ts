// Pointer lock through the real host, with `SK_POINTER_LOCK=fake`: gpui pins mouse events and measures their motion
// exactly as for a system lock, but the cursor is never hidden or held (and the window counts as focused, which a
// harness window never is). Gestures are real injected presses; Escape a real injected keystroke.

import { afterAll, describe, expect, test } from "bun:test";
import { launch } from "../src/index";

const app = await launch({
  app: "packages/test/fixtures/pointer-lock/src/main.ts",
  window: [420, 300],
  label: "pointer-lock",
  env: { SK_POINTER_LOCK: "fake" },
});
afterAll(async () => {
  await app.close();
});

const T = 30_000;
const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms));

async function logOf(run: () => Promise<unknown>, moves = true): Promise<string[]> {
  await app.eval("__SK_TEST_LOCK__.log.length = 0");
  await run();
  await sleep(250);
  const log = JSON.parse(String(await app.eval("JSON.stringify(__SK_TEST_LOCK__.log)"))) as string[];
  return moves ? log : log.filter((l) => !l.startsWith("move:"));
}
const locked = async () => (await app.eval("__SK_TEST_LOCK__.locked()")) as string | null;
const move = (x: number, y: number) => app.send("mouse", { x, y, kind: "move" });
const clickPad = () => app.locator("#pad").click({ real: true });
const lockByClick = () => logOf(clickPad, false);

describe("pointer lock", () => {
  test(
    "a request without a user gesture fails",
    async () => {
      const log = await logOf(() => app.eval("__SK_TEST_LOCK__.request()"));
      expect(log).toEqual(["error", "rejected:NotAllowedError"]);
      expect(await locked()).toBeNull();
    },
    T,
  );

  test(
    "a click locks; moves anywhere reach the element with their movement while clientX holds still",
    async () => {
      const log = await lockByClick();
      expect(log).toEqual(["change:pad", "resolved"]);
      expect(await locked()).toBe("pad");

      const pad = (await app.query("#pad")).nodes[0]!.bounds!;
      const at = { x: Math.round(pad.x + pad.w / 2), y: Math.round(pad.y + pad.h / 2) };
      const moves = await logOf(async () => {
        await move(5, 5);
        await sleep(100);
        await move(35, 5);
        await sleep(100);
        await move(10, 290);
      });
      const reported = moves.filter((l) => l.startsWith("move:"));
      expect(reported.at(-2)).toBe(`move:${at.x},${at.y} by 30,0`);
      expect(reported.at(-1)).toBe(`move:${at.x},${at.y} by -25,285`);
    },
    T,
  );

  test(
    "Escape releases before the page hears it, and only a new gesture locks again",
    async () => {
      const log = await logOf(() => app.press("Escape"));
      expect(log).toEqual(["change:null"]);
      expect(await locked()).toBeNull();

      const again = await logOf(() => app.eval("__SK_TEST_LOCK__.request()"));
      expect(again).toEqual(["error", "rejected:NotAllowedError"]);

      expect(await lockByClick()).toEqual(["change:pad", "resolved"]);
    },
    T,
  );

  test(
    "exitPointerLock releases; a second exit is silent",
    async () => {
      expect(await locked()).toBe("pad");
      expect(await logOf(() => app.eval("__SK_TEST_LOCK__.exit()"))).toEqual(["change:null"]);
      expect(await logOf(() => app.eval("__SK_TEST_LOCK__.exit()"))).toEqual([]);
    },
    T,
  );

  test(
    "removing the locked element releases",
    async () => {
      expect(await lockByClick()).toEqual(["change:pad", "resolved"]);
      expect(await logOf(() => app.eval("__SK_TEST_LOCK__.remove()"))).toEqual(["change:null"]);
      expect(await locked()).toBeNull();
      await app.eval("__SK_TEST_LOCK__.restore()");
    },
    T,
  );

  test(
    "unlocked, a move's movement is how far the pointer went",
    async () => {
      const pad = (await app.query("#pad")).nodes[0]!.bounds!;
      const x = Math.round(pad.x + 20);
      const y = Math.round(pad.y + 20);
      const log = await logOf(async () => {
        await move(x, y);
        await sleep(100);
        await move(x + 7, y + 3);
      });
      expect(log.at(-1)).toBe(`move:${x + 7},${y + 3} by 7,3`);
    },
    T,
  );
});
