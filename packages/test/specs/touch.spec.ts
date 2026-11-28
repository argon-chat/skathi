// Touch, as a platform reports a finger (devtools `touch` dispatches gpui's TouchEvent): each contact is a pointer of
// its own, captured where it went down; the first of a sequence is primary and alone gets the mouse events and click.

import { afterAll, describe, expect, test } from "bun:test";
import { launch, MACOS } from "../src/index";

const app = await launch({
  app: "packages/test/fixtures/touch/src/main.ts",
  window: [420, 320],
  label: "touch",
});
afterAll(async () => {
  await app.close();
});

const T = 30_000;
const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms));

async function touch(kind: string, id: number, x: number, y: number, extra: Record<string, unknown> = {}) {
  await app.send("touch", { x, y, kind, id, ...extra });
  // A second contact's moves are delivered once a frame.
  await sleep(80);
}

/** The log with pointer ids named by first appearance (A, B, ...): the host numbers contacts as it likes. */
async function log(): Promise<string[]> {
  const raw = JSON.parse(String(await app.eval("JSON.stringify(__SK_TEST_TOUCH__.log.splice(0))"))) as string[];
  const names = new Map<string, string>();
  return raw.map((line) =>
    line.replace(/(:|@\w+:)(\d+)(?=:|$)/g, (_, pre: string, id: string) => {
      if (!names.has(id)) names.set(id, String.fromCharCode(65 + names.size));
      return `${pre}${names.get(id)}`;
    }),
  );
}

describe("touch input", () => {
  test.skipIf(MACOS)(
    "two fingers: own ids, implicit capture, mouse events and click for the primary only",
    async () => {
      await log();
      await touch("start", 1, 60, 60, { pressure: 0.4, width: 20, height: 24 });
      await touch("start", 2, 300, 60);
      await touch("move", 2, 100, 80);
      await touch("end", 2, 100, 80);
      await touch("end", 1, 60, 60);
      expect(await log()).toEqual([
        "pointerover@left:A",
        "mouseover@left",
        "pointerdown@left:touch:A:primary:b1:p0.40:20x24",
        "mousedown@left",
        "got@left:A",
        "pointerover@right:B",
        "pointerdown@right:touch:B:other:b1:p0.50:1x1",
        "got@right:B",
        "pointermove@right:touch:B:other:b1:p0.50:1x1",
        "pointerup@right:touch:B:other:b0:p0.00:1x1",
        "lost@right:B",
        "pointerout@right:B",
        "pointerleave@right:B",
        "pointerup@left:touch:A:primary:b0:p0.00:20x24",
        "mouseup@left",
        "lost@left:A",
        "pointerout@left:A",
        "pointerleave@left:A",
        "mouseout@left",
        "mouseleave@left",
        "click@left",
      ]);
    },
    T,
  );

  test.skipIf(MACOS)(
    "a cancelled contact gets pointercancel and loses its capture",
    async () => {
      await log();
      await touch("start", 1, 300, 60);
      await touch("start", 2, 60, 60);
      await touch("cancel", 2, 60, 60);
      await touch("cancel", 1, 300, 60);
      const seen = (await log()).filter((l) => l.startsWith("pointercancel") || l.startsWith("lost"));
      expect(seen).toEqual(["pointercancel@left:B", "lost@left:B", "pointercancel@right:A", "lost@right:A"]);
    },
    T,
  );
});
